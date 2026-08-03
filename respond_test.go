package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestGeneratedResponseDoesNotContactUpstream(t *testing.T) {
	upstreamRequests := 0
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		upstreamRequests++
		_, _ = w.Write([]byte("upstream"))
	}))
	defer upstream.Close()

	cfg, err := decodeConfig([]byte(`{"listen":":8080","sites":[{"hosts":["example.com"],"target":"` + upstream.URL + `","response":{"set":{"X-Site":["site"],"X-Test":["site"]}},"routes":[{"path":"/maintenance","respond":{"status":503,"body":"temporarily unavailable","headers":{"Content-Type":["text/plain"],"X-Test":["route"]}}}]}]}`))
	if err != nil {
		t.Fatal(err)
	}
	router, err := newRouter(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	recorder := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "http://example.com/maintenance", nil)
	router.ServeHTTP(recorder, req)

	if recorder.Code != http.StatusServiceUnavailable || recorder.Body.String() != "temporarily unavailable" {
		t.Fatalf("response = %d %q; want 503 body", recorder.Code, recorder.Body.String())
	}
	if got := recorder.Header().Get("X-Site"); got != "site" {
		t.Fatalf("site response header = %q; want site", got)
	}
	if got := recorder.Header().Get("X-Test"); got != "route" {
		t.Fatalf("respond header = %q; want route", got)
	}
	if upstreamRequests != 0 {
		t.Fatalf("upstream requests = %d; want 0", upstreamRequests)
	}
}

func TestGeneratedResponseHEADAndOptionsStar(t *testing.T) {
	router, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"example.com"},
		Target: "http://127.0.0.1:1",
		Routes: []routeConfig{{
			Path:    "/maintenance",
			Respond: &generatedResponse{Status: http.StatusServiceUnavailable, Body: []byte("body")},
		}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	head := httptest.NewRecorder()
	headRequest := httptest.NewRequest(http.MethodHead, "http://example.com/maintenance", nil)
	router.ServeHTTP(head, headRequest)
	if head.Code != http.StatusServiceUnavailable || head.Body.Len() != 0 {
		t.Fatalf("HEAD response = %d %q; want 503 and empty body", head.Code, head.Body.String())
	}

	options := httptest.NewRecorder()
	optionsRequest := httptest.NewRequest(http.MethodOptions, "http://example.com/maintenance", nil)
	optionsRequest.RequestURI = "*"
	router.ServeHTTP(options, optionsRequest)
	if options.Code != http.StatusOK || options.Header().Get("Content-Length") != "0" {
		t.Fatalf("OPTIONS * response = %d headers=%v; want existing general response", options.Code, options.Header())
	}
}

func TestGeneratedResponseKeepsRequestID(t *testing.T) {
	router, err := newRouter(config{
		RequestID: true,
		Sites: []siteConfig{{
			Hosts:  []string{"example.com"},
			Target: "http://127.0.0.1:1",
			Routes: []routeConfig{{
				Path: "/maintenance",
				Respond: &generatedResponse{
					Status:  http.StatusServiceUnavailable,
					Headers: responseHeaderPolicy{Set: map[string][]string{requestIDHeader: {"configured"}}},
				},
			}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	req := httptest.NewRequest(http.MethodGet, "http://example.com/maintenance", nil)
	req.Header.Set(requestIDHeader, "fixed-id")
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, req)
	if got := recorder.Result().Header.Get(requestIDHeader); got != "fixed-id" {
		t.Fatalf("generated response ID = %q; want fixed-id", got)
	}
}

func TestGeneratedResponseConfigValidation(t *testing.T) {
	valid := `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":200,"body":"ok","headers":{"X-Test":["yes"]}}}]}]}`
	if _, err := decodeConfig([]byte(valid)); err != nil {
		t.Fatalf("valid respond config rejected: %v", err)
	}
	invalid := []string{
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":101}}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":204,"body":"body"}}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":200,"headers":{"X-Test":"yes"}}}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":200},"target":"http://localhost:3001"}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":200},"strip":true}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/x","respond":{"status":200},"rewrite":"/y"}]}]}`,
	}
	for _, value := range invalid {
		if _, err := decodeConfig([]byte(value)); err == nil {
			t.Fatalf("invalid respond config accepted: %s", value)
		}
	}
}

func TestGeneratedResponseAccessLogHEADBytes(t *testing.T) {
	router, err := newRouter(config{Log: true, Sites: []siteConfig{{
		Hosts:  []string{"example.com"},
		Target: "http://127.0.0.1:1",
		Routes: []routeConfig{{Path: "/x", Respond: &generatedResponse{Status: http.StatusOK, Body: []byte("body")}}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	var output bytes.Buffer
	router.accessLogger = newAccessLogger(&output)

	req := httptest.NewRequest(http.MethodHead, "http://example.com/x", nil)
	router.ServeHTTP(httptest.NewRecorder(), req)
	var record accessRecord
	if err := json.Unmarshal(output.Bytes(), &record); err != nil {
		t.Fatal(err)
	}
	if record.Status != http.StatusOK || record.Bytes != 0 {
		t.Fatalf("access record = %#v; want status 200 and zero bytes", record)
	}
}

func BenchmarkServeGeneratedResponse(b *testing.B) {
	response := &generatedResponse{
		Status:   http.StatusOK,
		Body:     []byte("benchmark body"),
		Prepared: http.Header{"Content-Type": {"text/plain"}, "X-Test": {"value"}},
	}
	request := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		serveGeneratedResponse(httptest.NewRecorder(), request, response)
	}
}
