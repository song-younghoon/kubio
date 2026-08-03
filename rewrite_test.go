package main

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestRewriteProxyPath(t *testing.T) {
	tests := []struct {
		name        string
		source      string
		destination string
		path        string
		rawPath     string
		wantPath    string
		wantRaw     string
	}{
		{name: "wildcard exact prefix", source: "/api/*", destination: "/v1/*", path: "/api", wantPath: "/v1"},
		{name: "wildcard trailing slash", source: "/api/*", destination: "/v1/*", path: "/api/", wantPath: "/v1/"},
		{name: "wildcard suffix", source: "/api/*", destination: "/v1/*", path: "/api/users", wantPath: "/v1/users"},
		{name: "exact source", source: "/health", destination: "/status", path: "/health", wantPath: "/status"},
		{name: "root wildcard", source: "/*", destination: "/v1/*", path: "/users", wantPath: "/v1/users"},
		{name: "empty result", source: "/api", destination: "/*", path: "/api", wantPath: "/"},
		{name: "escaped suffix", source: "/api/*", destination: "/v1/*", path: "/api/users", rawPath: "/api/%75sers", wantPath: "/v1/users", wantRaw: "/v1/%75sers"},
		{name: "escaped source prefix", source: "/api/a/*", destination: "/v1/*", path: "/api/a/users", rawPath: "/api%2Fa/users", wantPath: "/v1/users", wantRaw: "/v1/users"},
		{name: "invalid raw path", source: "/api/*", destination: "/v1/*", path: "/api/users", rawPath: "/api/%ZZ", wantPath: "/v1/users"},
		{name: "literal percent config", source: "/api/*", destination: "/v%20/*", path: "/api/users", wantPath: "/v%20/users"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			source, err := newPathPattern(test.source)
			if err != nil {
				t.Fatal(err)
			}
			destination, err := newPathPattern(test.destination)
			if err != nil {
				t.Fatal(err)
			}
			in := &url.URL{Path: test.path, RawPath: test.rawPath}
			out := new(url.URL)
			rewriteProxyPath(out, in, source, destination)
			if out.Path != test.wantPath || out.RawPath != test.wantRaw {
				t.Fatalf("rewrite = path %q raw %q; want path %q raw %q", out.Path, out.RawPath, test.wantPath, test.wantRaw)
			}
		})
	}
}

func TestRewritePreservesQueryAndDoesNotChangeClientPath(t *testing.T) {
	var gotPath, gotRawPath, gotQuery string
	var gotForceQuery bool
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		gotRawPath = r.URL.RawPath
		gotQuery = r.URL.RawQuery
		gotForceQuery = r.URL.ForceQuery
		_, _ = w.Write([]byte("ok"))
	}))
	defer upstream.Close()

	router, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"example.com"},
		Target: upstream.URL,
		Routes: []routeConfig{{Path: "/api/*", Rewrite: "/v1/*"}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	req := httptest.NewRequest(http.MethodGet, "http://example.com/api/users", nil)
	req.URL.RawPath = "/api/%75sers"
	req.URL.RawQuery = ""
	req.URL.ForceQuery = true
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, req)

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d; want 200", recorder.Code)
	}
	if gotPath != "/v1/users" || gotRawPath != "/v1/%75sers" {
		t.Fatalf("upstream path = %q raw %q; want /v1/users and /v1/%%75sers", gotPath, gotRawPath)
	}
	if gotQuery != "" || !gotForceQuery {
		t.Fatalf("upstream query = %q force=%v; want raw query and force=true", gotQuery, gotForceQuery)
	}
	if req.URL.Path != "/api/users" || req.URL.RawPath != "/api/%75sers" {
		t.Fatalf("client request changed to path %q raw %q", req.URL.Path, req.URL.RawPath)
	}
}

func TestRewriteConfigValidation(t *testing.T) {
	base := `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/api/*","rewrite":"/v1/*"}]}]}`
	if _, err := decodeConfig([]byte(base)); err != nil {
		t.Fatalf("valid rewrite rejected: %v", err)
	}
	for _, value := range []string{
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/api","rewrite":""}]}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/api","rewrite":"/v1/*","strip":true}]}]}`,
		`{"listen":":8080","rewrite":"/v1","sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/api","rewrite":null}]}]}`,
	} {
		if _, err := decodeConfig([]byte(value)); err == nil {
			t.Fatalf("invalid rewrite config accepted: %s", value)
		}
	}
}

func TestRewriteHasNoRoutePathRewriteForUnselectedRoute(t *testing.T) {
	var path string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		path = r.URL.Path
		_, _ = w.Write([]byte("ok"))
	}))
	defer upstream.Close()

	router, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"example.com"},
		Target: upstream.URL,
		Routes: []routeConfig{
			{Path: "/api/*", Rewrite: "/v1/*"},
			{Path: "/other/*"},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	req := httptest.NewRequest(http.MethodGet, "http://example.com/other/item", nil)
	router.ServeHTTP(httptest.NewRecorder(), req)
	if path != "/other/item" {
		t.Fatalf("unselected route changed upstream path to %q", path)
	}
	if strings.Contains(path, "/v1") {
		t.Fatalf("unselected rewrite applied to %q", path)
	}
}
