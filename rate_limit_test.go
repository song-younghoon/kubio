package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

func TestDecodeLimitConfiguration(t *testing.T) {
	base := `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`
	withLimit := func(limit string) string {
		return `{"listen":":8080","limit":` + limit + `,"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`
	}
	valid := `{"listen":":8080","limit":{"rate":1000,"burst":2000},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`
	cfg, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Limit == nil || cfg.Limit.Rate != 1000 || cfg.Limit.Burst != 2000 {
		t.Fatalf("limit = %#v", cfg.Limit)
	}
	cfg, err = decodeConfig([]byte(base))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Limit != nil {
		t.Fatalf("absent limit = %#v, want nil", cfg.Limit)
	}

	invalid := map[string]string{
		"null":             `null`,
		"array":            `[]`,
		"string":           `"limit"`,
		"empty":            `{}`,
		"missing rate":     `{"burst":1}`,
		"missing burst":    `{"rate":1}`,
		"null rate":        `{"rate":null,"burst":1}`,
		"decimal rate":     `{"rate":1.0,"burst":1}`,
		"exponent burst":   `{"rate":1,"burst":1e0}`,
		"zero rate":        `{"rate":0,"burst":1}`,
		"large burst":      `{"rate":1,"burst":1000001}`,
		"unknown field":    `{"rate":1,"burst":1,"extra":true}`,
		"duplicate field":  `{"rate":1,"rate":2,"burst":1}`,
		"root alias":       `{"rateLimit":{"rate":1,"burst":1}}`,
		"nested placement": `{"sites":[{"hosts":["*"],"target":"http://localhost:3000","limit":{"rate":1,"burst":1}}]}`,
	}
	for name, limit := range invalid {
		t.Run(name, func(t *testing.T) {
			data := withLimit(limit)
			if name == "root alias" || name == "nested placement" {
				data = limit
			}
			if _, err := decodeConfig([]byte(data)); err == nil {
				t.Fatal("invalid limit configuration was accepted")
			}
		})
	}
}

func TestRateLimiterRetainsNanotokensAndHandlesClockRollback(t *testing.T) {
	start := time.Unix(0, 0)
	limiter := newRateLimiterAt(&limitConfig{Rate: 2, Burst: 2}, start)
	if !limiter.allowAt(start) || !limiter.allowAt(start) {
		t.Fatal("full bucket did not admit its burst")
	}
	if limiter.allowAt(start) {
		t.Fatal("empty bucket admitted a request")
	}
	if limiter.allowAt(start.Add(499 * time.Millisecond)) {
		t.Fatal("partial token admitted a request")
	}
	if !limiter.allowAt(start.Add(999 * time.Millisecond)) {
		t.Fatal("exact token boundary was rejected")
	}
	if limiter.allowAt(start.Add(500 * time.Millisecond)) {
		t.Fatal("clock rollback changed the balance")
	}
	if saturatingMultiply(maxInt64, 2) != maxInt64 || saturatingAdd(maxInt64, 1) != maxInt64 {
		t.Fatal("overflow did not saturate")
	}
}

func TestRateLimitRejectsBeforeRouteAndLogsEmpty429(t *testing.T) {
	var hits atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		hits.Add(1)
		_, _ = w.Write([]byte("upstream"))
	}))
	defer upstream.Close()

	router, err := newRouter(config{
		Log:       true,
		RequestID: true,
		Limit:     &limitConfig{Rate: 1, Burst: 1},
		Sites: []siteConfig{{
			Hosts:  []string{"example.com"},
			Target: upstream.URL,
			ResponseHeaders: responseHeaderPolicy{
				Set: map[string][]string{"X-Configured": {"must-not-appear"}},
			},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	if !router.limiter.allow() {
		t.Fatal("failed to consume initial token")
	}
	var output bytes.Buffer
	router.accessLogger = newAccessLogger(&output)

	request := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
	request.Header.Set(requestIDHeader, "limited-id")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)

	if response.Code != http.StatusTooManyRequests || response.Body.Len() != 0 {
		t.Fatalf("response = %d %q; want empty 429", response.Code, response.Body.String())
	}
	if response.Header().Get(requestIDHeader) != "limited-id" {
		t.Fatalf("response request ID = %q", response.Header().Get(requestIDHeader))
	}
	if response.Header().Get("Retry-After") != "" || response.Header().Get("X-Configured") != "" {
		t.Fatalf("limiter response headers = %v", response.Header())
	}
	if hits.Load() != 0 {
		t.Fatalf("upstream hits = %d; want 0", hits.Load())
	}
	var record accessRecordWithID
	if err := json.Unmarshal(output.Bytes(), &record); err != nil {
		t.Fatalf("access log = %q: %v", output.String(), err)
	}
	if record.Status != http.StatusTooManyRequests || record.Bytes != 0 || record.ID != "limited-id" {
		t.Fatalf("access record = %#v", record)
	}
}

func TestRateLimitRejectsOptionsBeforeBodyDiscard(t *testing.T) {
	router, err := newRouter(config{
		Limit: &limitConfig{Rate: 1, Burst: 1},
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	if !router.limiter.allow() {
		t.Fatal("failed to consume initial token")
	}
	body := &limiterCountingBody{}
	request := httptest.NewRequest(http.MethodOptions, "http://example.com/", io.NopCloser(body))
	request.RequestURI = "*"
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusTooManyRequests || body.reads != 0 {
		t.Fatalf("response = %d, body reads = %d; want 429 and no reads", response.Code, body.reads)
	}
}

func TestRateLimiterIsSharedAcrossReloadedGenerations(t *testing.T) {
	first, err := newRouter(config{
		Limit: &limitConfig{Rate: 1, Burst: 1},
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer first.close()
	second, err := newRouter(config{
		Limit: &limitConfig{Rate: 10, Burst: 2},
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer second.close()
	reloadable := newReloadableRouter(first, nil)
	reloadable.StoreGeneration(second, nil)
	if reloadable.current.Load().router.limiter != first.limiter {
		t.Fatal("reload did not retain the process limiter")
	}
	if !first.limiter.allow() || !first.limiter.allow() {
		t.Fatal("reloaded limiter did not start with the new full bucket")
	}
	if first.limiter.allow() {
		t.Fatal("reloaded limiter exceeded the new burst")
	}
}

func TestRateLimiterSharesHandleAcrossEnableTransitions(t *testing.T) {
	disabled, err := newRouter(config{
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer disabled.close()
	enabled, err := newRouter(config{
		Limit: &limitConfig{Rate: 1, Burst: 1},
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer enabled.close()
	reloadable := newReloadableRouter(disabled, nil)
	reloadable.StoreGeneration(enabled, nil)
	current := reloadable.current.Load().router
	if current.limiter != disabled.limiter || current.limit == nil {
		t.Fatal("enable transition did not retain the shared limiter handle")
	}
	if !current.limiter.allow() || current.limiter.allow() {
		t.Fatal("enable transition did not publish a fresh bucket")
	}

	disabledAgain, err := newRouter(config{
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer disabledAgain.close()
	reloadable.StoreGeneration(disabledAgain, nil)
	current = reloadable.current.Load().router
	if current.limiter != disabled.limiter || current.limit != nil {
		t.Fatal("disable transition did not retain the shared limiter handle")
	}
	if !current.limiter.allow() {
		t.Fatal("disabled transition retained rate limiting")
	}
}

func TestOldDisabledRouterUsesPublishedLimiter(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("ok"))
	}))
	defer upstream.Close()
	oldRouter, err := newRouter(config{
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: upstream.URL}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer oldRouter.close()
	newRouter, err := newRouter(config{
		Limit: &limitConfig{Rate: 1, Burst: 1},
		Sites: []siteConfig{{Hosts: []string{"*"}, Target: upstream.URL}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer newRouter.close()
	reloadable := newReloadableRouter(oldRouter, nil)
	reloadable.StoreGeneration(newRouter, nil)
	if !oldRouter.limiter.allow() {
		t.Fatal("failed to consume published token")
	}
	response := httptest.NewRecorder()
	oldRouter.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "http://example.com/", nil))
	if response.Code != http.StatusTooManyRequests || response.Body.Len() != 0 {
		t.Fatalf("old router response = %d %q; want empty 429", response.Code, response.Body.String())
	}
}

type limiterCountingBody struct {
	reads int
}

func (b *limiterCountingBody) Read([]byte) (int, error) {
	b.reads++
	return 0, io.EOF
}
