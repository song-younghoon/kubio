package main

import (
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func routerFromJSON(data string) (*router, error) {
	cfg, err := decodeConfig([]byte(data))
	if err != nil {
		return nil, err
	}
	return newRouter(cfg)
}

func TestDecodeRouteMatch(t *testing.T) {
	t.Setenv("KUBIO_MATCH", "expanded")
	valid := `{
  "listen": ":8080",
  "sites": [{
    "hosts": ["*"],
    "target": "http://localhost:3000",
    "headers": {"X-Injected": "yes"},
    "routes": [{
      "path": "/*",
      "match": {
        "header": {
          "X-Environment": ["production", "staging"],
          "X-Literal": "${KUBIO_MATCH}",
          "X-Empty": ""
        },
        "query": {
          "flag": "",
          "mode": ["one", "two"],
          "이름": "값"
        }
      }
    }]
  }]
}`
	cfg, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	match := cfg.Sites[0].Routes[0].Match
	if match == nil || len(match.Header) != 3 || len(match.Query) != 3 {
		t.Fatalf("match = %#v", match)
	}
	if got := match.Header["X-Literal"]; len(got) != 1 || got[0] != "${KUBIO_MATCH}" {
		t.Fatalf("literal value = %q", got)
	}
	if _, err := newRouter(cfg); err != nil {
		t.Fatalf("valid match rejected: %v", err)
	}

	withMatch := func(match string) string {
		return `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":` + match + `}]}]}`
	}
	invalid := map[string]string{
		"null match":            withMatch(`null`),
		"scalar match":          withMatch(`true`),
		"array match":           withMatch(`[]`),
		"empty match":           withMatch(`{}`),
		"unknown match field":   withMatch(`{"extra":{"x":"y"}}`),
		"null header":           withMatch(`{"header":null}`),
		"scalar header":         withMatch(`{"header":"x"}`),
		"array header":          withMatch(`{"header":[]}`),
		"empty header":          withMatch(`{"header":{}}`),
		"null query":            withMatch(`{"query":null}`),
		"scalar query":          withMatch(`{"query":1}`),
		"array query":           withMatch(`{"query":[]}`),
		"empty query":           withMatch(`{"query":{}}`),
		"null value":            withMatch(`{"query":{"x":null}}`),
		"numeric value":         withMatch(`{"query":{"x":1}}`),
		"object value":          withMatch(`{"query":{"x":{}}}`),
		"empty alternatives":    withMatch(`{"query":{"x":[]}}`),
		"null alternative":      withMatch(`{"query":{"x":[null]}}`),
		"numeric alternative":   withMatch(`{"query":{"x":[1]}}`),
		"duplicate alternative": withMatch(`{"query":{"x":["a","a"]}}`),
		"empty header name":     withMatch(`{"header":{"":"x"}}`),
		"invalid header name":   withMatch(`{"header":{"Bad Header":"x"}}`),
		"duplicate header case": withMatch(`{"header":{"X-Test":"x","x-test":"y"}}`),
		"invalid header value":  withMatch(`{"header":{"X-Test":"bad\rvalue"}}`),
		"empty query name":      withMatch(`{"query":{"":"x"}}`),
		"duplicate match":       `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":{"query":{"x":"a"}},"match":{"query":{"x":"b"}}}]}]}`,
		"duplicate property":    withMatch(`{"query":{"x":"a","x":"b"}}`),
		"plural alias":          withMatch(`{"headers":{"X-Test":"x"}}`),
		"route alias":           `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","matches":{"query":{"x":"a"}}}]}]}`,
		"site placement":        `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","match":{"query":{"x":"a"}}}]}`,
		"backend placement":     `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"match":{"query":{"x":"a"}}}},"sites":[{"hosts":["*"],"backend":"app"}]}`,
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := routerFromJSON(data); err == nil {
				t.Fatal("invalid match was accepted")
			}
		})
	}
}

func TestRouteMatchErrorsHideAlternatives(t *testing.T) {
	for _, data := range []string{
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":{"query":{"token":["top-secret","top-secret"]}}}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":{"header":{"Authorization":"top-secret\r"}}}]}]}`,
	} {
		_, err := routerFromJSON(data)
		if err == nil {
			t.Fatal("invalid match was accepted")
		}
		if strings.Contains(err.Error(), "top-secret") {
			t.Fatalf("alternative leaked in error: %v", err)
		}
	}
}

func TestRouteMatchesHeadersAndQuery(t *testing.T) {
	backend := func(name string) *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			_, _ = fmt.Fprintf(w, "%s|%s", name, req.URL.RawQuery)
		}))
	}
	fallback := backend("fallback")
	matched := backend("matched")
	defer fallback.Close()
	defer matched.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:   []string{"example.com"},
		Target:  fallback.URL,
		Headers: map[string]string{"X-Injected": "yes"},
		Routes: []routeConfig{
			{
				Path:   "/checkout/*",
				Target: matched.URL,
				Match: &routeMatchConfig{
					Header: map[string][]string{"x-environment": {"production", "staging"}, "X-Empty": {""}},
					Query:  map[string][]string{"preview": {"1", "true"}, "flag": {""}},
				},
			},
			{Path: "/inject/*", Target: matched.URL, Match: &routeMatchConfig{Header: map[string][]string{"X-Injected": {"yes"}}}},
			{Path: "/host/*", Target: matched.URL, Match: &routeMatchConfig{Header: map[string][]string{"HOST": {"Example.COM.:8443"}}}},
			{Path: "/length/*", Target: matched.URL, Match: &routeMatchConfig{Header: map[string][]string{"Content-Length": {"3"}}}},
			{Path: "/plus/*", Target: matched.URL, Match: &routeMatchConfig{Query: map[string][]string{"a+b": {"yes"}}}},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}

	request := func(path string, mutate func(*http.Request)) string {
		req := httptest.NewRequest(http.MethodGet, "http://proxy"+path, nil)
		req.Host = "example.com"
		if mutate != nil {
			mutate(req)
		}
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, req)
		return res.Body.String()
	}
	success := func(req *http.Request) {
		req.Header.Add("X-Environment", "development")
		req.Header.Add("X-Environment", "production")
		req.Header.Set("X-Empty", "")
	}
	if got := request("/checkout/pay?%70review=true&flag", success); got != "matched|%70review=true&flag" {
		t.Fatalf("matched response = %q", got)
	}
	if got := request("/checkout/pay?preview=true", success); got != "fallback|preview=true" {
		t.Fatalf("missing query property = %q", got)
	}
	if got := request("/inject/test", nil); got != "fallback|" {
		t.Fatalf("injected header affected matching: %q", got)
	}
	if got := request("/inject/test", func(req *http.Request) { req.Header.Set("X-Injected", "yes") }); got != "matched|" {
		t.Fatalf("received header did not match: %q", got)
	}
	if got := request("/inject/test", func(req *http.Request) { req.Header["x-injected"] = []string{"yes"} }); got != "matched|" {
		t.Fatalf("non-canonical header did not match: %q", got)
	}
	if got := request("/inject/test", func(req *http.Request) {
		req.Header["X-Injected"] = []string{"no"}
		req.Header["x-injected"] = []string{"yes"}
	}); got != "matched|" {
		t.Fatalf("case-variant header values did not match: %q", got)
	}
	if got := request("/host/test", func(req *http.Request) { req.Host = "Example.COM.:8443" }); got != "matched|" {
		t.Fatalf("raw Host did not match: %q", got)
	}
	if got := request("/length/test", func(req *http.Request) {
		req.Body = io.NopCloser(strings.NewReader("abc"))
		req.ContentLength = 3
	}); got != "fallback|" {
		t.Fatalf("ContentLength was synthesized: %q", got)
	}
	if got := request("/plus/test?a%2Bb=yes", nil); got != "matched|a%2Bb=yes" {
		t.Fatalf("encoded plus name = %q", got)
	}
	if got := request("/plus/test?a+b=yes", nil); got != "fallback|a+b=yes" {
		t.Fatalf("raw plus name = %q", got)
	}
}

func TestMalformedQueryOnlyRejectsQueryRoutes(t *testing.T) {
	routes := []route{
		{pattern: pathPattern{path: "/query", wildcard: true, depth: 1}, match: compileRouteMatch(routeMatchConfig{Query: map[string][]string{"a": {"1"}}})},
		{pattern: pathPattern{path: "/header", wildcard: true, depth: 1}, match: compileRouteMatch(routeMatchConfig{Header: map[string][]string{"X-Test": {"yes"}}})},
	}
	selected := site{routes: routes}
	query := httptest.NewRequest(http.MethodGet, "http://proxy/query/path?a=1;b=2", nil)
	if got := selected.selectRoute(query); got.route != nil {
		t.Fatalf("malformed query selected route %d", got.index)
	}
	header := httptest.NewRequest(http.MethodGet, "http://proxy/header/path?a=1;b=2", nil)
	header.Header.Set("X-Test", "yes")
	if got := selected.selectRoute(header); got.route == nil || got.index != 1 {
		t.Fatalf("header-only route = %d", got.index)
	}
}

func TestRouteMatchPriority(t *testing.T) {
	pattern := pathPattern{path: "/api", wildcard: true, depth: 1}
	request := func() *http.Request {
		req := httptest.NewRequest(http.MethodGet, "http://proxy/api/users?q=1", nil)
		req.Header.Set("X-Test", "a")
		return req
	}
	match := func(header, query map[string][]string) routeMatch {
		return compileRouteMatch(routeMatchConfig{Header: header, Query: query})
	}
	for _, test := range []struct {
		name   string
		routes []route
		want   int
	}{
		{
			name: "method preserves prior priority",
			routes: []route{
				{pattern: pattern},
				{pattern: pattern, methods: []string{http.MethodGet}},
			},
			want: 1,
		},
		{
			name: "condition kinds tie by declaration",
			routes: []route{
				{pattern: pattern, methods: []string{http.MethodGet}},
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, nil)},
			},
			want: 1,
		},
		{
			name: "more conditions",
			routes: []route{
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, nil)},
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, map[string][]string{"q": {"1"}})},
			},
			want: 1,
		},
		{
			name: "fewer alternatives",
			routes: []route{
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, map[string][]string{"q": {"1"}})},
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a", "b"}}, map[string][]string{"q": {"1"}})},
			},
			want: 0,
		},
		{
			name: "later complete tie",
			routes: []route{
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, nil)},
				{pattern: pattern, match: match(map[string][]string{"X-Test": {"a"}}, nil)},
			},
			want: 1,
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			got := (&site{routes: test.routes}).selectRoute(request())
			if got.route == nil || got.index != test.want {
				t.Fatalf("selected route %d, want %d", got.index, test.want)
			}
		})
	}
}

func TestIndexedRouteMatchSelectionMatchesLinear(t *testing.T) {
	patterns := []pathPattern{
		{wildcard: true},
		{path: "/api", wildcard: true, depth: 1},
		{path: "/api", wildcard: true, depth: 1},
		{path: "/api/admin", wildcard: true, depth: 2},
		{path: "/health", depth: 1},
		{path: "/health", depth: 1},
		{path: "/unused-a", depth: 1},
		{path: "/unused-b", depth: 1},
		{path: "/unused-c", depth: 1},
		{path: "/unused-d", depth: 1},
	}
	routes := make([]route, len(patterns))
	for index, pattern := range patterns {
		routes[index].pattern = pattern
	}
	routes[1].match = compileRouteMatch(routeMatchConfig{Header: map[string][]string{"X-Test": {"yes"}}})
	routes[2].match = compileRouteMatch(routeMatchConfig{Query: map[string][]string{"q": {"1"}}})
	routes[3].methods = []string{http.MethodPost}
	routes[4].match = compileRouteMatch(routeMatchConfig{Query: map[string][]string{"ready": {""}}})
	routes[5].match = compileRouteMatch(routeMatchConfig{Header: map[string][]string{"Host": {"proxy"}}})
	linear := site{routes: routes}
	indexed := site{routes: routes}
	indexed.buildRouteIndex()

	for _, target := range []string{
		"http://proxy/api/users?q=1",
		"http://proxy/api/users?q=2",
		"http://proxy/api/admin/users?q=1",
		"http://proxy/health?ready",
		"http://proxy/health?bad=%ZZ",
		"http://proxy/other",
	} {
		req := httptest.NewRequest(http.MethodGet, target, nil)
		req.Header.Set("X-Test", "yes")
		want := linear.selectRoute(req)
		got := indexed.selectRoute(req)
		if (got.route == nil) != (want.route == nil) || got.route != nil && (got.index != want.index || got.prefix != want.prefix) {
			t.Errorf("%s selected (%d, %q), want (%d, %q)", target, got.index, got.prefix, want.index, want.prefix)
		}
	}
}

func TestIneligibleRouteMatchDoesNotConsumeBackend(t *testing.T) {
	a := newTextBackend(t, "a")
	b := newTextBackend(t, "b")
	fallback := newTextBackend(t, "fallback")
	defer a.Close()
	defer b.Close()
	defer fallback.Close()
	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"matched": {Targets: []string{a.URL, b.URL}}},
		Sites: []siteConfig{{
			Hosts:  []string{"*"},
			Target: fallback.URL,
			Routes: []routeConfig{{
				Path:    "/*",
				Backend: "matched",
				Match:   &routeMatchConfig{Header: map[string][]string{"X-Match": {"yes"}}},
			}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := proxyResponse(t, handler, "proxy", "/"); got != "fallback" {
		t.Fatalf("ineligible response = %q", got)
	}
	for _, want := range []string{"a", "b"} {
		req := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
		req.Header.Set("X-Match", "yes")
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, req)
		if got := res.Body.String(); got != want {
			t.Fatalf("backend = %q, want %q", got, want)
		}
	}
}
