package main

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

type clientTLSMaterial struct {
	caPath, serverCertPath, serverKeyPath string
	clientCertPath, clientKeyPath         string
	serverCertificate, clientCertificate  tls.Certificate
	ca                                    *x509.Certificate
}

func newClientTLSMaterial(t testing.TB) clientTLSMaterial {
	t.Helper()
	directory := t.TempDir()
	now := time.Now().Add(-time.Hour)
	caKey := newTestECDSAKey(t)
	caTemplate := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "Kubio Test CA"},
		NotBefore:             now,
		NotAfter:              now.Add(2 * time.Hour),
		BasicConstraintsValid: true,
		IsCA:                  true,
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature,
	}
	caDER := createTestCertificate(t, caTemplate, caTemplate, &caKey.PublicKey, caKey)
	ca, err := x509.ParseCertificate(caDER)
	if err != nil {
		t.Fatal(err)
	}
	caPath := filepath.Join(directory, "ca.pem")
	writePEM(t, caPath, "CERTIFICATE", caDER)

	serverCertPath, serverKeyPath, serverCertificate := writeSignedClientTLSCertificate(t, directory, "server", ca, caKey, []string{"upstream.test"}, []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth})
	clientCertPath, clientKeyPath, clientCertificate := writeSignedClientTLSCertificate(t, directory, "client", ca, caKey, nil, []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth})
	return clientTLSMaterial{
		caPath: caPath, serverCertPath: serverCertPath, serverKeyPath: serverKeyPath,
		clientCertPath: clientCertPath, clientKeyPath: clientKeyPath,
		serverCertificate: serverCertificate, clientCertificate: clientCertificate, ca: ca,
	}
}

func newTestECDSAKey(t testing.TB) *ecdsa.PrivateKey {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return key
}

func createTestCertificate(t testing.TB, template, parent *x509.Certificate, publicKey, signer any) []byte {
	t.Helper()
	der, err := x509.CreateCertificate(rand.Reader, template, parent, publicKey, signer)
	if err != nil {
		t.Fatal(err)
	}
	return der
}

func writeSignedClientTLSCertificate(t testing.TB, directory, name string, ca *x509.Certificate, caKey *ecdsa.PrivateKey, dnsNames []string, usages []x509.ExtKeyUsage) (string, string, tls.Certificate) {
	t.Helper()
	key := newTestECDSAKey(t)
	now := time.Now().Add(-time.Hour)
	template := &x509.Certificate{
		SerialNumber: big.NewInt(int64(len(name) + 10)),
		Subject:      pkix.Name{CommonName: name},
		DNSNames:     dnsNames,
		NotBefore:    now,
		NotAfter:     now.Add(2 * time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:  usages,
	}
	der := createTestCertificate(t, template, ca, &key.PublicKey, caKey)
	keyDER, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	certPath := filepath.Join(directory, name+".crt")
	keyPath := filepath.Join(directory, name+".key")
	writePEM(t, certPath, "CERTIFICATE", der)
	writePEM(t, keyPath, "PRIVATE KEY", keyDER)
	certificate, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		t.Fatal(err)
	}
	return certPath, keyPath, certificate
}

func writePEM(t testing.TB, path, blockType string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, pem.EncodeToMemory(&pem.Block{Type: blockType, Bytes: data}), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestUpstreamClientCertificateAuthenticates(t *testing.T) {
	material := newClientTLSMaterial(t)
	pool := x509.NewCertPool()
	pool.AddCert(material.ca)
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("authenticated"))
	}))
	server.TLS = &tls.Config{
		Certificates: []tls.Certificate{material.serverCertificate},
		ClientAuth:   tls.RequireAndVerifyClientCert,
		ClientCAs:    pool,
		MinVersion:   tls.VersionTLS12,
	}
	server.StartTLS()
	defer server.Close()

	router, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"example.com"},
		Target: server.URL,
		TLS: &upstreamTLSConfig{
			CAPath: material.caPath, Name: "upstream.test",
			CertPath: material.clientCertPath, KeyPath: material.clientKeyPath,
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	defer router.close()

	request := httptest.NewRequest(http.MethodGet, "http://example.com/", nil)
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusOK || response.Body.String() != "authenticated" {
		t.Fatalf("response = %d %q; want authenticated response", response.Code, response.Body.String())
	}
	if len(router.directTransports) != 1 || len(router.directTransports[0].TLSClientConfig.Certificates) != 1 {
		t.Fatalf("client certificates = %#v; want one configured certificate", router.directTransports[0].TLSClientConfig.Certificates)
	}
}

func TestUpstreamClientCertificateCombinedFileAndStrictPEM(t *testing.T) {
	material := newClientTLSMaterial(t)
	certData, err := os.ReadFile(material.clientCertPath)
	if err != nil {
		t.Fatal(err)
	}
	keyData, err := os.ReadFile(material.clientKeyPath)
	if err != nil {
		t.Fatal(err)
	}
	combined := filepath.Join(t.TempDir(), "client.pem")
	if err := os.WriteFile(combined, append(certData, keyData...), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadUpstreamClientCertificate(combined, combined); err != nil {
		t.Fatalf("combined certificate rejected: %v", err)
	}

	for name, data := range map[string][]byte{
		"certificate trailing data":    append(append([]byte(nil), certData...), []byte("secret")...),
		"certificate inter-block data": append(append(append([]byte(nil), certData...), []byte("secret\n")...), certData...),
		"key leading data":             append([]byte("secret\n"), keyData...),
		"key trailing data":            append(append([]byte(nil), keyData...), []byte("secret")...),
		"key invalid block type":       bytes.Replace(keyData, []byte("PRIVATE KEY"), []byte("INVALIDPRIVATE KEY"), 1),
		"certificate wrong block":      append(pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: []byte("bad")}), certData...),
	} {
		t.Run(name, func(t *testing.T) {
			badCert := filepath.Join(t.TempDir(), "cert.pem")
			badKey := filepath.Join(filepath.Dir(badCert), "key.pem")
			if err := os.WriteFile(badCert, data, 0o600); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(badKey, keyData, 0o600); err != nil {
				t.Fatal(err)
			}
			if _, err := loadUpstreamClientCertificate(badCert, badKey); err == nil {
				t.Fatal("invalid client certificate accepted")
			}
		})
	}
}

func TestDirectTLSTransportsDoNotShareChangedMaterial(t *testing.T) {
	material := newClientTLSMaterial(t)
	caPath := filepath.Join(t.TempDir(), "ca.pem")
	writePEM(t, caPath, "CERTIFICATE", material.ca.Raw)

	cache := newDirectTransportCache()
	first, err := cache.get(nil, &upstreamTLSConfig{CAPath: caPath, Name: "upstream.test"})
	if err != nil {
		t.Fatal(err)
	}
	writePEM(t, caPath, "CERTIFICATE", material.serverCertificate.Certificate[0])
	second, err := cache.get(nil, &upstreamTLSConfig{CAPath: caPath, Name: "upstream.test"})
	if err != nil {
		t.Fatal(err)
	}
	if first == second {
		t.Fatal("transport cache reused changed TLS material")
	}
	first.CloseIdleConnections()
	second.CloseIdleConnections()
}

func TestUpstreamClientCertificateConfigAndHTTPValidation(t *testing.T) {
	data := `{"listen":":8080","sites":[{"hosts":["*"],"target":"https://localhost:3000","tls":{"cert":"client.crt","key":"client.key"}}]}`
	cfg, err := decodeConfig([]byte(data))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Sites[0].TLS == nil || cfg.Sites[0].TLS.CertPath != "client.crt" || cfg.Sites[0].TLS.KeyPath != "client.key" {
		t.Fatalf("decoded upstream client TLS = %#v", cfg.Sites[0].TLS)
	}

	_, err = newRouter(config{Sites: []siteConfig{{
		Hosts: []string{"*"}, Target: "http://localhost:3000",
		TLS: &upstreamTLSConfig{CertPath: "/missing/cert", KeyPath: "/missing/key"},
	}}})
	if err == nil {
		t.Fatal("HTTP target accepted upstream client certificate")
	}
}

func BenchmarkDirectTLSTransportCache(b *testing.B) {
	material := newClientTLSMaterial(b)
	config := &upstreamTLSConfig{
		CAPath: material.caPath, Name: "upstream.test",
		CertPath: material.clientCertPath, KeyPath: material.clientKeyPath,
	}
	cache := newDirectTransportCache()
	if _, err := cache.get(nil, config); err != nil {
		b.Fatal(err)
	}
	b.ResetTimer()
	for range b.N {
		if _, err := cache.get(nil, config); err != nil {
			b.Fatal(err)
		}
	}
	b.StopTimer()
	for _, transport := range cache.all {
		transport.CloseIdleConnections()
	}
}
