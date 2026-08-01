package main

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestRouterMatchesWildcardHostAndStripsPathPrefix(t *testing.T) {
	var gotPath string
	var gotHeaders map[string]string
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		gotPath = req.URL.Path
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
