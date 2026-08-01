package main

import (
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"
)

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

func BenchmarkProxyRequestParallel(b *testing.B) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	defer backend.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: backend.URL,
	}}})
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
				if selected.selectRoute("/api/users").route == nil {
					b.Fatal("route not selected")
				}
			}
		})
	}
}
