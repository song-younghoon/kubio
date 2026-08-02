package main

import (
	"bufio"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httptrace"
	"net/textproto"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func closedBackendURL(t *testing.T) string {
	t.Helper()
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	url := server.URL
	server.Close()
	return url
}

func TestBackendResilienceConfig(t *testing.T) {
	valid := `{
  "listen": ":8080",
  "backends": {
    "app": {
      "targets": ["http://app-1:3000", "http://app-2:3000", "http://app-3:3000"],
      "tries": 2,
      "retry": {"status": [502, 503, 504], "delay": "25ms", "deadline": "2s"},
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
	if backend.Tries != 2 || backend.Timeout.Dial != 250*time.Millisecond || backend.Timeout.Header != 90*time.Second ||
		backend.Retry.Delay != 25*time.Millisecond || backend.Retry.Deadline != 2*time.Second ||
		!slices.Equal(backend.Retry.Status, []int{502, 503, 504}) {
		t.Fatalf("decoded backend = %+v", backend)
	}

	legacy, err := decodeConfig([]byte(`{"listen":":8080","backends":{"app":{"targets":["http://app:3000"]}},"sites":[{"hosts":["*"],"backend":"app"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	if got := legacy.Backends["app"]; got.Tries != 1 || got.Timeout != (backendTimeout{}) || got.Retry != nil {
		t.Fatalf("default backend = %+v", got)
	}

	withBackend := func(fields string) string {
		return `{"listen":":8080","backends":{"app":{"targets":["http://a:3000","http://b:3000"],` + fields + `}},"sites":[{"hosts":["*"],"backend":"app"}]}`
	}
	invalid := map[string]string{
		"tries null":                withBackend(`"tries":null`),
		"tries string":              withBackend(`"tries":"1"`),
		"tries boolean":             withBackend(`"tries":true`),
		"tries decimal":             withBackend(`"tries":1.0`),
		"tries exponent":            withBackend(`"tries":1e0`),
		"tries zero":                withBackend(`"tries":0`),
		"tries negative":            withBackend(`"tries":-1`),
		"tries above targets":       withBackend(`"tries":3`),
		"timeout null":              withBackend(`"timeout":null`),
		"timeout array":             withBackend(`"timeout":[]`),
		"timeout empty":             withBackend(`"timeout":{}`),
		"timeout unknown":           withBackend(`"timeout":{"read":"1s"}`),
		"dial null":                 withBackend(`"timeout":{"dial":null}`),
		"dial number":               withBackend(`"timeout":{"dial":1}`),
		"dial empty":                withBackend(`"timeout":{"dial":""}`),
		"dial zero":                 withBackend(`"timeout":{"dial":"0s"}`),
		"dial negative":             withBackend(`"timeout":{"dial":"-1s"}`),
		"dial whitespace":           withBackend(`"timeout":{"dial":" 1s"}`),
		"header invalid":            withBackend(`"timeout":{"header":"soon"}`),
		"duration environment":      withBackend(`"timeout":{"header":"${KUBIO_TIMEOUT}"}`),
		"duplicate timeout field":   withBackend(`"timeout":{"dial":"1s","dial":"2s"}`),
		"retry null":                withBackend(`"tries":2,"retry":null`),
		"retry scalar":              withBackend(`"tries":2,"retry":true`),
		"retry array":               withBackend(`"tries":2,"retry":[]`),
		"retry empty":               withBackend(`"tries":2,"retry":{}`),
		"retry missing tries":       withBackend(`"retry":{"status":[503]}`),
		"retry one try":             withBackend(`"tries":1,"retry":{"status":[503]}`),
		"retry null status":         withBackend(`"tries":2,"retry":{"status":null}`),
		"retry scalar status":       withBackend(`"tries":2,"retry":{"status":503}`),
		"retry empty status":        withBackend(`"tries":2,"retry":{"status":[]}`),
		"retry status string":       withBackend(`"tries":2,"retry":{"status":["503"]}`),
		"retry status decimal":      withBackend(`"tries":2,"retry":{"status":[503.0]}`),
		"retry status exponent":     withBackend(`"tries":2,"retry":{"status":[5.03e2]}`),
		"retry status low":          withBackend(`"tries":2,"retry":{"status":[399]}`),
		"retry status high":         withBackend(`"tries":2,"retry":{"status":[600]}`),
		"retry duplicate status":    withBackend(`"tries":2,"retry":{"status":[503,503]}`),
		"retry duplicate field":     withBackend(`"tries":2,"retry":{"status":[503],"status":[504]}`),
		"retry unknown field":       withBackend(`"tries":2,"retry":{"status":[503],"codes":[503]}`),
		"retry alias":               withBackend(`"tries":2,"retries":{"status":[503]}`),
		"retry delay null":          withBackend(`"tries":2,"retry":{"status":[503],"delay":null}`),
		"retry delay number":        withBackend(`"tries":2,"retry":{"status":[503],"delay":1}`),
		"retry delay empty":         withBackend(`"tries":2,"retry":{"status":[503],"delay":""}`),
		"retry delay zero":          withBackend(`"tries":2,"retry":{"status":[503],"delay":"0s"}`),
		"retry delay negative":      withBackend(`"tries":2,"retry":{"status":[503],"delay":"-1s"}`),
		"retry delay whitespace":    withBackend(`"tries":2,"retry":{"status":[503],"delay":" 1s"}`),
		"retry deadline null":       withBackend(`"tries":2,"retry":{"status":[503],"deadline":null}`),
		"retry deadline number":     withBackend(`"tries":2,"retry":{"status":[503],"deadline":1}`),
		"retry deadline empty":      withBackend(`"tries":2,"retry":{"status":[503],"deadline":""}`),
		"retry deadline zero":       withBackend(`"tries":2,"retry":{"status":[503],"deadline":"0s"}`),
		"retry deadline negative":   withBackend(`"tries":2,"retry":{"status":[503],"deadline":"-1s"}`),
		"retry deadline whitespace": withBackend(`"tries":2,"retry":{"status":[503],"deadline":" 1s"}`),
		"retry duplicate delay":     withBackend(`"tries":2,"retry":{"status":[503],"delay":"1s","delay":"2s"}`),
		"site timeout":              `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://app:3000","timeout":{"dial":"1s"}}]}`,
		"site retry":                `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://app:3000","retry":{"status":[503]}}]}`,
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
		backend.transport.ResponseHeaderTimeout != proxyResponseHeaderTimeout {
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
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Delay: -time.Second}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Deadline: -time.Second}},
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
	closedURL := closedBackendURL(t)

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

func TestBackendRetriesConfiguredStatuses(t *testing.T) {
	var firstHits atomic.Int32
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		firstHits.Add(1)
		w.Header().Set("X-Attempt", "first")
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = io.WriteString(w, "unavailable")
	}))
	defer first.Close()

	var secondHits atomic.Int32
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		secondHits.Add(1)
		w.Header().Set("X-Attempt", "second")
		_, _ = io.WriteString(w, "healthy")
	}))
	defer second.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL},
			Tries:   2,
			Retry:   &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}

	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusOK || response.Body.String() != "healthy" || response.Header().Get("X-Attempt") != "second" {
		t.Fatalf("response = %d %q headers=%v", response.Code, response.Body.String(), response.Header())
	}
	if firstHits.Load() != 1 || secondHits.Load() != 1 {
		t.Fatalf("hits = first %d, second %d", firstHits.Load(), secondHits.Load())
	}
}

func TestBackendRetryDelayAppliesBeforeNextAttempt(t *testing.T) {
	const delay = 30 * time.Millisecond

	t.Run("status response", func(t *testing.T) {
		first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			w.WriteHeader(http.StatusServiceUnavailable)
		}))
		defer first.Close()
		second := newTextBackend(t, "healthy")
		defer second.Close()

		handler, err := newRouter(config{
			Backends: map[string]backendConfig{"app": {
				Targets: []string{first.URL, second.URL}, Tries: 2,
				Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Delay: delay},
			}},
			Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		})
		if err != nil {
			t.Fatal(err)
		}
		started := time.Now()
		if got := proxyResponse(t, handler, "proxy", "/"); got != "healthy" {
			t.Fatalf("response = %q", got)
		}
		if elapsed := time.Since(started); elapsed < delay-5*time.Millisecond {
			t.Fatalf("retry elapsed = %s, want at least %s", elapsed, delay-5*time.Millisecond)
		}
	})

	t.Run("transport error", func(t *testing.T) {
		first := closedBackendURL(t)
		second := newTextBackend(t, "healthy")
		defer second.Close()
		handler, err := newRouter(config{
			Backends: map[string]backendConfig{"app": {
				Targets: []string{first, second.URL}, Tries: 2,
				Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Delay: delay},
			}},
			Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		})
		if err != nil {
			t.Fatal(err)
		}
		started := time.Now()
		if got := proxyResponse(t, handler, "proxy", "/"); got != "healthy" {
			t.Fatalf("response = %q", got)
		}
		if elapsed := time.Since(started); elapsed < delay-5*time.Millisecond {
			t.Fatalf("retry elapsed = %s, want at least %s", elapsed, delay-5*time.Millisecond)
		}
	})
}

func TestBackendRetryDelayStopsWhenContextIsCanceled(t *testing.T) {
	firstDone := make(chan struct{})
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
		close(firstDone)
	}))
	defer first.Close()
	secondHits := atomic.Int32{}
	second := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		secondHits.Add(1)
	}))
	defer second.Close()

	backend, err := newBackend(backendConfig{
		Targets: []string{first.URL, second.URL}, Tries: 2,
		Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Delay: time.Second},
	})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil).WithContext(ctx)
	start := backend.nextTargetIndex()
	request.URL.Scheme = backend.targets[start].Scheme
	request.URL.Host = backend.targets[start].Host
	result := make(chan error, 1)
	go func() {
		_, err := backend.RoundTrip(withBackendRetry(request, start))
		result <- err
	}()
	<-firstDone
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatalf("error = %v, want context canceled", err)
	}
	if secondHits.Load() != 0 {
		t.Fatalf("second target hits = %d, want 0", secondHits.Load())
	}
}

func TestBackendRetryDeadlineBoundsAttemptSequence(t *testing.T) {
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		<-request.Context().Done()
	}))
	defer first.Close()
	secondHits := atomic.Int32{}
	second := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		secondHits.Add(1)
	}))
	defer second.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL}, Tries: 2,
			Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Deadline: 30 * time.Millisecond},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusGatewayTimeout || secondHits.Load() != 0 {
		t.Fatalf("response = %d, second target hits = %d", response.Code, secondHits.Load())
	}
}

func TestBackendRetryDeadlineCoversFinalResponseBody(t *testing.T) {
	block := make(chan struct{})
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/plain")
		w.WriteHeader(http.StatusOK)
		flusher, ok := w.(http.Flusher)
		if !ok {
			return
		}
		_, _ = io.WriteString(w, "part")
		flusher.Flush()
		<-block
	}))
	defer func() {
		close(block)
		first.Close()
	}()
	secondHits := atomic.Int32{}
	second := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		secondHits.Add(1)
	}))
	defer second.Close()

	backend, err := newBackend(backendConfig{
		Targets: []string{first.URL, second.URL}, Tries: 2,
		Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Deadline: 40 * time.Millisecond},
	})
	if err != nil {
		t.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	start := backend.nextTargetIndex()
	request.URL.Scheme = backend.targets[start].Scheme
	request.URL.Host = backend.targets[start].Host
	response, err := backend.RoundTrip(withBackendRetry(request, start))
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	firstBody := make([]byte, 4)
	if _, err := io.ReadFull(response.Body, firstBody); err != nil || string(firstBody) != "part" {
		t.Fatalf("first body = %q, error = %v", firstBody, err)
	}
	_, err = response.Body.Read(make([]byte, 1))
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("body error = %v, want deadline exceeded", err)
	}
	if secondHits.Load() != 0 {
		t.Fatalf("second target hits = %d, want 0", secondHits.Load())
	}
}

func TestBackendStatusRetryStopsAtUnlistedOrFinalStatus(t *testing.T) {
	tests := []struct {
		name       string
		firstCode  int
		secondCode int
		wantCode   int
		wantBody   string
		wantSecond int32
	}{
		{name: "unlisted", firstCode: http.StatusBadGateway, secondCode: http.StatusOK, wantCode: http.StatusBadGateway, wantBody: "first", wantSecond: 0},
		{name: "final matching", firstCode: http.StatusServiceUnavailable, secondCode: http.StatusServiceUnavailable, wantCode: http.StatusServiceUnavailable, wantBody: "second", wantSecond: 1},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			var secondHits atomic.Int32
			first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(test.firstCode)
				_, _ = io.WriteString(w, "first")
			}))
			defer first.Close()
			second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				secondHits.Add(1)
				w.WriteHeader(test.secondCode)
				_, _ = io.WriteString(w, "second")
			}))
			defer second.Close()

			handler, err := newRouter(config{
				Backends: map[string]backendConfig{"app": {
					Targets: []string{first.URL, second.URL},
					Tries:   2,
					Retry:   &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}},
				}},
				Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
			})
			if err != nil {
				t.Fatal(err)
			}
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
			if response.Code != test.wantCode || response.Body.String() != test.wantBody || secondHits.Load() != test.wantSecond {
				t.Fatalf("response = %d %q, second hits = %d", response.Code, response.Body.String(), secondHits.Load())
			}
		})
	}
}

func TestBackendStatusRetryStopsAfterInformationalResponse(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	serverDone := make(chan error, 1)
	go func() {
		connection, err := listener.Accept()
		if err != nil {
			serverDone <- err
			return
		}
		defer connection.Close()
		reader := bufio.NewReader(connection)
		for {
			line, err := reader.ReadString('\n')
			if err != nil {
				serverDone <- err
				return
			}
			if line == "\r\n" {
				break
			}
		}
		_, err = io.WriteString(connection, "HTTP/1.1 103 Early Hints\r\nX-Early: yes\r\n\r\nHTTP/1.1 503 Service Unavailable\r\nContent-Length: 11\r\nConnection: close\r\n\r\nunavailable")
		serverDone <- err
	}()

	var secondHits atomic.Int32
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		secondHits.Add(1)
		_, _ = io.WriteString(w, "healthy")
	}))
	defer second.Close()

	backend, err := newBackend(backendConfig{
		Targets: []string{"http://" + listener.Addr().String(), second.URL},
		Tries:   2,
		Retry:   &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}},
	})
	if err != nil {
		t.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	start := backend.nextTargetIndex()
	request.URL.Scheme = backend.targets[start].Scheme
	request.URL.Host = backend.targets[start].Host
	response, err := backend.RoundTrip(withBackendRetry(request, start))
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusServiceUnavailable || secondHits.Load() != 0 {
		t.Fatalf("response = %d, second hits = %d", response.StatusCode, secondHits.Load())
	}
	if err := <-serverDone; err != nil {
		t.Fatal(err)
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
	closedURL := closedBackendURL(t)

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
				Backends: map[string]backendConfig{"app": {
					Targets: []string{closedURL, healthy.URL},
					Tries:   2,
					Retry:   &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}},
				}},
				Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
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
		closedURL := closedBackendURL(t)
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
