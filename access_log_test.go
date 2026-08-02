package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"
)

func TestDecodeAccessLogConfiguration(t *testing.T) {
	for _, test := range []struct {
		name string
		raw  string
		want bool
	}{
		{name: "absent", raw: `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`},
		{name: "false", raw: `{"listen":":8080","log":false,"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`},
		{name: "true", raw: `{"listen":":8080","log":true,"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`, want: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			cfg, err := decodeConfig([]byte(test.raw))
			if err != nil {
				t.Fatal(err)
			}
			if cfg.Log != test.want {
				t.Fatalf("log = %t, want %t", cfg.Log, test.want)
			}
		})
	}

	invalid := map[string]string{
		"null":              `{"listen":":8080","log":null,"sites":[]}`,
		"string":            `{"listen":":8080","log":"true","sites":[]}`,
		"number":            `{"listen":":8080","log":1,"sites":[]}`,
		"array":             `{"listen":":8080","log":[],"sites":[]}`,
		"object":            `{"listen":":8080","log":{},"sites":[]}`,
		"alias":             `{"listen":":8080","accessLog":true,"sites":[]}`,
		"duplicate":         `{"listen":":8080","log":true,"log":false,"sites":[]}`,
		"site field":        `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","log":true}]}`,
		"route field":       `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","log":true}]}]}`,
		"backend field":     `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"log":true}},"sites":[]}`,
		"timeout field":     `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"timeout":{"dial":"1s","log":true}}},"sites":[]}`,
		"response field":    `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","response":{"remove":["Server"],"log":true}}]}`,
		"environment value": `{"listen":":8080","log":"${KUBIO_LOG}","sites":[]}`,
	}
	for name, raw := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(raw)); err == nil {
				t.Fatal("invalid log configuration was accepted")
			}
		})
	}
}

func TestAccessLogRecordsOriginalRequestAndResponse(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("X-Secret", "response-secret")
		w.WriteHeader(http.StatusCreated)
		_, _ = w.Write([]byte("ok"))
	}))
	defer backend.Close()

	handler, err := newRouter(config{Log: true, Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: backend.URL,
		Routes: []routeConfig{{Path: "/api/*", Strip: true}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	handler.accessLogger = log.New(&output, "", 0)
	req := httptest.NewRequest(http.MethodPost, "http://proxy/api/users%2Factive?token=request-secret", nil)
	req.Host = "example.com:8443"
	req.RemoteAddr = "192.0.2.10:1234"
	req.Header.Set("Authorization", "request-secret")
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)

	if res.Code != http.StatusCreated || res.Body.String() != "ok" {
		t.Fatalf("response = %d %q", res.Code, res.Body.String())
	}
	line := strings.TrimSuffix(output.String(), "\n")
	if strings.Count(output.String(), "\n") != 1 || strings.Contains(line, "secret") {
		t.Fatalf("access log = %q", output.String())
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal([]byte(line), &fields); err != nil {
		t.Fatalf("invalid JSON log: %v", err)
	}
	if len(fields) != 9 {
		t.Fatalf("field count = %d, want 9", len(fields))
	}
	var record accessRecord
	if err := json.Unmarshal([]byte(line), &record); err != nil {
		t.Fatal(err)
	}
	if record.Method != http.MethodPost || record.Host != "example.com:8443" ||
		record.Path != "/api/users%2Factive" || record.Proto != "HTTP/1.1" ||
		record.Peer != "192.0.2.10" || record.Status != http.StatusCreated || record.Bytes != 2 {
		t.Fatalf("record = %#v", record)
	}
	if _, err := time.Parse(time.RFC3339Nano, record.Time); err != nil || record.DurationUs < 0 {
		t.Fatalf("time=%q durationUs=%d", record.Time, record.DurationUs)
	}
}

func TestAccessLogObservesResponseWriter(t *testing.T) {
	underlying := &hijackResponseWriter{}
	observed := &accessResponseWriter{ResponseWriter: underlying}
	observed.WriteHeader(http.StatusEarlyHints)
	if observed.status != 0 {
		t.Fatalf("informational status recorded as %d", observed.status)
	}
	if _, err := observed.Write([]byte("abc")); err != nil {
		t.Fatal(err)
	}
	observed.WriteHeader(http.StatusInternalServerError)
	if observed.status != http.StatusOK || observed.bytes != 3 {
		t.Fatalf("status=%d bytes=%d", observed.status, observed.bytes)
	}
	if observed.Unwrap() != underlying {
		t.Fatal("underlying writer was not exposed")
	}

	flushed := &accessResponseWriter{ResponseWriter: httptest.NewRecorder()}
	if err := flushed.FlushError(); err != nil || flushed.status != http.StatusOK {
		t.Fatalf("flush: status=%d err=%v", flushed.status, err)
	}
	unsupported := &accessResponseWriter{ResponseWriter: &hijackResponseWriter{}}
	if err := unsupported.FlushError(); !errors.Is(err, http.ErrNotSupported) || unsupported.status != 0 {
		t.Fatalf("unsupported flush: status=%d err=%v", unsupported.status, err)
	}

	server, client := net.Pipe()
	defer client.Close()
	hijacker := &hijackResponseWriter{connection: server}
	upgraded := &accessResponseWriter{ResponseWriter: hijacker}
	connection, _, err := upgraded.Hijack()
	if err != nil {
		t.Fatal(err)
	}
	_ = connection.Close()
	if upgraded.status != http.StatusSwitchingProtocols {
		t.Fatalf("hijack status = %d", upgraded.status)
	}
}

func TestAccessLogPanicsAreLoggedAndPreserved(t *testing.T) {
	handler, err := newRouter(config{Log: true, Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}}})
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	handler.accessLogger = log.New(&output, "", 0)
	want := errors.New("panic marker")
	handler.sites[0].proxy.Rewrite = func(*httputil.ProxyRequest) { panic(want) }
	func() {
		defer func() {
			if got := recover(); got != want {
				t.Fatalf("panic = %v, want %v", got, want)
			}
		}()
		handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	}()
	var record accessRecord
	if err := json.Unmarshal(bytes.TrimSpace(output.Bytes()), &record); err != nil {
		t.Fatal(err)
	}
	if record.Status != http.StatusInternalServerError || record.Bytes != 0 {
		t.Fatalf("record = %#v", record)
	}
}

func TestAccessLogHandlesOptionsStar(t *testing.T) {
	handler, err := newRouter(config{Log: true, Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}}})
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	handler.accessLogger = log.New(&output, "", 0)
	req := httptest.NewRequest(http.MethodOptions, "http://proxy/", strings.NewReader(strings.Repeat("x", 5<<10)))
	req.RequestURI = "*"
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	if res.Code != http.StatusOK || res.Header().Get("Content-Length") != "0" || res.Body.Len() != 0 {
		t.Fatalf("response = %d headers=%v body=%q", res.Code, res.Header(), res.Body.String())
	}
	var record accessRecord
	if err := json.Unmarshal(bytes.TrimSpace(output.Bytes()), &record); err != nil {
		t.Fatal(err)
	}
	if record.Status != http.StatusOK || record.Bytes != 0 {
		t.Fatalf("record = %#v", record)
	}
}

func TestAccessLogReloadKeepsRequestGeneration(t *testing.T) {
	entered := make(chan struct{})
	release := make(chan struct{})
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		close(entered)
		<-release
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()
	old, err := newRouter(config{Log: true, Sites: []siteConfig{{Hosts: []string{"*"}, Target: backend.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	old.accessLogger = log.New(&output, "", 0)
	current := newReloadableRouter(old)
	next, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}}})
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	go func() {
		current.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "http://proxy/old", nil))
		close(done)
	}()
	<-entered
	current.Store(next)
	close(release)
	<-done
	if strings.Count(output.String(), "\n") != 1 || next.accessLogger != nil {
		t.Fatalf("old log=%q new logger=%v", output.String(), next.accessLogger)
	}
}

func TestAccessLogWriteFailureDoesNotChangeResponse(t *testing.T) {
	handler, err := newRouter(config{Log: true, Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: "http://localhost:3000"}}})
	if err != nil {
		t.Fatal(err)
	}
	handler.accessLogger = log.New(failingLogWriter{}, "", 0)
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, httptest.NewRequest(http.MethodGet, "http://other.test/", nil))
	if res.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404", res.Code)
	}
}

func TestAccessLogBrokenStdoutDoesNotTerminateProcess(t *testing.T) {
	if os.Getenv("KUBIO_TEST_BROKEN_STDOUT") == "1" {
		handler, err := newRouter(config{Log: true, Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}}})
		if err != nil {
			panic(err)
		}
		req := httptest.NewRequest(http.MethodOptions, "http://proxy/", nil)
		req.RequestURI = "*"
		handler.ServeHTTP(httptest.NewRecorder(), req)
		return
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	_ = reader.Close()
	defer writer.Close()
	cmd := exec.Command(os.Args[0], "-test.run=^TestAccessLogBrokenStdoutDoesNotTerminateProcess$")
	cmd.Env = append(os.Environ(), "KUBIO_TEST_BROKEN_STDOUT=1")
	cmd.Stdout = writer
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		t.Fatalf("child terminated: %v; stderr=%s", err, stderr.String())
	}
}

type failingLogWriter struct{}

func (failingLogWriter) Write([]byte) (int, error) { return 0, errors.New("write failed") }

type hijackResponseWriter struct {
	header     http.Header
	connection net.Conn
}

func (w *hijackResponseWriter) Header() http.Header {
	if w.header == nil {
		w.header = make(http.Header)
	}
	return w.header
}

func (*hijackResponseWriter) WriteHeader(int) {}

func (*hijackResponseWriter) Write(data []byte) (int, error) { return len(data), nil }

func (w *hijackResponseWriter) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	return w.connection, bufio.NewReadWriter(bufio.NewReader(w.connection), bufio.NewWriter(w.connection)), nil
}
