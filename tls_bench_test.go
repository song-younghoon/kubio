package main

import (
	"crypto/tls"
	"net/http"
	"net/http/httptest"
	"testing"
)

type benchmarkResponseWriter struct {
	header http.Header
}

func (w *benchmarkResponseWriter) Header() http.Header {
	return w.header
}

func (w *benchmarkResponseWriter) WriteHeader(int) {}

func (w *benchmarkResponseWriter) Write(value []byte) (int, error) {
	return len(value), nil
}

// These benchmarks isolate the reloadable handler dispatch from network I/O
// and the real server response-writer lifecycle.
func BenchmarkRouterServeHTTP(b *testing.B) {
	router, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: "http://127.0.0.1:3000"}}})
	if err != nil {
		b.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	response := &benchmarkResponseWriter{header: make(http.Header)}
	b.ReportAllocs()
	b.ResetTimer()
	for range b.N {
		router.ServeHTTP(response, request)
	}
}

func BenchmarkReloadableRouterServeHTTPNoMatchHandler(b *testing.B) {
	router, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: "http://127.0.0.1:3000"}}})
	if err != nil {
		b.Fatal(err)
	}
	handler := newReloadableRouter(router, nil)
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	response := &benchmarkResponseWriter{header: make(http.Header)}
	b.ReportAllocs()
	b.ResetTimer()
	for range b.N {
		handler.ServeHTTP(response, request)
	}
}

func BenchmarkTLSCertificateLookup(b *testing.B) {
	router, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://127.0.0.1:3000"}}})
	if err != nil {
		b.Fatal(err)
	}
	want := &tls.Certificate{}
	handler := newReloadableRouter(router, want)
	b.ReportAllocs()
	b.ResetTimer()
	for range b.N {
		got, err := handler.GetCertificate(nil)
		if err != nil || got != want {
			b.Fatalf("certificate = %p, %v; want %p", got, err, want)
		}
	}
}

func BenchmarkTLSCertificateLookupParallel(b *testing.B) {
	router, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://127.0.0.1:3000"}}})
	if err != nil {
		b.Fatal(err)
	}
	want := &tls.Certificate{}
	handler := newReloadableRouter(router, want)
	b.ReportAllocs()
	b.ResetTimer()
	b.RunParallel(func(pb *testing.PB) {
		for pb.Next() {
			got, err := handler.GetCertificate(nil)
			if err != nil || got != want {
				b.Fatalf("certificate = %p, %v; want %p", got, err, want)
			}
		}
	})
}
