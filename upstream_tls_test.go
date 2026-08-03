package main

import (
	"encoding/pem"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDecodeUpstreamTLSLoadsCAAndValidatesNames(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("ok"))
	}))
	defer server.Close()

	caPath := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(caPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: server.Certificate().Raw}), 0o600); err != nil {
		t.Fatal(err)
	}
	data := `{"listen":":8080","sites":[{"hosts":["*"],"target":"` + server.URL + `","tls":{"ca":"` + caPath + `","name":"example.com"}}]}`
	cfg, err := decodeConfig([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Sites[0].TLS == nil || cfg.Sites[0].TLS.CAPath != caPath || cfg.Sites[0].TLS.RootCAs != nil || cfg.Sites[0].TLS.Name != "example.com" {
		t.Fatalf("decoded upstream tls = %#v", cfg.Sites[0].TLS)
	}
	router, err := newRouter(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()
	if got := router.directTransports[0].TLSClientConfig.ServerName; got != "example.com" {
		t.Fatalf("server name = %q", got)
	}
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	request.Host = "example.com"
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusOK || response.Body.String() != "ok" {
		t.Fatalf("response = %d %q", response.Code, response.Body.String())
	}
}

func TestUpstreamTLSRejectsHTTPAndBackendOverrides(t *testing.T) {
	nameOnly := &upstreamTLSConfig{Name: "example.com"}
	if _, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000", TLS: nameOnly}}}); err == nil {
		t.Fatal("HTTP direct target accepted upstream TLS")
	}
	if _, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000"}, TLS: nameOnly}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app", TLS: nameOnly}},
	}); err == nil {
		t.Fatal("site upstream TLS accepted with backend")
	}
	if _, err := newRouter(config{
		Backends: map[string]backendConfig{"app": {Targets: []string{"http://localhost:3000"}}},
		Sites:    []siteConfig{{Hosts: []string{"*"}, Backend: "app", Routes: []routeConfig{{Path: "/*", Backend: "app", TLS: nameOnly}}}},
	}); err == nil {
		t.Fatal("route upstream TLS accepted with backend")
	}
}

func TestUpstreamTLSStrictCAParsing(t *testing.T) {
	valid := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: []byte{1, 2, 3}})
	for name, data := range map[string][]byte{
		"empty":            nil,
		"garbage":          []byte("secret\n"),
		"wrong block":      pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: []byte("secret")}),
		"invalid cert":     valid,
		"trailing garbage": append(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: serverCertificateDER(t)}), []byte("secret")...),
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := parseUpstreamCAPool(data); err == nil {
				t.Fatal("invalid CA data accepted")
			} else if strings.Contains(err.Error(), "secret") {
				t.Fatalf("CA contents leaked: %v", err)
			}
		})
	}
}

func serverCertificateDER(t *testing.T) []byte {
	t.Helper()
	server := httptest.NewTLSServer(http.NotFoundHandler())
	defer server.Close()
	return append([]byte(nil), server.Certificate().Raw...)
}

func TestValidateUpstreamServerName(t *testing.T) {
	for _, name := range []string{"example.com", "internal", "127.0.0.1", "2001:db8::1"} {
		if err := validateUpstreamServerName(name); err != nil {
			t.Errorf("valid name %q rejected: %v", name, err)
		}
	}
	for _, name := range []string{"", "*.example.com", "example.com.", "example..com", "https://example.com", "[::1]", "fe80::1%eth0", "bad name", "éxample.com", "-example.com", "example-.com"} {
		if err := validateUpstreamServerName(name); err == nil {
			t.Errorf("invalid name %q accepted", name)
		}
	}
}
