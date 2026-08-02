package main

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
)

func newTextBackend(t *testing.T, text string) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(text))
	}))
}

func proxyResponse(t *testing.T, handler http.Handler, host, path string) string {
	return proxyMethodResponse(t, handler, http.MethodGet, host, path)
}

func proxyMethodResponse(t *testing.T, handler http.Handler, method, host, path string) string {
	t.Helper()
	req := httptest.NewRequest(method, "http://proxy"+path, nil)
	req.Host = host
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, req)
	return res.Body.String()
}

func matchesHost(host string, patterns []string) bool {
	parsed, err := newHostPatterns(patterns)
	if err != nil {
		return false
	}
	_, ok := bestHostMatch(normalizeHost(host), parsed)
	return ok
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

func TestDecodeConfigMethods(t *testing.T) {
	t.Setenv("KUBIO_METHOD", "EXPANDED")
	valid := "{\"listen\":\":8080\",\"sites\":[{\"hosts\":[\"*\"],\"target\":\"http://localhost:3000\",\"routes\":[{\"path\":\"/*\",\"methods\":[\"GET\",\"get\",\"!#$%&'*+-.^_`|~AZaz09\",\"$KUBIO_METHOD\"]}]}]}"
	cfg, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatalf("valid methods rejected: %v", err)
	}
	methods := cfg.Sites[0].Routes[0].Methods
	if len(methods) != 4 || methods[1] != "get" || methods[3] != "$KUBIO_METHOD" {
		t.Fatalf("decoded methods = %q", methods)
	}

	withMethods := func(value string) string {
		return `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","methods":` + value + `}]}]}`
	}
	invalid := map[string]string{
		"null":               withMethods(`null`),
		"not array":          withMethods(`"GET"`),
		"empty array":        withMethods(`[]`),
		"null item":          withMethods(`[null]`),
		"non-string item":    withMethods(`[1]`),
		"empty item":         withMethods(`[""]`),
		"whitespace":         withMethods(`[" GET"]`),
		"separator":          withMethods(`["GET/POST"]`),
		"non-ASCII":          withMethods(`["GÉT"]`),
		"duplicate":          withMethods(`["GET","GET"]`),
		"environment syntax": withMethods(`["${KUBIO_METHOD}"]`),
		"duplicate field":    `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","methods":["GET"],"methods":["POST"]}]}]}`,
		"wrong case":         `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","Methods":["GET"]}]}]}`,
		"site field":         `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","methods":["GET"]}]}`,
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(data)); err == nil {
				t.Fatal("invalid methods were accepted")
			}
		})
	}
}

func TestValidMethodUsesExactHTTPTokenSet(t *testing.T) {
	const allowed = "!#$%&'*+-.^_`|~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
	for value := 0; value < 256; value++ {
		method := string([]byte{byte(value)})
		want := strings.IndexByte(allowed, byte(value)) >= 0
		if got := validMethod(method); got != want {
			t.Errorf("validMethod(%q) = %t, want %t", method, got, want)
		}
	}
}

func TestDecodeConfigSupportsBackends(t *testing.T) {
	data := `{
  "listen": ":8080",
  "backends": {
    "app": {"targets": ["http://app-1:3000", "http://app-2:3000"]}
  },
  "sites": [{
    "hosts": ["*"],
    "backend": "app",
    "routes": [
      {"path": "/inherited/*"},
      {"path": "/direct/*", "target": "http://legacy:3000"},
      {"path": "/named/*", "backend": "app"}
    ]
  }]
}`
	cfg, err := decodeConfig([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	if got := cfg.Backends["app"].Targets; len(got) != 2 || got[0] != "http://app-1:3000" || got[1] != "http://app-2:3000" {
		t.Fatalf("backend targets = %v", got)
	}
	if cfg.Sites[0].Backend != "app" || cfg.Sites[0].Routes[0].Target != "" || cfg.Sites[0].Routes[0].Backend != "" ||
		cfg.Sites[0].Routes[1].Target != "http://legacy:3000" || cfg.Sites[0].Routes[2].Backend != "app" {
		t.Fatalf("decoded selections = %#v", cfg.Sites[0])
	}

	invalid := map[string]string{
		"null backends":            `{"listen":":8080","backends":null,"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"non-object backends":      `{"listen":":8080","backends":[],"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"empty backend name":       `{"listen":":8080","backends":{"":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"null backend":             `{"listen":":8080","backends":{"app":null},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"missing targets":          `{"listen":":8080","backends":{"app":{}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"null targets":             `{"listen":":8080","backends":{"app":{"targets":null}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"non-array targets":        `{"listen":":8080","backends":{"app":{"targets":"http://localhost:3000"}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"empty targets":            `{"listen":":8080","backends":{"app":{"targets":[]}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"non-string target":        `{"listen":":8080","backends":{"app":{"targets":[1]}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"unknown backend field":    `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"extra":true}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"duplicate backend key":    `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]},"app":{"targets":["http://localhost:4000"]}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"site both selections":     `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","backend":"app"}]}`,
		"site neither selection":   `{"listen":":8080","sites":[{"hosts":["*"]}]}`,
		"empty site backend":       `{"listen":":8080","sites":[{"hosts":["*"],"backend":""}]}`,
		"non-string site backend":  `{"listen":":8080","sites":[{"hosts":["*"],"backend":1}]}`,
		"null target with backend": `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"target":null,"backend":"app"}]}`,
		"null backend with target": `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","backend":null}]}`,
		"route both selections":    `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"backend":"app","routes":[{"path":"/*","target":"http://localhost:4000","backend":"app"}]}]}`,
		"empty route backend":      `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"backend":"app","routes":[{"path":"/*","backend":""}]}]}`,
		"non-string route backend": `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"backend":"app","routes":[{"path":"/*","backend":1}]}]}`,
		"null route backend":       `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"]}},"sites":[{"hosts":["*"],"backend":"app","routes":[{"path":"/*","backend":null}]}]}`,
	}
	for name, raw := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(raw)); err == nil {
				t.Fatal("invalid config was accepted")
			}
		})
	}
}

func TestBackendValidation(t *testing.T) {
	directSite := siteConfig{Hosts: []string{"*"}, Target: "http://localhost:3000"}
	tests := map[string]config{
		"empty backend name": {
			Backends: map[string]backendConfig{"": {Targets: []string{"http://localhost:3000"}}},
			Sites:    []siteConfig{directSite},
		},
		"empty targets": {
			Backends: map[string]backendConfig{"app": {}},
			Sites:    []siteConfig{directSite},
		},
		"duplicate targets": {
			Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000", "http://localhost:3000"}}},
			Sites:    []siteConfig{directSite},
		},
		"invalid target": {
			Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000/path"}}},
			Sites:    []siteConfig{directSite},
		},
		"undefined site backend": {
			Sites: []siteConfig{{Hosts: []string{"*"}, Backend: "missing"}},
		},
		"case-sensitive reference": {
			Backends: map[string]backendConfig{"App": {Targets: []string{"http://localhost:3000"}}},
			Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		},
		"site both selections": {
			Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000"}}},
			Sites:    []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000", Backend: "app"}},
		},
		"site neither selection": {
			Sites: []siteConfig{{Hosts: []string{"*"}}},
		},
		"route both selections": {
			Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000"}}},
			Sites: []siteConfig{{
				Hosts: []string{"*"}, Target: "http://localhost:3000",
				Routes: []routeConfig{{Path: "/*", Target: "http://localhost:4000", Backend: "app"}},
			}},
		},
		"undefined route backend": {
			Sites: []siteConfig{{
				Hosts: []string{"*"}, Target: "http://localhost:3000",
				Routes: []routeConfig{{Path: "/*", Backend: "missing"}},
			}},
		},
		"empty route methods": {
			Sites: []siteConfig{{
				Hosts: []string{"*"}, Target: "http://localhost:3000",
				Routes: []routeConfig{{Path: "/*", Methods: []string{}}},
			}},
		},
		"invalid route method": {
			Sites: []siteConfig{{
				Hosts: []string{"*"}, Target: "http://localhost:3000",
				Routes: []routeConfig{{Path: "/*", Methods: []string{"GET POST"}}},
			}},
		},
		"duplicate route method": {
			Sites: []siteConfig{{
				Hosts: []string{"*"}, Target: "http://localhost:3000",
				Routes: []routeConfig{{Path: "/*", Methods: []string{"GET", "GET"}}},
			}},
		},
	}
	for name, cfg := range tests {
		t.Run(name, func(t *testing.T) {
			if _, err := newRouter(cfg); err == nil {
				t.Fatal("invalid config was accepted")
			}
		})
	}

	if _, err := newRouter(config{
		Backends: map[string]backendConfig{
			" unused ": {Targets: []string{"http://localhost:4000"}},
		},
		Sites: []siteConfig{directSite},
	}); err != nil {
		t.Fatalf("valid unused backend rejected: %v", err)
	}
}

func TestBackendRoundRobinIsSharedAndRoutesInherit(t *testing.T) {
	newObservedBackend := func(name string) *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			_, _ = w.Write([]byte(name + "|" + req.URL.Path + "|" + req.Header.Get("X-Site") + "|" + req.Header.Get("X-Route")))
		}))
	}
	a := newObservedBackend("a")
	b := newObservedBackend("b")
	c := newObservedBackend("c")
	direct := newObservedBackend("direct")
	defer a.Close()
	defer b.Close()
	defer c.Close()
	defer direct.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{
			"app":   {Targets: []string{a.URL, b.URL, c.URL}},
			"other": {Targets: []string{c.URL, a.URL}},
		},
		Sites: []siteConfig{
			{
				Hosts:   []string{"one.test"},
				Backend: "app",
				Headers: map[string]string{"X-Site": "site"},
				Routes: []routeConfig{
					{Path: "/inherit/*", Headers: map[string]string{"X-Route": "inherit"}, Strip: true},
					{Path: "/same/*", Backend: "app"},
					{Path: "/direct/*", Target: direct.URL},
					{Path: "/other/*", Backend: "other"},
				},
			},
			{Hosts: []string{"two.test"}, Backend: "app"},
		},
	})
	if err != nil {
		t.Fatal(err)
	}

	requests := []struct {
		host string
		path string
		want string
	}{
		{host: "one.test", path: "/", want: "a|/|site|"},
		{host: "one.test", path: "/inherit/users", want: "b|/users|site|inherit"},
		{host: "two.test", path: "/", want: "c|/||"},
		{host: "one.test", path: "/same/value", want: "a|/same/value|site|"},
		{host: "one.test", path: "/direct/value", want: "direct|/direct/value|site|"},
		{host: "one.test", path: "/", want: "b|/|site|"},
		{host: "one.test", path: "/other/value", want: "c|/other/value|site|"},
		{host: "one.test", path: "/", want: "c|/|site|"},
		{host: "one.test", path: "/other/value", want: "a|/other/value|site|"},
	}
	for _, request := range requests {
		if got := proxyResponse(t, handler, request.host, request.path); got != request.want {
			t.Errorf("%s %s = %q, want %q", request.host, request.path, got, request.want)
		}
	}
}

func TestBackendSelectionIsConsumedOnFailureAndStatus(t *testing.T) {
	closed := newTextBackend(t, "closed")
	closedURL := closed.URL
	closed.Close()
	healthy := newTextBackend(t, "healthy")
	defer healthy.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{closedURL, healthy.URL}}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	res := httptest.NewRecorder()
	handler.ServeHTTP(res, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if res.Code != http.StatusBadGateway {
		t.Fatalf("failed target status = %d, want %d", res.Code, http.StatusBadGateway)
	}
	if got := proxyResponse(t, handler, "proxy", "/"); got != "healthy" {
		t.Fatalf("target after failure = %q, want healthy", got)
	}

	first := newTextBackend(t, "first")
	second := newTextBackend(t, "second")
	defer first.Close()
	defer second.Close()
	canceledHandler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{first.URL, second.URL}}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil).WithContext(ctx)
	canceledHandler.ServeHTTP(httptest.NewRecorder(), request)
	if got := proxyResponse(t, canceledHandler, "proxy", "/"); got != "second" {
		t.Fatalf("target after client cancellation = %q, want second", got)
	}

	unavailable := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer unavailable.Close()
	statusHandler, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{unavailable.URL, healthy.URL}}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	res = httptest.NewRecorder()
	statusHandler.ServeHTTP(res, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if res.Code != http.StatusServiceUnavailable {
		t.Fatalf("upstream status = %d, want %d", res.Code, http.StatusServiceUnavailable)
	}
	if got := proxyResponse(t, statusHandler, "proxy", "/"); got != "healthy" {
		t.Fatalf("target after upstream status = %q, want healthy", got)
	}
}

func TestBackendSelectionIsConcurrencySafe(t *testing.T) {
	backend, err := newBackend([]string{"http://a:3000", "http://b:3000", "http://c:3000"})
	if err != nil {
		t.Fatal(err)
	}

	const requests = 300
	start := make(chan struct{})
	selected := make(chan string, requests)
	var workers sync.WaitGroup
	workers.Add(requests)
	for range requests {
		go func() {
			defer workers.Done()
			<-start
			selected <- backend.nextTarget().Hostname()
		}()
	}
	close(start)
	workers.Wait()
	close(selected)

	counts := map[string]int{}
	for host := range selected {
		counts[host]++
	}
	for _, host := range []string{"a", "b", "c"} {
		if counts[host] != requests/3 {
			t.Fatalf("selection counts = %v", counts)
		}
	}
}

func TestBackendStateSurvivesFailedReloadAndResetsOnSuccess(t *testing.T) {
	a := newTextBackend(t, "a")
	b := newTextBackend(t, "b")
	defer a.Close()
	defer b.Close()
	cfg := config{
		Backends: map[string]backendConfig{"app": {Targets: []string{a.URL, b.URL}}},
		Sites: []siteConfig{{
			Hosts:   []string{"*"},
			Backend: "app",
			Routes:  []routeConfig{{Path: "/*", Methods: []string{"GET"}}},
		}},
	}
	initial, err := newRouter(cfg)
	if err != nil {
		t.Fatal(err)
	}
	handler := newReloadableRouter(initial)
	if got := proxyResponse(t, handler, "proxy", "/"); got != "a" {
		t.Fatalf("initial target = %q, want a", got)
	}
	if _, err := newRouter(config{
		Backends: cfg.Backends,
		Sites: []siteConfig{{
			Hosts:   []string{"*"},
			Backend: "app",
			Routes:  []routeConfig{{Path: "/*", Methods: []string{}}},
		}},
	}); err == nil {
		t.Fatal("invalid methods reload was accepted")
	}
	if got := proxyResponse(t, handler, "proxy", "/"); got != "b" {
		t.Fatalf("target after failed reload = %q, want b", got)
	}

	reloaded, err := newRouter(config{
		Backends: cfg.Backends,
		Sites: []siteConfig{{
			Hosts:   []string{"*"},
			Backend: "app",
			Routes:  []routeConfig{{Path: "/*", Methods: []string{"GET", "POST"}}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	handler.Store(reloaded)
	if got := proxyResponse(t, handler, "proxy", "/"); got != "a" {
		t.Fatalf("target after successful reload = %q, want a", got)
	}
}

func TestInFlightBackendRequestKeepsPreviousRouter(t *testing.T) {
	entered := make(chan struct{}, 1)
	release := make(chan struct{})
	oldA := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		entered <- struct{}{}
		<-release
		_, _ = w.Write([]byte("old"))
	}))
	oldB := newTextBackend(t, "old-b")
	newA := newTextBackend(t, "new")
	newB := newTextBackend(t, "new-b")
	defer oldA.Close()
	defer oldB.Close()
	defer newA.Close()
	defer newB.Close()
	defer close(release)

	build := func(first, second string) *router {
		router, err := newRouter(config{
			Backends: map[string]backendConfig{"app": {Targets: []string{first, second}}},
			Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		})
		if err != nil {
			t.Fatal(err)
		}
		return router
	}
	handler := newReloadableRouter(build(oldA.URL, oldB.URL))
	oldResult := make(chan string, 1)
	go func() {
		oldResult <- proxyResponse(t, handler, "proxy", "/")
	}()
	<-entered
	handler.Store(build(newA.URL, newB.URL))
	if got := proxyResponse(t, handler, "proxy", "/"); got != "new" {
		t.Fatalf("new router target = %q, want new", got)
	}
	release <- struct{}{}
	if got := <-oldResult; got != "old" {
		t.Fatalf("in-flight target = %q, want old", got)
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

func TestRouteMethodsMatchExactlyAndPreserveMethod(t *testing.T) {
	newBackend := func(name string) *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			w.Header().Set("X-Backend", name)
			w.Header().Set("X-Method", req.Method)
		}))
	}
	fallback := newBackend("fallback")
	upper := newBackend("upper")
	lower := newBackend("lower")
	open := newBackend("open")
	defer fallback.Close()
	defer upper.Close()
	defer lower.Close()
	defer open.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: fallback.URL,
		Routes: []routeConfig{
			{Path: "/jobs/*", Methods: []string{"GET"}, Target: upper.URL},
			{Path: "/jobs/*", Methods: []string{"get"}, Target: lower.URL},
			{Path: "/open/*", Target: open.URL},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		method  string
		path    string
		header  string
		backend string
	}{
		{method: "GET", path: "/jobs/1", backend: "upper"},
		{method: "get", path: "/jobs/1", backend: "lower"},
		{method: "HEAD", path: "/jobs/1", backend: "fallback"},
		{method: "POST", path: "/jobs/1", header: "GET", backend: "fallback"},
		{method: "PATCH", path: "/open/1", backend: "open"},
	}
	for _, test := range tests {
		req := httptest.NewRequest(test.method, "http://proxy"+test.path, nil)
		if test.header != "" {
			req.Header.Set("X-HTTP-Method-Override", test.header)
		}
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, req)
		if got := res.Header().Get("X-Backend"); got != test.backend {
			t.Errorf("%s %s backend = %q, want %q", test.method, test.path, got, test.backend)
		}
		if got := res.Header().Get("X-Method"); got != test.method {
			t.Errorf("%s %s upstream method = %q", test.method, test.path, got)
		}
	}
}

func TestRouteMethodPriorityAndFallback(t *testing.T) {
	definitions := []struct {
		path    string
		methods []string
	}{
		{path: "/*", methods: []string{"GET"}},
		{path: "/api/*"},
		{path: "/api/admin/*", methods: []string{"POST"}},
		{path: "/jobs/*", methods: []string{"GET"}},
		{path: "/jobs/*", methods: []string{"GET", "POST"}},
		{path: "/open/*", methods: []string{"GET"}},
		{path: "/open/*"},
		{path: "/tie/*", methods: []string{"GET", "POST"}},
		{path: "/tie/*", methods: []string{"GET", "PUT"}},
		{path: "/exact"},
		{path: "/exact/*", methods: []string{"GET"}},
	}
	routes := make([]route, len(definitions))
	for index, definition := range definitions {
		pattern, err := newPathPattern(definition.path)
		if err != nil {
			t.Fatal(err)
		}
		routes[index] = route{pattern: pattern, methods: definition.methods}
	}
	selected := site{routes: routes}
	for _, test := range []struct {
		path   string
		method string
		want   int
	}{
		{path: "/api/admin/1", method: "GET", want: 1},
		{path: "/api/admin/1", method: "POST", want: 2},
		{path: "/jobs/1", method: "GET", want: 3},
		{path: "/jobs/1", method: "POST", want: 4},
		{path: "/open/1", method: "GET", want: 5},
		{path: "/open/1", method: "PUT", want: 6},
		{path: "/tie/1", method: "GET", want: 8},
		{path: "/exact", method: "GET", want: 9},
		{path: "/none", method: "DELETE", want: -1},
	} {
		got := selected.selectRoute(test.path, test.method)
		if test.want < 0 {
			if got.route != nil {
				t.Errorf("%s %s selected route %d, want none", test.method, test.path, got.index)
			}
			continue
		}
		if got.route == nil || got.index != test.want {
			t.Errorf("%s %s selected route %d, want %d", test.method, test.path, got.index, test.want)
		}
	}
}

func TestMethodIneligibleRouteDoesNotApplyOrConsumeBackend(t *testing.T) {
	newBackend := func(name string) *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			_, _ = w.Write([]byte(name + "|" + req.URL.Path + "|" + req.Header.Get("X-Route")))
		}))
	}
	fallback := newBackend("fallback")
	a := newBackend("a")
	b := newBackend("b")
	defer fallback.Close()
	defer a.Close()
	defer b.Close()

	handler, err := newRouter(config{
		Backends: map[string]backendConfig{"writers": {Targets: []string{a.URL, b.URL}}},
		Sites: []siteConfig{{
			Hosts:  []string{"*"},
			Target: fallback.URL,
			Routes: []routeConfig{{
				Path:    "/jobs/*",
				Methods: []string{"POST"},
				Backend: "writers",
				Headers: map[string]string{"X-Route": "selected"},
				Strip:   true,
			}},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}

	if got := proxyMethodResponse(t, handler, "GET", "proxy", "/jobs/1"); got != "fallback|/jobs/1|" {
		t.Fatalf("ineligible route result = %q", got)
	}
	if got := proxyMethodResponse(t, handler, "POST", "proxy", "/jobs/1"); got != "a|/1|selected" {
		t.Fatalf("first eligible route result = %q", got)
	}
	if got := proxyMethodResponse(t, handler, "POST", "proxy", "/jobs/2"); got != "b|/2|selected" {
		t.Fatalf("second eligible route result = %q", got)
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

func TestIndexedRouteSelectionPreservesPriority(t *testing.T) {
	all := newTextBackend(t, "all")
	api := newTextBackend(t, "api")
	apiLast := newTextBackend(t, "api-last")
	admin := newTextBackend(t, "admin")
	health := newTextBackend(t, "health")
	defer all.Close()
	defer api.Close()
	defer apiLast.Close()
	defer admin.Close()
	defer health.Close()

	routeConfigs := []routeConfig{
		{Path: "/*", Target: all.URL},
		{Path: "/api/*", Target: api.URL},
		{Path: "/api/admin/*", Target: admin.URL},
		{Path: "/health", Target: health.URL},
		{Path: "/unused-a", Target: all.URL},
		{Path: "/unused-b", Target: all.URL},
		{Path: "/unused-c", Target: all.URL},
		{Path: "/unused-d", Target: all.URL},
		{Path: "/unused-e", Target: all.URL},
		{Path: "/api/*", Target: apiLast.URL},
	}
	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: all.URL,
		Routes: routeConfigs,
	}}})
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		path string
		want string
	}{
		{path: "/", want: "all"},
		{path: "/api/users", want: "api-last"},
		{path: "/api/admin/users", want: "admin"},
		{path: "/health", want: "health"},
	} {
		if got := proxyResponse(t, handler, "anything", test.path); got != test.want {
			t.Errorf("path %q = %q, want %q", test.path, got, test.want)
		}
	}
}

func TestIndexedRouteSelectionMatchesLinearSelection(t *testing.T) {
	definitions := []struct {
		path    string
		methods []string
	}{
		{path: "/*"},
		{path: "/api/*", methods: []string{"GET", "POST"}},
		{path: "/api/*", methods: []string{"GET"}},
		{path: "/api/admin/*", methods: []string{"POST"}},
		{path: "/health", methods: []string{"GET"}},
		{path: "/health", methods: []string{"POST"}},
		{path: "/unused-a"},
		{path: "/unused-b"},
		{path: "/unused-c"},
		{path: "/unused-d"},
	}
	routes := make([]route, len(definitions))
	for index, definition := range definitions {
		pattern, err := newPathPattern(definition.path)
		if err != nil {
			t.Fatal(err)
		}
		routes[index] = route{pattern: pattern, methods: definition.methods}
	}

	linear := site{routes: routes}
	indexed := site{routes: routes}
	indexed.buildRouteIndex()
	for _, test := range []struct {
		path   string
		method string
	}{
		{path: "/", method: "DELETE"},
		{path: "/api/users", method: "GET"},
		{path: "/api/users", method: "POST"},
		{path: "/api/users", method: "DELETE"},
		{path: "/api/admin/users", method: "GET"},
		{path: "/api/admin/users", method: "POST"},
		{path: "/health", method: "GET"},
		{path: "/health", method: "POST"},
		{path: "/health", method: "PUT"},
		{path: "/other", method: "GET"},
	} {
		want := linear.selectRoute(test.path, test.method)
		got := indexed.selectRoute(test.path, test.method)
		if got.route == nil || want.route == nil {
			if got.route != want.route {
				t.Errorf("%s %s selected route mismatch", test.method, test.path)
			}
			continue
		}
		if got.index != want.index || got.prefix != want.prefix {
			t.Errorf("%s %s selected route (%d, %q), want (%d, %q)", test.method, test.path, got.index, got.prefix, want.index, want.prefix)
		}
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
