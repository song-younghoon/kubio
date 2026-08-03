package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

type requestIDHijacker struct {
	header http.Header
	peer   net.Conn
}

func (w *requestIDHijacker) Header() http.Header { return w.header }

func (w *requestIDHijacker) WriteHeader(int) {}

func (w *requestIDHijacker) Write(data []byte) (int, error) { return len(data), nil }

func (w *requestIDHijacker) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	connection, peer := net.Pipe()
	w.peer = peer
	return connection, bufio.NewReadWriter(bufio.NewReader(peer), bufio.NewWriter(peer)), nil
}

func TestRequestIDForAcceptsOnlyOneValidValue(t *testing.T) {
	tests := []struct {
		name   string
		header http.Header
		want   string
	}{
		{name: "canonical", header: http.Header{requestIDHeader: {"client-123"}}, want: "client-123"},
		{name: "case insensitive", header: http.Header{"x-request-id": {"client-456"}}, want: "client-456"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
			req.Header = test.header
			got, err := requestIDFor(req)
			if err != nil || got != test.want {
				t.Fatalf("requestIDFor() = %q, %v; want %q", got, err, test.want)
			}
		})
	}

	for _, header := range []http.Header{
		{requestIDHeader: {"one", "two"}},
		{requestIDHeader: {"bad value"}},
		{requestIDHeader: {strings.Repeat("a", 129)}},
		{requestIDHeader: {"one"}, "x-request-id": {"two"}},
	} {
		req := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
		req.Header = header
		got, err := requestIDFor(req)
		if err != nil || len(got) != 32 {
			t.Fatalf("requestIDFor(%v) = %q, %v; want a generated 32-byte hex ID", header, got, err)
		}
		if _, err := hex.DecodeString(got); err != nil {
			t.Fatalf("generated ID %q is not lowercase hex: %v", got, err)
		}
	}
}

func TestRequestIDWriterKeepsOneIDForUpgradeHeaders(t *testing.T) {
	underlying := &requestIDHijacker{header: make(http.Header)}
	writer := newRequestIDWriter(underlying, "upgrade-id")
	connection, _, err := writer.Hijack()
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	defer underlying.peer.Close()

	header := writer.Header()
	header.Add(requestIDHeader, "upstream")
	final := writer.Header()
	if values := final.Values(requestIDHeader); len(values) != 1 || values[0] != "upgrade-id" {
		t.Fatalf("upgrade response IDs = %v; want [upgrade-id]", values)
	}
}

func TestRequestIDIsPropagatedAndOwnsResponseHeader(t *testing.T) {
	var upstreamID string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstreamID = r.Header.Get(requestIDHeader)
		w.Header().Set(requestIDHeader, "upstream")
		w.Header().Add("Trailer", requestIDHeader)
		w.WriteHeader(http.StatusCreated)
		_, _ = w.Write([]byte("ok"))
		w.Header().Set(requestIDHeader, "trailer")
	}))
	defer upstream.Close()

	router, err := newRouter(config{
		RequestID: true,
		Sites: []siteConfig{{
			Hosts:  []string{"example.com"},
			Target: upstream.URL,
			ResponseHeaders: responseHeaderPolicy{
				Set:    map[string][]string{requestIDHeader: {"configured"}},
				Remove: []string{requestIDHeader},
			},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	req := httptest.NewRequest(http.MethodGet, "http://example.com/path", nil)
	req.Header["x-request-id"] = []string{"client-id"}
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, req)

	if upstreamID != "client-id" {
		t.Fatalf("upstream X-Request-ID = %q; want client-id", upstreamID)
	}
	if got := recorder.Result().Header.Get(requestIDHeader); got != "client-id" {
		t.Fatalf("response X-Request-ID = %q; want client-id", got)
	}
	result := recorder.Result()
	if got := result.Trailer.Get(requestIDHeader); got != "" {
		t.Fatalf("response trailer X-Request-ID = %q; want it removed", got)
	}
}

func TestRequestIDIsGeneratedForInvalidInputAndErrorResponses(t *testing.T) {
	router, err := newRouter(config{
		RequestID: true,
		Sites:     []siteConfig{{Hosts: []string{"example.com"}, Target: "http://127.0.0.1:1"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	req := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
	req.Header[requestIDHeader] = []string{"one", "two"}
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, req)
	if got := recorder.Result().Header.Get(requestIDHeader); len(got) != 32 {
		t.Fatalf("generated response X-Request-ID = %q; want 32 characters", got)
	}

	unknown := httptest.NewRecorder()
	unknownReq := httptest.NewRequest(http.MethodGet, "http://unknown.example/", nil)
	router.ServeHTTP(unknown, unknownReq)
	if unknown.Code != http.StatusNotFound {
		t.Fatalf("unknown host status = %d; want 404", unknown.Code)
	}
	if got := unknown.Result().Header.Get(requestIDHeader); len(got) != 32 {
		t.Fatalf("404 X-Request-ID = %q; want 32 characters", got)
	}

	options := httptest.NewRecorder()
	optionsReq := httptest.NewRequest(http.MethodOptions, "http://example.com/", nil)
	optionsReq.RequestURI = "*"
	router.ServeHTTP(options, optionsReq)
	if got := options.Result().Header.Get(requestIDHeader); len(got) != 32 {
		t.Fatalf("OPTIONS * X-Request-ID = %q; want 32 characters", got)
	}
}

func TestRequestIDAccessLogContainsAcceptedID(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("ok"))
	}))
	defer upstream.Close()

	router, err := newRouter(config{
		RequestID: true,
		Sites:     []siteConfig{{Hosts: []string{"example.com"}, Target: upstream.URL}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	var output bytes.Buffer
	router.accessLogger = newAccessLogger(&output)

	req := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
	req.Header.Set(requestIDHeader, "log-id")
	router.ServeHTTP(httptest.NewRecorder(), req)

	var record struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(output.Bytes(), &record); err != nil {
		t.Fatal(err)
	}
	if record.ID != "log-id" {
		t.Fatalf("access log ID = %q; want log-id", record.ID)
	}
}

func TestRequestIDConfigRequiresBoolean(t *testing.T) {
	_, err := decodeConfig([]byte(`{"listen":":8080","id":"true","sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`))
	if err == nil {
		t.Fatal("decodeConfig() accepted a non-boolean id")
	}
}
