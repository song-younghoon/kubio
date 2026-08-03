package main

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"io"
	"log"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func writeTestCertificate(t *testing.T, directory, name string, serial int64, notBefore, notAfter time.Time) (*tlsConfig, *x509.Certificate) {
	t.Helper()
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(serial),
		Subject:               pkix.Name{CommonName: name},
		DNSNames:              []string{"example.com"},
		NotBefore:             notBefore,
		NotAfter:              notAfter,
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	certificateDER, err := x509.CreateCertificate(rand.Reader, template, template, publicKey, privateKey)
	if err != nil {
		t.Fatal(err)
	}
	privateDER, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		t.Fatal(err)
	}
	certPath := filepath.Join(directory, name+".crt")
	keyPath := filepath.Join(directory, name+".key")
	if err := os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificateDER}), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: privateDER}), 0o600); err != nil {
		t.Fatal(err)
	}
	parsed, err := x509.ParseCertificate(certificateDER)
	if err != nil {
		t.Fatal(err)
	}
	return &tlsConfig{Cert: certPath, Key: keyPath}, parsed
}

func TestDecodeTLSConfiguration(t *testing.T) {
	t.Setenv("KUBIO_CERT", "expanded")
	valid := `{"listen":":8443","tls":{"cert":" ${KUBIO_CERT} ","key":"same.pem"},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`
	cfg, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.TLS == nil || cfg.TLS.Cert != " ${KUBIO_CERT} " || cfg.TLS.Key != "same.pem" {
		t.Fatalf("tls = %#v", cfg.TLS)
	}
	absent, err := decodeConfig([]byte(`{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`))
	if err != nil || absent.TLS != nil {
		t.Fatalf("absent tls = %#v, err=%v", absent.TLS, err)
	}

	withTLS := func(value string) string {
		return `{"listen":":8443","tls":` + value + `,"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`
	}
	invalid := map[string]string{
		"null":                           withTLS(`null`),
		"boolean":                        withTLS(`true`),
		"string":                         withTLS(`"cert"`),
		"array":                          withTLS(`[]`),
		"empty object":                   withTLS(`{}`),
		"missing cert":                   withTLS(`{"key":"key.pem"}`),
		"missing key":                    withTLS(`{"cert":"cert.pem"}`),
		"null cert":                      withTLS(`{"cert":null,"key":"key.pem"}`),
		"null key":                       withTLS(`{"cert":"cert.pem","key":null}`),
		"numeric cert":                   withTLS(`{"cert":1,"key":"key.pem"}`),
		"numeric key":                    withTLS(`{"cert":"cert.pem","key":1}`),
		"empty cert":                     withTLS(`{"cert":"","key":"key.pem"}`),
		"empty key":                      withTLS(`{"cert":"cert.pem","key":""}`),
		"unknown field":                  withTLS(`{"cert":"cert.pem","key":"key.pem","extra":true}`),
		"duplicate tls":                  `{"listen":":8443","tls":{"cert":"a","key":"b"},"tls":{"cert":"c","key":"d"},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"duplicate cert":                 withTLS(`{"cert":"a","cert":"b","key":"c"}`),
		"https alias":                    `{"listen":":8443","https":{"cert":"a","key":"b"},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"certificate alias":              withTLS(`{"certificate":"a","key":"b"}`),
		"privateKey alias":               withTLS(`{"cert":"a","privateKey":"b"}`),
		"site incomplete client pair":    `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","tls":{"cert":"a"}}]}`,
		"route incomplete client pair":   `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","tls":{"cert":"a"}}]}]}`,
		"backend incomplete client pair": `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"tls":{"cert":"a"}}},"sites":[{"hosts":["*"],"backend":"app"}]}`,
		"match placement":                `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000","routes":[{"path":"/*","match":{"query":{"x":"a"},"tls":{"cert":"a","key":"b"}}}]}]}`,
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if _, err := decodeConfig([]byte(data)); err == nil {
				t.Fatal("invalid tls configuration was accepted")
			}
		})
	}
}

func TestLoadTLSCertificate(t *testing.T) {
	directory := t.TempDir()
	now := time.Now()
	first, firstLeaf := writeTestCertificate(t, directory, "first", 1, now.Add(-time.Hour), now.Add(time.Hour))
	second, _ := writeTestCertificate(t, directory, "second", 2, now.Add(-time.Hour), now.Add(time.Hour))
	loaded, err := loadTLSCertificate(first)
	if err != nil {
		t.Fatal(err)
	}
	leaf, err := x509.ParseCertificate(loaded.Certificate[0])
	if err != nil || leaf.SerialNumber.Cmp(firstLeaf.SerialNumber) != 0 {
		t.Fatalf("loaded leaf = %v, err=%v", leaf, err)
	}
	if certificate, err := loadTLSCertificate(nil); err != nil || certificate != nil {
		t.Fatalf("nil config = %v, %v", certificate, err)
	}
	if _, err := loadTLSCertificate(&tlsConfig{Cert: first.Cert, Key: second.Key}); err == nil {
		t.Fatal("mismatched key was accepted")
	}
	combined := filepath.Join(directory, "combined.pem")
	certificatePEM, err := os.ReadFile(first.Cert)
	if err != nil {
		t.Fatal(err)
	}
	keyPEM, err := os.ReadFile(first.Key)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(combined, append(certificatePEM, keyPEM...), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadTLSCertificate(&tlsConfig{Cert: combined, Key: combined}); err != nil {
		t.Fatalf("combined certificate and key rejected: %v", err)
	}
	if _, err := loadTLSCertificate(&tlsConfig{Cert: filepath.Join(directory, "missing.crt"), Key: first.Key}); err == nil {
		t.Fatal("missing certificate was accepted")
	}
	badCert := filepath.Join(directory, "bad.crt")
	badKey := filepath.Join(directory, "bad.key")
	if err := os.WriteFile(badCert, []byte("CERTIFICATE-SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(badKey, []byte("PRIVATE-KEY-SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadTLSCertificate(&tlsConfig{Cert: badCert, Key: badKey}); err == nil || strings.Contains(err.Error(), "CERTIFICATE-SECRET") || strings.Contains(err.Error(), "PRIVATE-KEY-SECRET") {
		t.Fatalf("unsafe malformed-pair error: %v", err)
	}
	expired, _ := writeTestCertificate(t, directory, "expired", 3, now.Add(-2*time.Hour), now.Add(-time.Hour))
	if _, err := loadTLSCertificate(expired); err != nil {
		t.Fatalf("expired certificate rejected: %v", err)
	}
}

func startTestTLSServer(t *testing.T, handler *reloadableRouter) (string, func()) {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := &http.Server{
		Handler: handler,
		TLSConfig: &tls.Config{
			MinVersion:     tls.VersionTLS12,
			GetCertificate: handler.GetCertificate,
		},
		ErrorLog: log.New(io.Discard, "", 0),
	}
	if len(server.TLSConfig.Certificates) != 0 {
		t.Fatal("static certificate slice is not empty")
	}
	done := make(chan error, 1)
	go func() { done <- server.ServeTLS(listener, "", "") }()
	closeServer := func() {
		_ = server.Close()
		if err := <-done; err != nil && !errors.Is(err, http.ErrServerClosed) {
			t.Errorf("serve TLS: %v", err)
		}
	}
	return listener.Addr().String(), closeServer
}

func dialTestTLS(t *testing.T, address, serverName string) *x509.Certificate {
	t.Helper()
	connection, err := tls.Dial("tcp", address, &tls.Config{
		InsecureSkipVerify: true,
		ServerName:         serverName,
		MinVersion:         tls.VersionTLS12,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	return connection.ConnectionState().PeerCertificates[0]
}

func TestTLSListenerAndAtomicGenerationReload(t *testing.T) {
	backend := func(name string) *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			_, _ = w.Write([]byte(name + "|" + req.Header.Get("X-Forwarded-Proto")))
		}))
	}
	oldBackend := backend("old")
	newBackend := backend("new")
	defer oldBackend.Close()
	defer newBackend.Close()
	oldRouter, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: oldBackend.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	updatedRouter, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"example.com"}, Target: newBackend.URL}}})
	if err != nil {
		t.Fatal(err)
	}
	directory := t.TempDir()
	now := time.Now()
	oldConfig, oldLeaf := writeTestCertificate(t, directory, "old", 10, now.Add(-time.Hour), now.Add(time.Hour))
	newConfig, newLeaf := writeTestCertificate(t, directory, "new", 20, now.Add(-time.Hour), now.Add(time.Hour))
	oldCertificate, err := loadTLSCertificate(oldConfig)
	if err != nil {
		t.Fatal(err)
	}
	newCertificate, err := loadTLSCertificate(newConfig)
	if err != nil {
		t.Fatal(err)
	}
	handler := newReloadableRouter(oldRouter, oldCertificate)
	address, closeServer := startTestTLSServer(t, handler)
	defer closeServer()

	for _, serverName := range []string{"", "unknown.example"} {
		if leaf := dialTestTLS(t, address, serverName); leaf.SerialNumber.Cmp(oldLeaf.SerialNumber) != 0 {
			t.Fatalf("%q certificate serial = %s", serverName, leaf.SerialNumber)
		}
	}
	transport := &http.Transport{
		ForceAttemptHTTP2: true,
		TLSClientConfig:   &tls.Config{InsecureSkipVerify: true, ServerName: "example.com", MinVersion: tls.VersionTLS12},
	}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	request := func() (string, string) {
		req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, "https://"+address+"/", nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Host = "example.com"
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		body, err := io.ReadAll(response.Body)
		if err != nil {
			t.Fatal(err)
		}
		return string(body), response.Proto
	}
	if body, proto := request(); body != "old|https" || proto != "HTTP/2.0" {
		t.Fatalf("old response = %q over %s", body, proto)
	}

	handler.StoreGeneration(updatedRouter, newCertificate)
	if body, _ := request(); body != "new|https" {
		t.Fatalf("new response = %q", body)
	}
	for _, serverName := range []string{"", "unknown.example"} {
		if leaf := dialTestTLS(t, address, serverName); leaf.SerialNumber.Cmp(newLeaf.SerialNumber) != 0 {
			t.Fatalf("reloaded %q certificate serial = %s", serverName, leaf.SerialNumber)
		}
	}
	if connection, err := tls.Dial("tcp", address, &tls.Config{InsecureSkipVerify: true, MinVersion: tls.VersionTLS10, MaxVersion: tls.VersionTLS11}); err == nil {
		_ = connection.Close()
		t.Fatal("TLS 1.1 connection succeeded")
	}
}

func TestUnavailableRuntimeCertificateFailsLookup(t *testing.T) {
	router, err := newRouter(config{Sites: []siteConfig{{Hosts: []string{"*"}, Target: "http://localhost:3000"}}})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := newReloadableRouter(router, nil).GetCertificate(nil); err == nil {
		t.Fatal("missing runtime certificate was accepted")
	}
}

func TestReloadableRouterPublishesCompleteGenerations(t *testing.T) {
	firstRouter := &router{}
	secondRouter := &router{}
	firstCertificate := &tls.Certificate{}
	secondCertificate := &tls.Certificate{}
	handler := newReloadableRouter(firstRouter, firstCertificate)
	var invalid atomic.Bool
	var observations atomic.Uint64
	var observedOnce sync.Once
	observed := make(chan struct{})
	start := make(chan struct{})
	stop := make(chan struct{})
	var readers sync.WaitGroup
	for range 4 {
		readers.Add(1)
		go func() {
			defer readers.Done()
			<-start
			for {
				select {
				case <-stop:
					return
				default:
				}
				generation := handler.current.Load()
				if generation.router == firstRouter {
					if generation.certificate != firstCertificate {
						invalid.Store(true)
					}
				} else if generation.router == secondRouter {
					if generation.certificate != secondCertificate {
						invalid.Store(true)
					}
				} else {
					invalid.Store(true)
				}
				observations.Add(1)
				observedOnce.Do(func() { close(observed) })
			}
		}()
	}
	close(start)
	select {
	case <-observed:
	case <-time.After(time.Second):
		t.Fatal("generation reader did not start")
	}
	for index := range 1000 {
		if index%2 == 0 {
			handler.StoreGeneration(secondRouter, secondCertificate)
		} else {
			handler.StoreGeneration(firstRouter, firstCertificate)
		}
	}
	close(stop)
	readers.Wait()
	if observations.Load() == 0 {
		t.Fatal("generation reader observed no generations")
	}
	if invalid.Load() {
		t.Fatal("observed a partially published runtime generation")
	}
}
