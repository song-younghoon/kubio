package main

import (
	"net/http"
	"net/http/httptest"
	"slices"
	"strconv"
	"testing"
)

func BenchmarkBackendSelection(b *testing.B) {
	for _, count := range []int{1, 2, 8} {
		b.Run(strconv.Itoa(count), func(b *testing.B) {
			targets := make([]string, count)
			for index := range targets {
				targets[index] = "http://backend-" + strconv.Itoa(index) + ":3000"
			}
			backend, err := newBackend(backendConfig{Targets: targets})
			if err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				_ = backend.nextTarget()
			}
		})
	}
}

func BenchmarkBackendSelectionParallel(b *testing.B) {
	backend, err := newBackend(backendConfig{Targets: []string{
		"http://backend-0:3000",
		"http://backend-1:3000",
		"http://backend-2:3000",
	}})
	if err != nil {
		b.Fatal(err)
	}
	b.ReportAllocs()
	b.RunParallel(func(pb *testing.PB) {
		for pb.Next() {
			_ = backend.nextTarget()
		}
	})
}

func BenchmarkLinearRouteSelection(b *testing.B) {
	patterns, err := newHostPatterns([]string{
		"*",
		"*.example.com",
		"*.api.example.com",
		"api.example.com",
	})
	if err != nil {
		b.Fatal(err)
	}
	routes := []pathPattern{
		{path: "", wildcard: true},
		{path: "/api", wildcard: true, depth: 1},
		{path: "/api/admin", wildcard: true, depth: 2},
		{path: "/health", depth: 1},
	}
	routeDefs := make([]route, len(routes))
	for index := range routes {
		routeDefs[index].pattern = routes[index]
	}
	host := "v1.api.example.com"
	path := "/api/admin/users"
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_, _ = bestHostMatch(host, patterns)
		var selected routeCandidate
		for index := range routes {
			prefix, ok := routes[index].match(path)
			if !ok {
				continue
			}
			candidate := routeCandidate{
				route:  &routeDefs[index],
				prefix: prefix,
				exact:  !routes[index].wildcard,
				depth:  routes[index].depth,
				index:  index,
			}
			if selected.route == nil || betterRoute(candidate, selected) {
				selected = candidate
			}
		}
		if selected.index < 0 {
			b.Fatal("unreachable")
		}
	}
}

func BenchmarkProxyRequest(b *testing.B) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: backend.URL,
		Routes: []routeConfig{{Path: "/api/*", Strip: true}},
	}}})
	if err != nil {
		b.Fatal(err)
	}
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		req := httptest.NewRequest(http.MethodGet, "http://proxy/api/users?x=1", nil)
		req.RemoteAddr = "198.51.100.7:1234"
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, req)
		if res.Code != http.StatusNoContent {
			b.Fatalf("status = %d", res.Code)
		}
	}
}

func BenchmarkApplyResponseHeaders(b *testing.B) {
	for _, test := range []struct {
		name            string
		policyHeaders   int
		upstreamHeaders int
		trailers        int
	}{
		{name: "small_4x16", policyHeaders: 4, upstreamHeaders: 16, trailers: 1},
		{name: "stress_64x128", policyHeaders: 64, upstreamHeaders: 128, trailers: 8},
	} {
		b.Run(test.name, func(b *testing.B) {
			configured := make(map[string][]string, test.policyHeaders)
			headers := make(http.Header, test.policyHeaders+test.upstreamHeaders)
			trailers := make(http.Header, test.trailers)
			for index := range test.upstreamHeaders {
				headers.Set("X-Upstream-"+strconv.Itoa(index), "upstream")
			}
			for index := range test.policyHeaders {
				name := "X-Policy-" + strconv.Itoa(index)
				configured[name] = []string{"configured"}
				headers[name] = []string{"upstream-a", "upstream-b"}
				if index < test.trailers {
					trailers[name] = nil
				}
			}
			configured["X-Policy-"+strconv.Itoa(test.policyHeaders-1)] = []string{""}
			response := &http.Response{Header: headers, Trailer: trailers}
			policy := responseHeaderPolicy{Set: configured}

			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				applyResponseHeaderPolicies(response, policy, responseHeaderPolicy{})
			}
			b.StopTimer()

			values := response.Header.Values("X-Policy-" + strconv.Itoa(test.policyHeaders-1))
			if len(values) != 1 || values[0] != "" {
				b.Fatalf("empty replacement = %q", values)
			}
		})
	}
}

func BenchmarkResponseHeaderPolicies(b *testing.B) {
	for _, test := range []struct {
		name            string
		policyHeaders   int
		upstreamHeaders int
		trailers        int
	}{
		{name: "small_6x16", policyHeaders: 6, upstreamHeaders: 16, trailers: 1},
		{name: "stress_60x128", policyHeaders: 60, upstreamHeaders: 128, trailers: 8},
	} {
		b.Run(test.name, func(b *testing.B) {
			site := responseHeaderPolicy{Set: map[string][]string{}, Add: map[string][]string{}}
			route := responseHeaderPolicy{Set: map[string][]string{}, Add: map[string][]string{}}
			headers := make(http.Header, test.policyHeaders+test.upstreamHeaders)
			trailers := make(http.Header, test.trailers)
			expected := make([][]string, test.policyHeaders)
			for index := range test.upstreamHeaders {
				headers.Set("X-Upstream-"+strconv.Itoa(index), "upstream")
			}
			for index := range test.policyHeaders {
				name := "X-Policy-" + strconv.Itoa(index)
				expected[index] = []string{"upstream-a", "upstream-b"}
				if index < test.trailers {
					trailers[name] = nil
				}
				switch index % 3 {
				case 0:
					site.Set[name] = []string{"site-a", "site-b"}
					route.Add[name] = []string{"route"}
					if index >= test.trailers {
						expected[index] = []string{"site-a", "site-b", "route"}
					}
				case 1:
					site.Add[name] = []string{"site"}
					route.Set[name] = []string{"upstream-a", "upstream-b"}
				case 2:
					site.Remove = append(site.Remove, name)
					route.Add[name] = []string{"route"}
					if index >= test.trailers {
						expected[index] = []string{"route"}
					}
				}
				headers[name] = expected[index]
			}
			response := &http.Response{Header: headers, Trailer: trailers}

			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				applyResponseHeaderPolicies(response, site, route)
			}
			b.StopTimer()

			for index, want := range expected {
				name := "X-Policy-" + strconv.Itoa(index)
				if values := response.Header.Values(name); !slices.Equal(values, want) {
					b.Fatalf("%s = %q, want %q", name, values, want)
				}
			}
		})
	}
}

func BenchmarkProxyRequestParallel(b *testing.B) {
	benchmarkProxyRequestParallel(b, func(target string) config {
		return config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: target}}}
	})
}

func BenchmarkBackendProxyRequestParallel(b *testing.B) {
	benchmarkProxyRequestParallel(b, func(target string) config {
		sameTarget := "HTTP" + target[len("http"):]
		return config{
			Backends: map[string]backendConfig{"app": {Targets: []string{target, sameTarget}}},
			Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app"}},
		}
	})
}

func benchmarkProxyRequestParallel(b *testing.B, configForTarget func(string) config) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()

	handler, err := newRouter(configForTarget(backend.URL))
	if err != nil {
		b.Fatal(err)
	}
	b.ReportAllocs()
	b.RunParallel(func(pb *testing.PB) {
		for pb.Next() {
			req := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
			req.RemoteAddr = "198.51.100.7:1234"
			res := httptest.NewRecorder()
			handler.ServeHTTP(res, req)
			if res.Code != http.StatusNoContent {
				b.Fatalf("status = %d", res.Code)
			}
		}
	})
}

func BenchmarkSiteSelection(b *testing.B) {
	for _, count := range []int{1, 8, 64, 512} {
		b.Run(strconv.Itoa(count), func(b *testing.B) {
			sites := make([]siteConfig, count)
			for index := range sites {
				sites[index] = siteConfig{
					Hosts:  []string{"site" + strconv.Itoa(index) + ".example.com"},
					Target: "http://127.0.0.1:3000",
				}
			}
			router, err := newRouter(config{Sites: sites})
			if err != nil {
				b.Fatal(err)
			}
			host := "site" + strconv.Itoa(count/2) + ".example.com"
			b.ReportAllocs()
			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				if router.selectSite(host) == nil {
					b.Fatal("site not selected")
				}
			}
		})
	}
}

func BenchmarkRouteTableSelection(b *testing.B) {
	for _, count := range []int{1, 8, 64, 512} {
		b.Run(strconv.Itoa(count), func(b *testing.B) {
			routes := make([]route, count)
			routes[0].pattern = pathPattern{path: "", wildcard: true}
			if count > 1 {
				routes[1].pattern = pathPattern{path: "/api", wildcard: true, depth: 1}
			}
			for index := 2; index < count; index++ {
				routes[index].pattern = pathPattern{path: "/unused-" + strconv.Itoa(index), depth: 1}
			}
			selected := site{routes: routes}
			if count > routeIndexThreshold {
				selected.buildRouteIndex()
			}
			b.ReportAllocs()
			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				if selected.selectRoute("/api/users", http.MethodGet).route == nil {
					b.Fatal("route not selected")
				}
			}
		})
	}
}

func BenchmarkMethodRouteSelection(b *testing.B) {
	api := pathPattern{path: "/api", wildcard: true, depth: 1}
	duplicates := make([]route, 64)
	for index := range duplicates {
		duplicates[index] = route{pattern: api, methods: []string{http.MethodPost}}
	}
	duplicates[len(duplicates)-1].methods = []string{http.MethodGet}

	for _, test := range []struct {
		name   string
		routes []route
		path   string
		method string
		want   int
	}{
		{name: "unrestricted", routes: []route{{pattern: api}}, path: "/api/users"},
		{name: "matching_method", routes: []route{{pattern: api, methods: []string{http.MethodGet}}}, path: "/api/users"},
		{
			name:   "matching_second_of_two",
			routes: []route{{pattern: api, methods: []string{http.MethodGet, http.MethodHead}}},
			path:   "/api/users",
			method: http.MethodHead,
		},
		{
			name: "method_ineligible_fallback",
			routes: []route{
				{pattern: pathPattern{wildcard: true}},
				{pattern: api, methods: []string{http.MethodPost}},
			},
			path: "/api/users",
		},
		{name: "duplicate_same_path_64", routes: duplicates, path: "/api/users", want: 63},
	} {
		b.Run(test.name, func(b *testing.B) {
			selected := site{routes: test.routes}
			method := test.method
			if method == "" {
				method = http.MethodGet
			}
			if len(test.routes) > routeIndexThreshold {
				selected.buildRouteIndex()
			}
			var result routeCandidate
			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				result = selected.selectRoute(test.path, method)
			}
			if result.route == nil || result.index != test.want {
				b.Fatalf("selected route %d, want %d", result.index, test.want)
			}
		})
	}
}
