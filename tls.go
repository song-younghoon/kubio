package main

import (
	"bytes"
	"crypto/tls"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"net/netip"
	"os"
	"strings"
	"unicode"
)

type upstreamTLSConfig struct {
	CAPath  string
	RootCAs *x509.CertPool
	Name    string
}

func loadTLSCertificate(config *tlsConfig) (*tls.Certificate, error) {
	if config == nil {
		return nil, nil
	}
	certificate, err := tls.LoadX509KeyPair(config.Cert, config.Key)
	if err != nil {
		return nil, fmt.Errorf("load tls certificate: %w", err)
	}
	return &certificate, nil
}

func decodeUpstreamTLS(raw rawUpstreamTLS) (*upstreamTLSConfig, error) {
	if !raw.set {
		return nil, nil
	}
	config := &upstreamTLSConfig{}
	if raw.Name.set {
		config.Name = raw.Name.value
	}
	if raw.CA.set {
		config.CAPath = raw.CA.value
	}
	return config, nil
}

func loadUpstreamTLS(config *upstreamTLSConfig) (*upstreamTLSConfig, error) {
	if config == nil || config.CAPath == "" || config.RootCAs != nil {
		return config, nil
	}
	data, err := os.ReadFile(config.CAPath)
	if err != nil {
		return nil, fmt.Errorf("tls.ca %q: read failed", config.CAPath)
	}
	pool, err := parseUpstreamCAPool(data)
	if err != nil {
		return nil, fmt.Errorf("tls.ca %q: %w", config.CAPath, err)
	}
	return &upstreamTLSConfig{CAPath: config.CAPath, RootCAs: pool, Name: config.Name}, nil
}

func parseUpstreamCAPool(data []byte) (*x509.CertPool, error) {
	pool, err := x509.SystemCertPool()
	if err != nil || pool == nil {
		pool = x509.NewCertPool()
	}

	count := 0
	for len(bytes.TrimSpace(data)) > 0 {
		data = bytes.TrimLeftFunc(data, unicode.IsSpace)
		if !bytes.HasPrefix(data, []byte("-----BEGIN ")) {
			return nil, fmt.Errorf("invalid PEM")
		}
		block, rest := pem.Decode(data)
		if block == nil {
			return nil, fmt.Errorf("invalid PEM")
		}
		if block.Type != "CERTIFICATE" {
			return nil, fmt.Errorf("PEM contains a non-certificate block")
		}
		certificate, err := x509.ParseCertificate(block.Bytes)
		if err != nil {
			return nil, fmt.Errorf("invalid certificate")
		}
		pool.AddCert(certificate)
		count++
		data = rest
	}
	if count == 0 {
		return nil, fmt.Errorf("no certificates found")
	}
	return pool, nil
}

func validateUpstreamServerName(name string) error {
	if address, err := netip.ParseAddr(name); err == nil {
		if address.Zone() != "" || strings.Contains(name, "%") {
			return fmt.Errorf("must be a DNS hostname or IP literal without a zone")
		}
		return nil
	}
	if len(name) > 253 || strings.HasSuffix(name, ".") {
		return fmt.Errorf("must be an ASCII DNS hostname or IP literal")
	}
	for _, label := range strings.Split(name, ".") {
		if len(label) == 0 || len(label) > 63 || label[0] == '-' || label[len(label)-1] == '-' {
			return fmt.Errorf("must be an ASCII DNS hostname or IP literal")
		}
		for index := 0; index < len(label); index++ {
			value := label[index]
			if (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z') ||
				(value >= '0' && value <= '9') || value == '-' {
				continue
			}
			return fmt.Errorf("must be an ASCII DNS hostname or IP literal")
		}
	}
	return nil
}
