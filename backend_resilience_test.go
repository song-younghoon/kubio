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

type countingBody struct {
	reader *strings.Reader
	closes atomic.Int32
}

func (b *countingBody) Read(p []byte) (int, error) { return b.reader.Read(p) }

func (b *countingBody) Close() error {
	b.closes.Add(1)
	return nil
}

func TestBackendResilienceConfig(t *testing.T) {
	valid := `{
  "listen": ":8080",
  "backends": {
    "app": {
      "targets": ["http://app-1:3000", "http://app-2:3000", "http://app-3:3000"],
      "weights": [3, 1, 2],
      "tries": 2,
      "retry": {"status": [502, 503, 504], "methods": ["POST", "PUT"], "body": {"max": 1048576}, "backoff": {"base": "25ms", "cap": "50ms", "jitter": "none"}, "deadline": "2s", "budget": {"max": 100, "window": "1s"}},
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
	if !slices.Equal(backend.Weights, []int{3, 1, 2}) || backend.Tries != 2 || backend.Timeout.Dial != 250*time.Millisecond || backend.Timeout.Header != 90*time.Second ||
		!slices.Equal(backend.Retry.Methods, []string{"POST", "PUT"}) || backend.Retry.Body == nil || backend.Retry.Body.Max != 1048576 ||
		backend.Retry.Backoff == nil || backend.Retry.Backoff.Base != 25*time.Millisecond ||
		backend.Retry.Backoff.Cap != 50*time.Millisecond || backend.Retry.Backoff.Jitter ||
		backend.Retry.Budget == nil || backend.Retry.Budget.Max != 100 || backend.Retry.Budget.Window != time.Second ||
		backend.Retry.Deadline != 2*time.Second ||
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
		"tries null":                    withBackend(`"tries":null`),
		"tries string":                  withBackend(`"tries":"1"`),
		"tries boolean":                 withBackend(`"tries":true`),
		"tries decimal":                 withBackend(`"tries":1.0`),
		"tries exponent":                withBackend(`"tries":1e0`),
		"tries zero":                    withBackend(`"tries":0`),
		"tries negative":                withBackend(`"tries":-1`),
		"tries above targets":           withBackend(`"tries":3`),
		"weights null":                  withBackend(`"weights":null`),
		"weights empty":                 withBackend(`"weights":[]`),
		"weights length":                withBackend(`"weights":[1]`),
		"weights decimal":               withBackend(`"weights":[1.0,1]`),
		"weights exponent":              withBackend(`"weights":[1e0,1]`),
		"weights zero":                  withBackend(`"weights":[0,1]`),
		"weights negative":              withBackend(`"weights":[-1,1]`),
		"weights too large":             withBackend(`"weights":[1001,1]`),
		"weights unknown":               withBackend(`"weight":[1,1]`),
		"weights duplicate field":       withBackend(`"weights":[1,1],"weights":[2,2]`),
		"timeout null":                  withBackend(`"timeout":null`),
		"timeout array":                 withBackend(`"timeout":[]`),
		"timeout empty":                 withBackend(`"timeout":{}`),
		"timeout unknown":               withBackend(`"timeout":{"read":"1s"}`),
		"dial null":                     withBackend(`"timeout":{"dial":null}`),
		"dial number":                   withBackend(`"timeout":{"dial":1}`),
		"dial empty":                    withBackend(`"timeout":{"dial":""}`),
		"dial zero":                     withBackend(`"timeout":{"dial":"0s"}`),
		"dial negative":                 withBackend(`"timeout":{"dial":"-1s"}`),
		"dial whitespace":               withBackend(`"timeout":{"dial":" 1s"}`),
		"header invalid":                withBackend(`"timeout":{"header":"soon"}`),
		"duration environment":          withBackend(`"timeout":{"header":"${KUBIO_TIMEOUT}"}`),
		"duplicate timeout field":       withBackend(`"timeout":{"dial":"1s","dial":"2s"}`),
		"retry null":                    withBackend(`"tries":2,"retry":null`),
		"retry scalar":                  withBackend(`"tries":2,"retry":true`),
		"retry array":                   withBackend(`"tries":2,"retry":[]`),
		"retry empty":                   withBackend(`"tries":2,"retry":{}`),
		"retry missing tries":           withBackend(`"retry":{"status":[503]}`),
		"retry one try":                 withBackend(`"tries":1,"retry":{"status":[503]}`),
		"retry null status":             withBackend(`"tries":2,"retry":{"status":null}`),
		"retry scalar status":           withBackend(`"tries":2,"retry":{"status":503}`),
		"retry empty status":            withBackend(`"tries":2,"retry":{"status":[]}`),
		"retry status string":           withBackend(`"tries":2,"retry":{"status":["503"]}`),
		"retry status decimal":          withBackend(`"tries":2,"retry":{"status":[503.0]}`),
		"retry status exponent":         withBackend(`"tries":2,"retry":{"status":[5.03e2]}`),
		"retry status low":              withBackend(`"tries":2,"retry":{"status":[399]}`),
		"retry status high":             withBackend(`"tries":2,"retry":{"status":[600]}`),
		"retry duplicate status":        withBackend(`"tries":2,"retry":{"status":[503,503]}`),
		"retry duplicate field":         withBackend(`"tries":2,"retry":{"status":[503],"status":[504]}`),
		"retry unknown field":           withBackend(`"tries":2,"retry":{"status":[503],"codes":[503]}`),
		"retry alias":                   withBackend(`"tries":2,"retries":{"status":[503]}`),
		"retry methods null":            withBackend(`"tries":2,"retry":{"status":[503],"methods":null}`),
		"retry methods empty":           withBackend(`"tries":2,"retry":{"status":[503],"methods":[]}`),
		"retry methods wildcard":        withBackend(`"tries":2,"retry":{"status":[503],"methods":["POST*"]}`),
		"retry methods duplicate":       withBackend(`"tries":2,"retry":{"status":[503],"methods":["POST","POST"]}`),
		"retry body null":               withBackend(`"tries":2,"retry":{"status":[503],"body":null}`),
		"retry body empty":              withBackend(`"tries":2,"retry":{"status":[503],"body":{}}`),
		"retry body max null":           withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":null}}`),
		"retry body max decimal":        withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":1.0}}`),
		"retry body max exponent":       withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":1e3}}`),
		"retry body max zero":           withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":0}}`),
		"retry body max negative":       withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":-1}}`),
		"retry body max too large":      withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":67108865}}`),
		"retry body unknown":            withBackend(`"tries":2,"retry":{"status":[503],"body":{"limit":1}}`),
		"retry body duplicate max":      withBackend(`"tries":2,"retry":{"status":[503],"body":{"max":1,"max":2}}`),
		"retry budget null":             withBackend(`"tries":2,"retry":{"status":[503],"budget":null}`),
		"retry budget empty":            withBackend(`"tries":2,"retry":{"status":[503],"budget":{}}`),
		"retry budget max decimal":      withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1.0,"window":"1s"}}`),
		"retry budget max exponent":     withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1e3,"window":"1s"}}`),
		"retry budget max zero":         withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":0,"window":"1s"}}`),
		"retry budget max too large":    withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1000001,"window":"1s"}}`),
		"retry budget window null":      withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1,"window":null}}`),
		"retry budget window zero":      withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1,"window":"0s"}}`),
		"retry budget unknown":          withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1,"window":"1s","burst":1}}`),
		"retry budget duplicate max":    withBackend(`"tries":2,"retry":{"status":[503],"budget":{"max":1,"max":2,"window":"1s"}}`),
		"retry delay null":              withBackend(`"tries":2,"retry":{"status":[503],"delay":null}`),
		"retry delay number":            withBackend(`"tries":2,"retry":{"status":[503],"delay":1}`),
		"retry delay empty":             withBackend(`"tries":2,"retry":{"status":[503],"delay":""}`),
		"retry delay zero":              withBackend(`"tries":2,"retry":{"status":[503],"delay":"0s"}`),
		"retry delay negative":          withBackend(`"tries":2,"retry":{"status":[503],"delay":"-1s"}`),
		"retry delay whitespace":        withBackend(`"tries":2,"retry":{"status":[503],"delay":" 1s"}`),
		"retry deadline null":           withBackend(`"tries":2,"retry":{"status":[503],"deadline":null}`),
		"retry deadline number":         withBackend(`"tries":2,"retry":{"status":[503],"deadline":1}`),
		"retry deadline empty":          withBackend(`"tries":2,"retry":{"status":[503],"deadline":""}`),
		"retry deadline zero":           withBackend(`"tries":2,"retry":{"status":[503],"deadline":"0s"}`),
		"retry deadline negative":       withBackend(`"tries":2,"retry":{"status":[503],"deadline":"-1s"}`),
		"retry deadline whitespace":     withBackend(`"tries":2,"retry":{"status":[503],"deadline":" 1s"}`),
		"retry duplicate delay":         withBackend(`"tries":2,"retry":{"status":[503],"delay":"1s","delay":"2s"}`),
		"retry backoff null":            withBackend(`"tries":2,"retry":{"status":[503],"backoff":null}`),
		"retry backoff scalar":          withBackend(`"tries":2,"retry":{"status":[503],"backoff":true}`),
		"retry backoff empty":           withBackend(`"tries":2,"retry":{"status":[503],"backoff":{}}`),
		"retry backoff missing base":    withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"cap":"1s"}}`),
		"retry backoff missing cap":     withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"1s"}}`),
		"retry backoff base invalid":    withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"soon","cap":"1s"}}`),
		"retry backoff cap invalid":     withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"1s","cap":"soon"}}`),
		"retry backoff cap low":         withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"2s","cap":"1s"}}`),
		"retry backoff jitter invalid":  withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"1s","cap":"1s","jitter":"random"}}`),
		"retry backoff duplicate field": withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"1s","cap":"1s"},"backoff":{"base":"2s","cap":"2s"}}`),
		"retry backoff duplicate base":  withBackend(`"tries":2,"retry":{"status":[503],"backoff":{"base":"1s","base":"2s","cap":"2s"}}`),
		"site timeout":                  `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://app:3000","timeout":{"dial":"1s"}}]}`,
		"site retry":                    `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://app:3000","retry":{"status":[503]}}]}`,
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
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Backoff: &backendBackoffConfig{Base: -time.Second, Cap: time.Second}}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Deadline: -time.Second}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Budget: &backendBudgetConfig{Max: 0, Window: time.Second}}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Tries: 2, Retry: &backendRetryConfig{Status: []int{503}, Budget: &backendBudgetConfig{Max: 1, Window: 0}}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Weights: []int{}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Weights: []int{1}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Weights: []int{0, 1}},
		{Targets: []string{"http://a:3000", "http://b:3000"}, Weights: []int{1001, 1}},
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

func TestBackendRetryBackoffAppliesBeforeNextAttempt(t *testing.T) {
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
				Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Backoff: &backendBackoffConfig{Base: delay, Cap: delay}},
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
				Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Backoff: &backendBackoffConfig{Base: delay, Cap: delay}},
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

func TestBackendRetryBackoffIsExponentialAndCapped(t *testing.T) {
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer first.Close()
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer second.Close()
	third := newTextBackend(t, "healthy")
	defer third.Close()

	const base = 20 * time.Millisecond
	const capDelay = 30 * time.Millisecond
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL, third.URL}, Tries: 3,
			Retry: &backendRetryConfig{
				Status:  []int{http.StatusServiceUnavailable},
				Backoff: &backendBackoffConfig{Base: base, Cap: capDelay},
			},
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
	if elapsed := time.Since(started); elapsed < base+capDelay-8*time.Millisecond {
		t.Fatalf("retry elapsed = %s, want at least %s", elapsed, base+capDelay-8*time.Millisecond)
	}
}

func TestBackendRetriesReplayableRequestBody(t *testing.T) {
	const wantBody = "request payload"
	var firstBody, secondBody string
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		data, _ := io.ReadAll(request.Body)
		firstBody = string(data)
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer first.Close()
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		data, _ := io.ReadAll(request.Body)
		secondBody = string(data)
		_, _ = io.WriteString(w, "healthy")
	}))
	defer second.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL}, Tries: 2,
			Retry: &backendRetryConfig{
				Status:  []int{http.StatusServiceUnavailable},
				Methods: []string{http.MethodPost},
				Body:    &backendBodyConfig{Max: 1024},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodPost, "http://proxy/", strings.NewReader(wantBody))
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK || response.Body.String() != "healthy" {
		t.Fatalf("response = %d %q", response.Code, response.Body.String())
	}
	if firstBody != wantBody || secondBody != wantBody {
		t.Fatalf("upstream bodies = %q and %q, want %q", firstBody, secondBody, wantBody)
	}
}

func TestBackendReplayBodyWithZeroContentLength(t *testing.T) {
	const wantBody = "zero-length framing body"
	var received string
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		_, _ = io.ReadAll(request.Body)
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer first.Close()
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		data, _ := io.ReadAll(request.Body)
		received = string(data)
		_, _ = io.WriteString(w, "healthy")
	}))
	defer second.Close()
	body := &countingBody{reader: strings.NewReader(wantBody)}
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL}, Tries: 2,
			Retry: &backendRetryConfig{
				Status:  []int{http.StatusServiceUnavailable},
				Methods: []string{http.MethodPost},
				Body:    &backendBodyConfig{Max: 1024},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodPost, "http://proxy/", nil)
	request.Body = body
	request.ContentLength = 0
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK || received != wantBody || body.closes.Load() != 1 {
		t.Fatalf("response = %d, body = %q, closes = %d", response.Code, received, body.closes.Load())
	}
}

func TestBackendRejectsReplayBodyAboveLimit(t *testing.T) {
	var hits atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		hits.Add(1)
	}))
	defer upstream.Close()
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{upstream.URL, closedBackendURL(t)}, Tries: 2,
			Retry: &backendRetryConfig{
				Status:  []int{http.StatusServiceUnavailable},
				Methods: []string{http.MethodPost},
				Body:    &backendBodyConfig{Max: 3},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "http://proxy/", strings.NewReader("payload")))
	if response.Code != http.StatusRequestEntityTooLarge || response.Body.String() != "Request Entity Too Large\n" {
		t.Fatalf("response = %d %q", response.Code, response.Body.String())
	}
	if response.Header().Get("Content-Type") != "text/plain; charset=utf-8" || response.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatalf("413 headers = %v", response.Header())
	}
	if hits.Load() != 0 {
		t.Fatalf("upstream hits = %d, want 0", hits.Load())
	}
}

func TestBackendRetryMethodsRemainExplicit(t *testing.T) {
	var secondHits atomic.Int32
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer first.Close()
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		secondHits.Add(1)
		_, _ = io.WriteString(w, "second")
	}))
	defer second.Close()
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL}, Tries: 2,
			Retry: &backendRetryConfig{
				Status:  []int{http.StatusServiceUnavailable},
				Methods: []string{http.MethodPost},
				Body:    &backendBodyConfig{Max: 1024},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusServiceUnavailable || secondHits.Load() != 0 {
		t.Fatalf("response = %d, second hits = %d", response.Code, secondHits.Load())
	}
}

func TestBackendRetryBudgetPreservesResponseWhenExhausted(t *testing.T) {
	var firstHits, secondHits, thirdHits atomic.Int32
	first := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		firstHits.Add(1)
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = io.WriteString(w, "first")
	}))
	defer first.Close()
	second := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		secondHits.Add(1)
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = io.WriteString(w, "second")
	}))
	defer second.Close()
	third := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		thirdHits.Add(1)
		_, _ = io.WriteString(w, "third")
	}))
	defer third.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first.URL, second.URL, third.URL}, Tries: 3,
			Retry: &backendRetryConfig{
				Status: []int{http.StatusServiceUnavailable},
				Budget: &backendBudgetConfig{Max: 1, Window: time.Hour},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusServiceUnavailable || response.Body.String() != "second" {
		t.Fatalf("response = %d %q", response.Code, response.Body.String())
	}
	if firstHits.Load() != 1 || secondHits.Load() != 1 || thirdHits.Load() != 0 {
		t.Fatalf("hits = %d, %d, %d", firstHits.Load(), secondHits.Load(), thirdHits.Load())
	}
}

func TestBackendRetryBudgetStopsTransportRetries(t *testing.T) {
	first := closedBackendURL(t)
	second := closedBackendURL(t)
	var thirdHits atomic.Int32
	third := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		thirdHits.Add(1)
	}))
	defer third.Close()
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {
			Targets: []string{first, second, third.URL}, Tries: 3,
			Retry: &backendRetryConfig{
				Status: []int{http.StatusServiceUnavailable},
				Budget: &backendBudgetConfig{Max: 1, Window: time.Hour},
			},
		}},
		Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if response.Code != http.StatusBadGateway || thirdHits.Load() != 0 {
		t.Fatalf("response = %d, third hits = %d", response.Code, thirdHits.Load())
	}
}

func TestRetryBudgetWindowAndContext(t *testing.T) {
	budget := &retryBudget{max: 1, window: 5 * time.Millisecond}
	if err := budget.reserve(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := budget.reserve(context.Background()); !errors.Is(err, errRetryBudgetExceeded) {
		t.Fatalf("second reservation error = %v", err)
	}
	time.Sleep(10 * time.Millisecond)
	if err := budget.reserve(context.Background()); err != nil {
		t.Fatalf("window reservation error = %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := budget.reserve(ctx); !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled reservation error = %v", err)
	}
}

func TestBackendRetryBackoffStopsWhenContextIsCanceled(t *testing.T) {
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
		Retry: &backendRetryConfig{Status: []int{http.StatusServiceUnavailable}, Backoff: &backendBackoffConfig{Base: time.Second, Cap: time.Second}},
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
			if got := retryableBackendRequest(test.request, nil, 0); got != test.want {
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
