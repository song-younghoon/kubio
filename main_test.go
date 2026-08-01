package main

import (
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func newTextBackend(t *testing.T, text string) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(text))
	}))
}

func proxyResponse(t *testing.T, handler http.Handler, host, path string) string {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "http://proxy"+path, nil)
	req.Host = host
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	return res.Body.String()
}

func TestDecodeConfigIsStrict(t *testing.T) {
	valid := `{
  "listen": ":8080",
  "sites": [{
    "hosts": ["*"],
    "target": "http://localhost:3000",
    "routes": [{"path": "/api/*", "strip": true}]
  }]
}`
	if _, err := decodeConfig([]byte(valid)); err != nil {
		t.Fatalf("valid config rejected: %v", err)
	}

	tests := map[string]string{
		"unknown field":        `{"listen":":8080","sites":[],"extra":true}`,
		"unknown nested field": `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","extra":true}]}`,
		"case changed field":   `{"Listen":":8080","sites":[]}`,
		"duplicate key":        `{"listen":":8080","listen":":9090","sites":[]}`,
		"duplicate nested key": `{"listen":":8080","sites":[{"hosts":["*"],"hosts":["example.com"],"target":"http://localhost:3000"}]}`,
		"trailing comma":       `{"listen":":8080","sites":[],}`,
		"null trust proxies":   `{"listen":":8080","trustProxies":null,"sites":[]}`,
		"null routes":          `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":null}]}`,
		"null route target":    `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/","target":null}]}]}`,
		"null strip":           `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/","strip":null}]}]}`,
	}
	for name, data := range tests {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(data)); err == nil {
				t.Fatal("invalid config was accepted")
			}
		})
	}
}

func TestRouterChoosesMostSpecificSiteAndRoute(t *testing.T) {
	star := newTextBackend(t, "star")
	wildcard := newTextBackend(t, "wildcard")
	longWildcard := newTextBackend(t, "long-wildcard")
	exact := newTextBackend(t, "exact")
	defer star.Close()
	defer wildcard.Close()
	defer longWildcard.Close()
	defer exact.Close()

	handler, err := newRouter(config{Sites: []siteConfig{
		{Hosts: []string{"*"}, Target: star.URL},
		{Hosts: []string{"*.example.com"}, Target: wildcard.URL},
		{Hosts: []string{"*.api.example.com"}, Target: longWildcard.URL},
		{Hosts: []string{"example.com"}, Target: exact.URL},
	}})
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		host string
		want string
	}{
		{host: "other.test", want: "star"},
		{host: "api.example.com", want: "wildcard"},
		{host: "v1.api.example.com", want: "long-wildcard"},
		{host: "example.com.:8080", want: "exact"},
	} {
		if got := proxyResponse(t, handler, test.host, "/"); got != test.want {
			t.Errorf("host %q = %q, want %q", test.host, got, test.want)
		}
	}

	allRoutes := newTextBackend(t, "all")
	api := newTextBackend(t, "api")
	admin := newTextBackend(t, "admin")
	exactPath := newTextBackend(t, "exact-path")
	tieFirst := newTextBackend(t, "tie-first")
	tieLast := newTextBackend(t, "tie-last")
	defer allRoutes.Close()
	defer api.Close()
	defer admin.Close()
	defer exactPath.Close()
	defer tieFirst.Close()
	defer tieLast.Close()

	routes, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: star.URL,
		Routes: []routeConfig{
			{Path: "/*", Target: allRoutes.URL},
			{Path: "/api/*", Target: api.URL},
			{Path: "/api/admin/*", Target: admin.URL},
			{Path: "/api/admin", Target: exactPath.URL},
			{Path: "/tie/*", Target: tieFirst.URL},
			{Path: "/tie/*", Target: tieLast.URL},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		path string
		want string
	}{
		{path: "/", want: "all"},
		{path: "/api", want: "api"},
		{path: "/api/users", want: "api"},
		{path: "/api/admin", want: "exact-path"},
		{path: "/api/admin/users", want: "admin"},
		{path: "/tie/value", want: "tie-last"},
		{path: "/apix", want: "all"},
	} {
		if got := proxyResponse(t, routes, "anything", test.path); got != test.want {
			t.Errorf("path %q = %q, want %q", test.path, got, test.want)
		}
	}
}

func TestRouterReturnsNotFoundForUnknownHost(t *testing.T) {
	backend := newTextBackend(t, "backend")
	defer backend.Close()
	handler, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: backend.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if res.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want %d", res.Code, http.StatusNotFound)
	}
}

func TestProxyAppliesHeadersAndForwardingRules(t *testing.T) {
	type observation struct {
		path     string
		escaped  string
		query    string
		host     string
		site     string
		empty    bool
		client   string
		override string
		route    string
		xff      string
		xfp      string
		xfh      string
	}
	var got observation
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		_, empty := req.Header["X-Empty"]
		got = observation{
			path:     req.URL.Path,
			escaped:  req.URL.EscapedPath(),
			query:    req.URL.RawQuery,
			host:     req.Host,
			site:     req.Header.Get("X-Site"),
			empty:    empty,
			client:   req.Header.Get("X-Client"),
			override: req.Header.Get("X-Override"),
			route:    req.Header.Get("X-Route"),
			xff:      req.Header.Get("X-Forwarded-For"),
			xfp:      req.Header.Get("X-Forwarded-Proto"),
			xfh:      req.Header.Get("X-Forwarded-Host"),
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()

	handler, err := newRouter(config{
		TrustProxies: []string{"192.0.2.0/24"},
		Sites: []siteConfig{{
			Hosts:  []string{"*"},
			Target: backend.URL,
			Headers: map[string]string{
				"X-Site":     "site",
				"X-Empty":    "",
				"X-Override": "site",
				"Host":       "configured.example",
			},
			Routes: []routeConfig{{
				Path: "/api/*",
				Headers: map[string]string{
					"X-Override": "route",
					"X-Route":    "yes",
				},
				Strip: true,
			}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequest(http.MethodGet, "http://proxy/api/users?a=1;b=2", nil)
	req.Host = "app.example:8080"
	req.RemoteAddr = "198.51.100.7:1234"
	req.Header.Set("X-Site", "client")
	req.Header.Set("X-Client", "preserve")
	req.Header.Set("X-Override", "client")
	req.Header.Set("X-Forwarded-For", "spoofed")
	req.Header.Set("X-Forwarded-Proto", "https")
	req.Header.Set("X-Forwarded-Host", "spoofed.example")
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	if res.Code != http.StatusNoContent {
		t.Fatalf("untrusted status = %d, want %d", res.Code, http.StatusNoContent)
	}
	if got.path != "/users" || got.query != "a=1;b=2" {
		t.Fatalf("untrusted request = %q?%q, want %q?%q", got.path, got.query, "/users", "a=1;b=2")
	}
	if got.host != "configured.example" || got.site != "site" || !got.empty || got.client != "preserve" || got.override != "route" || got.route != "yes" {
		t.Fatalf("configured headers = %#v", got)
	}
	if got.xff != "198.51.100.7" || got.xfp != "http" || got.xfh != "app.example:8080" {
		t.Fatalf("untrusted forwarded headers = %#v", got)
	}

	req = httptest.NewRequest(http.MethodGet, "http://proxy/other?x=1", nil)
	req.Host = "app.example:8080"
	req.RemoteAddr = "192.0.2.7:1234"
	req.Header.Set("X-Forwarded-For", "1.2.3.4")
	req.Header.Set("X-Forwarded-Proto", "https")
	req.Header.Set("X-Forwarded-Host", "public.example")
	res = httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	if res.Code != http.StatusNoContent {
		t.Fatalf("trusted status = %d, want %d", res.Code, http.StatusNoContent)
	}
	if got.xff != "1.2.3.4, 192.0.2.7" || got.xfp != "https" || got.xfh != "public.example" {
		t.Fatalf("trusted forwarded headers = %#v", got)
	}

	req = httptest.NewRequest(http.MethodGet, "http://proxy/api/a%2Fb", nil)
	req.Host = "app.example:8080"
	req.RemoteAddr = "198.51.100.7:1234"
	res = httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	if res.Code != http.StatusNoContent {
		t.Fatalf("escaped path status = %d, want %d", res.Code, http.StatusNoContent)
	}
	if got.path != "/a/b" || got.escaped != "/a%2Fb" {
		t.Fatalf("escaped path = %q (%q), want %q (%q)", got.path, got.escaped, "/a/b", "/a%2Fb")
	}
}

func TestHeaderAndTargetValidation(t *testing.T) {
	t.Setenv("KUBIO_EMPTY", "")
	if headers, err := resolveHeaders(map[string]string{"X-Empty": "${KUBIO_EMPTY}"}); err != nil || headers["X-Empty"] != "" {
		t.Fatalf("empty environment value: headers=%v err=%v", headers, err)
	}

	for name, headers := range map[string]map[string]string{
		"duplicate case": {"X-Test": "a", "x-test": "b"},
		"managed":        {"Connection": "close"},
		"managed te":     {"TE": "trailers"},
		"bad name":       {"X Test": "value"},
		"bad value":      {"X-Test": "line\nfeed"},
		"unset env":      {"X-Test": "${KUBIO_NOT_SET}"},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := resolveHeaders(headers); err == nil {
				t.Fatal("invalid headers were accepted")
			}
		})
	}

	for _, target := range []string{
		"ftp://localhost:3000",
		"http://localhost:3000/path",
		"http://localhost:3000?query=1",
		"http://user:pass@localhost:3000",
		"http://localhost:65536",
		"http://localhost:bad",
	} {
		if _, err := parseTarget(target); err == nil {
			t.Errorf("parseTarget(%q) accepted invalid target", target)
		}
	}
	for _, target := range []string{"http://localhost:3000", "https://[::1]:8443"} {
		if _, err := parseTarget(target); err != nil {
			t.Errorf("parseTarget(%q): %v", target, err)
		}
	}
	for _, host := range []string{"example.com:bad", "example.com:65536", "[::1]:bad"} {
		if _, err := newHostPattern(host); err == nil {
			t.Errorf("newHostPattern(%q) accepted invalid port", host)
		}
	}
}

func TestProxyErrorHandlerHidesUpstreamDetails(t *testing.T) {
	for _, test := range []struct {
		name   string
		err    error
		status int
	}{
		{name: "timeout", err: &net.DNSError{IsTimeout: true}, status: http.StatusGatewayTimeout},
		{name: "bad gateway", err: errors.New("upstream secret details"), status: http.StatusBadGateway},
	} {
		t.Run(test.name, func(t *testing.T) {
			res := httptest.NewRecorder()
			proxyErrorHandler(res, httptest.NewRequest(http.MethodGet, "http://proxy/", nil), test.err)
			if res.Code != test.status {
				t.Fatalf("status = %d, want %d", res.Code, test.status)
			}
			if strings.Contains(res.Body.String(), "upstream") || strings.Contains(res.Body.String(), "secret") {
				t.Fatalf("upstream details leaked: %q", res.Body.String())
			}
		})
	}
}

func TestProxyPassesUpstreamResponse(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("X-Backend", "ok")
		w.WriteHeader(http.StatusTeapot)
		_, _ = w.Write([]byte("backend"))
	}))
	defer backend.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: backend.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if res.Code != http.StatusTeapot || res.Header().Get("X-Backend") != "ok" || res.Body.String() != "backend" {
		t.Fatalf("response = %d, headers=%v, body=%q", res.Code, res.Header(), res.Body.String())
	}
}

func TestRouterMatchesWildcardHostAndStripsPathPrefix(t *testing.T) {
	var gotPath string
	var gotHost string
	var gotHeaders map[string]string
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		gotPath = req.URL.Path
		gotHost = req.Host
		gotHeaders = map[string]string{
			"Authorization": req.Header.Get("Authorization"),
			"X-Proxy":       req.Header.Get("X-Proxy"),
			"X-Route":       req.Header.Get("X-Route"),
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()
	t.Setenv("UPSTREAM_TOKEN", "route-token")

	handler, err := newRouter(config{
		Sites: []siteConfig{
			{
				Hosts:  []string{"*.example.com"},
				Target: backend.URL,
				Headers: map[string]string{
					"Authorization": "site-token",
					"X-Proxy":       "kubio",
				},
				Routes: []routeConfig{{
					Path:   "/api/*",
					Target: backend.URL,
					Headers: map[string]string{
						"Authorization": "Bearer ${UPSTREAM_TOKEN}",
						"X-Route":       "route",
					},
					Strip: true,
				}},
			},
		},
	})
	if err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequest(http.MethodGet, "http://proxy/api/users", nil)
	req.Host = "api.example.com:8080"
	req.Header.Set("X-Proxy", "client")
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)

	if res.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want %d", res.Code, http.StatusNoContent)
	}
	if gotPath != "/users" {
		t.Fatalf("backend path = %q, want %q", gotPath, "/users")
	}
	if gotHost != "api.example.com:8080" {
		t.Fatalf("backend host = %q, want %q", gotHost, "api.example.com:8080")
	}
	if gotHeaders["Authorization"] != "Bearer route-token" {
		t.Fatalf("authorization = %q, want %q", gotHeaders["Authorization"], "Bearer route-token")
	}
	if gotHeaders["X-Proxy"] != "kubio" {
		t.Fatalf("site header = %q, want %q", gotHeaders["X-Proxy"], "kubio")
	}
	if gotHeaders["X-Route"] != "route" {
		t.Fatalf("route header = %q, want %q", gotHeaders["X-Route"], "route")
	}

	if matchesHost("v1.api.example.com", []string{"*.example.com"}) {
		t.Fatal("nested subdomain unexpectedly matched")
	}
	if matchesHost("example.com", []string{"*.example.com"}) {
		t.Fatal("apex domain unexpectedly matched")
	}
}

func TestExpandEnvUsesShellEscaping(t *testing.T) {
	t.Setenv("KUBIO_TEST_TOKEN", "secret")

	tests := []struct {
		input string
		want  string
	}{
		{input: `${KUBIO_TEST_TOKEN}`, want: `secret`},
		{input: `\${KUBIO_TEST_TOKEN}`, want: `${KUBIO_TEST_TOKEN}`},
		{input: `\\${KUBIO_TEST_TOKEN}`, want: `\secret`},
	}

	for _, tt := range tests {
		got, err := expandEnv(tt.input)
		if err != nil {
			t.Fatalf("expandEnv(%q): %v", tt.input, err)
		}
		if got != tt.want {
			t.Errorf("expandEnv(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}

func TestReloadableRouterSwitchesConfiguration(t *testing.T) {
	backendA := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		_, _ = w.Write([]byte("a"))
	}))
	defer backendA.Close()
	backendB := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		_, _ = w.Write([]byte("b"))
	}))
	defer backendB.Close()

	first, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: backendA.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	second, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: backendB.URL}}})
	if err != nil {
		t.Fatal(err)
	}

	handler := newReloadableRouter(first)
	request := func() string {
		req := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, req)
		return res.Body.String()
	}

	if got := request(); got != "a" {
		t.Fatalf("initial response = %q, want %q", got, "a")
	}
	handler.Store(second)
	if got := request(); got != "b" {
		t.Fatalf("reloaded response = %q, want %q", got, "b")
	}
}
