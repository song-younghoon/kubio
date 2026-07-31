package main

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestRouterMatchesWildcardHostAndStripsPathPrefix(t *testing.T) {
	var gotPath string
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		gotPath = req.URL.Path
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()

	handler, err := newRouter(config{
		Sites: []siteConfig{
			{
				Hosts:  []string{"*.example.com"},
				Target: backend.URL,
				Routes: []routeConfig{{
					Path:   "/api/*",
					Target: backend.URL,
					Strip:  true,
				}},
			},
		},
	})
	if err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequest(http.MethodGet, "http://proxy/api/users", nil)
	req.Host = "api.example.com:8080"
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)

	if res.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want %d", res.Code, http.StatusNoContent)
	}
	if gotPath != "/users" {
		t.Fatalf("backend path = %q, want %q", gotPath, "/users")
	}

	if matchesHost("v1.api.example.com", []string{"*.example.com"}) {
		t.Fatal("nested subdomain unexpectedly matched")
	}
	if matchesHost("example.com", []string{"*.example.com"}) {
		t.Fatal("apex domain unexpectedly matched")
	}
}
