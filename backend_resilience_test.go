package main

import (
	"bufio"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httptrace"
	"net/textproto"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestBackendResilienceConfig(t *testing.T) {
	valid := `{
  "listen": ":8080",
  "backends": {
    "app": {
      "targets": ["http://app-1:3000", "http://app-2:3000", "http://app-3:3000"],
      "tries": 2,
      "timeout": {"dial": "250ms", "header": "1m30s"}
    }
  },
  "sites": [{"hosts": ["*"], "backend": "app"}]
}`
	cfg, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	backend := cfg.Backends["app"]
	if backend.Tries != 2 || backend.Timeout.Dial != 250*time.Millisecond || backend.Timeout.Header != 90*time.Second {
		t.Fatalf("decoded backend = %+v", backend)
	}

	legacy, err := decodeConfig([]byte(`{"listen":":8080","backends":{"app":{"targets":["http://app:3000"]}},"sites":[{"hosts":["*"],"backend":"app"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	if got := legacy.Backends["app"]; got.Tries != 1 || got.Timeout != (backendTimeout{}) {
		t.Fatalf("default backend = %+v", got)
	}

	withBackend := func(fields string) string {
		return `{"listen":":8080","backends":{"app":{"targets":["http://a:3000","http://b:3000"],` + fields + `}},"sites":[{"hosts":["*"],"backend":"app"}]}`
	}
	invalid := map[string]string{
		"tries null":              withBackend(`"tries":null`),
		"tries string":            withBackend(`"tries":"1"`),
		"tries boolean":           withBackend(`"tries":true`),
		"tries decimal":           withBackend(`"tries":1.0`),
		"tries exponent":          withBackend(`"tries":1e0`),
		"tries zero":              withBackend(`"tries":0`),
		"tries negative":          withBackend(`"tries":-1`),
		"tries above targets":     withBackend(`"tries":3`),
		"timeout null":            withBackend(`"timeout":null`),
		"timeout array":           withBackend(`"timeout":[]`),
		"timeout empty":           withBackend(`"timeout":{}`),
		"timeout unknown":         withBackend(`"timeout":{"read":"1s"}`),
		"dial null":               withBackend(`"timeout":{"dial":null}`),
		"dial number":             withBackend(`"timeout":{"dial":1}`),
		"dial empty":              withBackend(`"timeout":{"dial":""}`),
		"dial zero":               withBackend(`"timeout":{"dial":"0s"}`),
		"dial negative":           withBackend(`"timeout":{"dial":"-1s"}`),
		"dial whitespace":         withBackend(`"timeout":{"dial":" 1s"}`),
		"header invalid":          withBackend(`"timeout":{"header":"soon"}`),
		"duration environment":    withBackend(`"timeout":{"header":"${KUBIO_TIMEOUT}"}`),
		"duplicate timeout field": withBackend(`"timeout":{"dial":"1s","dial":"2s"}`),
		"site timeout":            `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://app:3000","timeout":{"dial":"1s"}}]}`,
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(data)); err == nil {
				t.Fatal("invalid config was accepted")
			}
		})
	}

	const secret = "DURATION_VALUE_MUST_NOT_LEAK"
	_, err = decodeConfig([]byte(withBackend(`"timeout":{"dial":"` + secret + `"}`)))
	if err == nil || strings.Contains(err.Error(), secret) {
		t.Fatalf("duration was accepted or leaked: %v", err)
	}
}

func TestBackendResilienceDefaultsAndValidation(t *testing.T) {
	backend, err := newBackend(backendConfig{Targets: []string{"http://a:3000", "http://b:3000"}})
	if err != nil {
		t.Fatal(err)
	}
	if backend.tries != 1 || backend.transport.TLSHandshakeTimeout != proxyDialTimeout ||
		backend.transport.ResponseHeaderTimeout != proxyResponseHeaderLimit {
		t.Fatalf("default backend = tries %d, TLS %s, header %s", backend.tries, backend.transport.TLSHandshakeTimeout, backend.transport.ResponseHeaderTimeout)
	}

	backend, err = newBackend(backendConfig{
		Targets: []string{"http://a:3000", "http://b:3000"},
		Tries:   2,
		Timeout: backendTimeout{Dial: 7 * time.Second, Header: 11 * time.Second},
	})
	if err != nil {
		t.Fatal(err)
	}
	if backend.tries != 2 || backend.transport.TLSHandshakeTimeout != 7*time.Second ||
		backend.transport.ResponseHeaderTimeout != 11*time.Second {
		t.Fatalf("custom backend = tries %d, TLS %s, header %s", backend.tries, backend.transport.TLSHandshakeTimeout, backend.transport.ResponseHeaderTimeout)
	}

	invalid := []backendConfig{
		{Targets: []string{"http://a:3000"}, Tries: -1},
		{Targets: []string{"http://a:3000"}, Tries: 2},
		{Targets: []string{"http://a:3000"}, Timeout: backendTimeout{Dial: -time.Second}},
		{Targets: []string{"http://a:3000"}, Timeout: backendTimeout{Header: -time.Second}},
	}
	for _, cfg := range invalid {
		if _, err := newBackend(cfg); err == nil {
			t.Fatalf("invalid backend accepted: %+v", cfg)
		}
	}
}

func TestBackendRetriesDistinctTargetsWithoutAdvancingSelector(t *testing.T) {
	var hits atomic.Int32
	healthy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		hits.Add(1)
		w.Header().Set("X-Policy", "upstream")
		_, _ = io.WriteString(w, "healthy")
	}))
	defer healthy.Close()
	closed := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	closedURL := closed.URL
	closed.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{closedURL, healthy.URL}, Tries: 2}},
		Sites: []siteConfig{{
			Hosts: []string{"*"}, Backend: "app",
			ResponseHeaders: responseHeaderPolicy{Set: map[string][]string{"X-Policy": {"configured"}}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}

	for request := 1; request <= 3; request++ {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
		if response.Code != http.StatusOK || response.Body.String() != "healthy" || response.Header().Get("X-Policy") != "configured" {
			t.Fatalf("request %d = %d %q %q", request, response.Code, response.Body.String(), response.Header().Get("X-Policy"))
		}
	}
	if hits.Load() != 3 {
		t.Fatalf("healthy hits = %d, want 3", hits.Load())
	}
}

func TestBackendDoesNotRetryHTTPResponse(t *testing.T) {
	var unavailableHits atomic.Int32
	unavailable := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		unavailableHits.Add(1)
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer unavailable.Close()

	var healthyHits atomic.Int32
	healthy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		healthyHits.Add(1)
		_, _ = io.WriteString(w, "healthy")
	}))
	defer healthy.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{unavailable.URL, healthy.URL}, Tries: 2}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}

	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusServiceUnavailable || unavailableHits.Load() != 1 || healthyHits.Load() != 0 {
		t.Fatalf("first response = %d, unavailable=%d, healthy=%d", response.Code, unavailableHits.Load(), healthyHits.Load())
	}
	if got := proxyResponse(t, handler, "proxy", "/"); got != "healthy" || healthyHits.Load() != 1 {
		t.Fatalf("second response = %q, healthy=%d", got, healthyHits.Load())
	}
}

func TestRetryableBackendRequest(t *testing.T) {
	request := func(method string) *http.Request {
		return httptest.NewRequest(method, "http://proxy/", nil)
	}
	emptyBody := request(http.MethodGet)
	emptyBody.Body = io.NopCloser(strings.NewReader(""))
	body := httptest.NewRequest(http.MethodGet, "http://proxy/", strings.NewReader("body"))
	unknownLength := request(http.MethodGet)
	unknownLength.ContentLength = -1
	transfer := request(http.MethodGet)
	transfer.TransferEncoding = []string{"chunked"}
	trailer := request(http.MethodGet)
	trailer.Trailer = http.Header{"X-Late": nil}
	upgrade := request(http.MethodGet)
	upgrade.Header.Set("Connection", "keep-alive, UpGrAdE")
	upgrade.Header.Set("Upgrade", "websocket")
	upgradeOnly := request(http.MethodGet)
	upgradeOnly.Header.Set("Upgrade", "websocket")
	connectionOnly := request(http.MethodGet)
	connectionOnly.Header.Set("Connection", "upgrade")

	tests := []struct {
		name    string
		request *http.Request
		want    bool
	}{
		{name: "GET", request: request(http.MethodGet), want: true},
		{name: "HEAD", request: request(http.MethodHead), want: true},
		{name: "OPTIONS", request: request(http.MethodOptions), want: true},
		{name: "TRACE", request: request(http.MethodTrace), want: true},
		{name: "lowercase", request: request("get")},
		{name: "POST", request: request(http.MethodPost)},
		{name: "CONNECT", request: request(http.MethodConnect)},
		{name: "empty body object", request: emptyBody},
		{name: "body", request: body},
		{name: "unknown length", request: unknownLength},
		{name: "transfer encoding", request: transfer},
		{name: "trailer", request: trailer},
		{name: "upgrade", request: upgrade},
		{name: "upgrade header only", request: upgradeOnly, want: true},
		{name: "connection token only", request: connectionOnly, want: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := retryableBackendRequest(test.request); got != test.want {
				t.Fatalf("retryable = %t, want %t", got, test.want)
			}
		})
	}
}

func TestBackendDoesNotRetryIneligibleRequests(t *testing.T) {
	var healthyHits atomic.Int32
	healthy := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		healthyHits.Add(1)
	}))
	defer healthy.Close()
	closed := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	closedURL := closed.URL
	closed.Close()

	requests := map[string]func() *http.Request{
		"method": func() *http.Request {
			return httptest.NewRequest(http.MethodPost, "http://proxy/", nil)
		},
		"body": func() *http.Request {
			return httptest.NewRequest(http.MethodGet, "http://proxy/", strings.NewReader("body"))
		},
		"empty body object": func() *http.Request {
			request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
			request.Body = io.NopCloser(strings.NewReader(""))
			return request
		},
		"transfer encoding": func() *http.Request {
			request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
			request.TransferEncoding = []string{"chunked"}
			return request
		},
		"trailer": func() *http.Request {
			request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
			request.Trailer = http.Header{"X-Late": nil}
			return request
		},
		"upgrade": func() *http.Request {
			request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
			request.Header.Set("Connection", "Upgrade")
			request.Header.Set("Upgrade", "websocket")
			return request
		},
	}
	for name, newRequest := range requests {
		t.Run(name, func(t *testing.T) {
			healthyHits.Store(0)
			handler, err := newRouter(config{
				Backends: map[string]backendConfig{"app": {Targets: []string{closedURL, healthy.URL}, Tries: 2}},
				Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
			})
			if err != nil {
				t.Fatal(err)
			}
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, newRequest())
			if response.Code != http.StatusBadGateway || healthyHits.Load() != 0 {
				t.Fatalf("response = %d, healthy hits = %d", response.Code, healthyHits.Load())
			}
		})
	}
}

func TestBackendHeaderTimeoutRetriesAndClassifiesLastError(t *testing.T) {
	blocked := func() (*httptest.Server, chan struct{}) {
		release := make(chan struct{})
		server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
			<-release
		}))
		return server, release
	}

	t.Run("retry succeeds", func(t *testing.T) {
		slow, release := blocked()
		defer func() {
			close(release)
			slow.Close()
		}()
		healthy := newTextBackend(t, "healthy")
		defer healthy.Close()
		handler, err := newRouter(config{
			Backends: map[string]backendConfig{"app": {
				Targets: []string{slow.URL, healthy.URL}, Tries: 2,
				Timeout: backendTimeout{Header: 30 * time.Millisecond},
			}},
			Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		})
		if err != nil {
			t.Fatal(err)
		}
		if got := proxyResponse(t, handler, "proxy", "/"); got != "healthy" {
			t.Fatalf("response = %q", got)
		}
	})

	t.Run("last timeout is 504", func(t *testing.T) {
		slow, release := blocked()
		defer func() {
			close(release)
			slow.Close()
		}()
		closed := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
		closedURL := closed.URL
		closed.Close()
		handler, err := newRouter(config{
			Backends: map[string]backendConfig{"app": {
				Targets: []string{closedURL, slow.URL}, Tries: 2,
				Timeout: backendTimeout{Header: 30 * time.Millisecond},
			}},
			Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		})
		if err != nil {
			t.Fatal(err)
		}
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
		if response.Code != http.StatusGatewayTimeout {
			t.Fatalf("status = %d, want %d", response.Code, http.StatusGatewayTimeout)
		}
	})
}

func TestBackendDoesNotRetryAfterInformationalResponse(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	release := make(chan struct{})
	serverErr := make(chan error, 1)
	go func() {
		connection, err := listener.Accept()
		if err != nil {
			serverErr <- err
			return
		}
		defer connection.Close()
		reader := bufio.NewReader(connection)
		for {
			line, err := reader.ReadString('\n')
			if err != nil {
				serverErr <- err
				return
			}
			if line == "\r\n" {
				break
			}
		}
		if _, err := io.WriteString(connection, "HTTP/1.1 103 Early Hints\r\nX-Early: yes\r\n\r\n"); err != nil {
			serverErr <- err
			return
		}
		<-release
		serverErr <- nil
	}()

	var healthyHits atomic.Int32
	healthy := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		healthyHits.Add(1)
	}))
	defer healthy.Close()
	backend, err := newBackend(backendConfig{
		Targets: []string{"http://" + listener.Addr().String(), healthy.URL},
		Tries:   2,
		Timeout: backendTimeout{Header: 100 * time.Millisecond},
	})
	if err != nil {
		t.Fatal(err)
	}

	var informational atomic.Int32
	traceError := errors.New("stop after informational response")
	request, err := http.NewRequest(http.MethodGet, "http://proxy/", nil)
	if err != nil {
		t.Fatal(err)
	}
	trace := &httptrace.ClientTrace{Got1xxResponse: func(code int, _ textproto.MIMEHeader) error {
		if code == http.StatusEarlyHints {
			informational.Add(1)
		}
		return traceError
	}}
	request = request.WithContext(httptrace.WithClientTrace(request.Context(), trace))
	start := backend.nextTargetIndex()
	request.URL.Scheme = backend.targets[start].Scheme
	request.URL.Host = backend.targets[start].Host
	request = withBackendRetry(request, start)
	_, err = backend.RoundTrip(request)
	close(release)
	if serverError := <-serverErr; serverError != nil {
		t.Fatal(serverError)
	}
	if !errors.Is(err, traceError) {
		t.Fatalf("error = %v, want trace error", err)
	}
	if informational.Load() != 1 || healthyHits.Load() != 0 {
		t.Fatalf("informational = %d, healthy hits = %d", informational.Load(), healthyHits.Load())
	}
}
