package main

import (
	"net/http"
	"net/http/httptest"
	"net/netip"
	"testing"
)

func TestDecodeRouteClientIPMatch(t *testing.T) {
	data := `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/internal/*","match":{"ip":["192.0.2.99/24","2001:db8::/32"]}}]}]}`
	cfg, err := decodeConfig([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	prefixes := cfg.Sites[0].Routes[0].Match.IP
	if len(prefixes) != 2 || prefixes[0].String() != "192.0.2.0/24" || prefixes[1].String() != "2001:db8::/32" {
		t.Fatalf("decoded prefixes = %v", prefixes)
	}

	withMatch := func(match string) string {
		return `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":` + match + `}]}]}`
	}
	invalid := map[string]string{
		"null ip":               withMatch(`{"ip":null}`),
		"scalar ip":             withMatch(`{"ip":"192.0.2.0/24"}`),
		"empty ip":              withMatch(`{"ip":[]}`),
		"null item":             withMatch(`{"ip":[null]}`),
		"non-string item":       withMatch(`{"ip":[1]}`),
		"empty item":            withMatch(`{"ip":[""]}`),
		"invalid CIDR":          withMatch(`{"ip":["192.0.2.1"]}`),
		"zone":                  withMatch(`{"ip":["fe80::1%eth0/128"]}`),
		"duplicate normalized":  withMatch(`{"ip":["192.0.2.1/24","192.0.2.0/24"]}`),
		"unknown field":         withMatch(`{"clientIP":["192.0.2.0/24"]}`),
		"duplicate ip field":    withMatch(`{"ip":["192.0.2.0/24"],"ip":["192.0.2.0/24"]}`),
		"duplicate match field": `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":{"ip":["192.0.2.0/24"]},"match":{"ip":["192.0.2.0/24"]}}]}]}`,
		"empty match":           withMatch(`{}`),
	}
	for name, raw := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := routerFromJSON(raw); err == nil {
				t.Fatal("invalid client IP match was accepted")
			}
		})
	}
}

func TestRouteClientIPTrustChainAndFallback(t *testing.T) {
	fallback := newTextBackend(t, "fallback")
	matched := newTextBackend(t, "matched")
	defer fallback.Close()
	defer matched.Close()

	prefix := func(value string) netip.Prefix { return netip.MustParsePrefix(value) }
	router, err := newRouter(config{
		TrustProxies: []string{"10.0.0.0/8", "::ffff:10.0.0.0/104"},
		Sites: []siteConfig{{
			Hosts:  []string{"*"},
			Target: fallback.URL,
			Routes: []routeConfig{
				{Path: "/trusted/*", Target: matched.URL, Match: &routeMatchConfig{IP: []netip.Prefix{prefix("198.51.100.0/24")}}},
				{Path: "/untrusted/*", Target: matched.URL, Match: &routeMatchConfig{IP: []netip.Prefix{prefix("192.0.2.0/24")}}},
				{Path: "/repeat/*", Target: matched.URL, Match: &routeMatchConfig{IP: []netip.Prefix{prefix("198.51.100.0/24")}}},
			},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	request := func(path, remote string, xff ...string) string {
		req := httptest.NewRequest(http.MethodGet, "http://proxy"+path, nil)
		req.RemoteAddr = remote
		for _, value := range xff {
			req.Header.Add("X-Forwarded-For", value)
		}
		response := httptest.NewRecorder()
		router.ServeHTTP(response, req)
		return response.Body.String()
	}
	if got := request("/trusted/test", "10.0.0.1:1234", "198.51.100.10, 10.0.0.2"); got != "matched" {
		t.Fatalf("trusted chain response = %q", got)
	}
	if got := request("/untrusted/test", "192.0.2.10:1234", "198.51.100.10"); got != "matched" {
		t.Fatalf("untrusted peer response = %q", got)
	}
	if got := request("/trusted/test", "192.0.2.10:1234", "198.51.100.10"); got != "fallback" {
		t.Fatalf("spoofed forwarded response = %q", got)
	}
	if got := request("/repeat/test", "10.0.0.1:1234", "198.51.100.10", "10.0.0.2"); got != "matched" {
		t.Fatalf("repeated header response = %q", got)
	}
	if got := request("/trusted/test", "10.0.0.1:1234", "198.51.100.10, bad"); got != "fallback" {
		t.Fatalf("malformed forwarded response = %q", got)
	}
	if got := request("/trusted/test", "10.0.0.1:1234", "10.0.0.2"); got != "fallback" {
		t.Fatalf("all-trusted response = %q", got)
	}
}

func TestRouteClientIPMatchesMappedAddresses(t *testing.T) {
	matched := newTextBackend(t, "matched")
	fallback := newTextBackend(t, "fallback")
	defer matched.Close()
	defer fallback.Close()
	router, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: fallback.URL,
		Routes: []routeConfig{{Path: "/mapped/*", Target: matched.URL, Match: &routeMatchConfig{IP: []netip.Prefix{netip.MustParsePrefix("::ffff:192.0.2.0/120")}}}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	req := httptest.NewRequest(http.MethodGet, "http://proxy/mapped/test", nil)
	req.RemoteAddr = "[::ffff:192.0.2.10]:1234"
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if got := response.Body.String(); got != "matched" {
		t.Fatalf("mapped response = %q", got)
	}
}
