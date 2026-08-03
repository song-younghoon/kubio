package main

import (
	"bytes"
	"crypto/sha256"
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
	CAPath     string
	RootCAs    *x509.CertPool
	CAHash     [32]byte
	Name       string
	CertPath   string
	KeyPath    string
	ClientCert *tls.Certificate
	CertHash   [32]byte
	KeyHash    [32]byte
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
	if raw.Cert.set {
		config.CertPath = raw.Cert.value
		config.KeyPath = raw.Key.value
	}
	return config, nil
}

func loadUpstreamTLS(config *upstreamTLSConfig) (*upstreamTLSConfig, error) {
	if config == nil {
		return config, nil
	}
	loaded := *config
	changed := false
	if config.CAPath != "" && config.RootCAs == nil {
		data, err := os.ReadFile(config.CAPath)
		if err != nil {
			return nil, fmt.Errorf("tls.ca %q: read failed", config.CAPath)
		}
		pool, err := parseUpstreamCAPool(data)
		if err != nil {
			return nil, fmt.Errorf("tls.ca %q: %w", config.CAPath, err)
		}
		loaded.RootCAs = pool
		loaded.CAHash = sha256.Sum256(data)
		changed = true
	}
	if config.CertPath != "" && config.ClientCert == nil {
		certificate, certHash, keyHash, err := loadUpstreamClientCertificateWithHashes(config.CertPath, config.KeyPath)
		if err != nil {
			return nil, err
		}
		loaded.ClientCert = &certificate
		loaded.CertHash = certHash
		loaded.KeyHash = keyHash
		changed = true
	}
	if !changed {
		return config, nil
	}
	return &loaded, nil
}

func loadUpstreamClientCertificate(certPath, keyPath string) (tls.Certificate, error) {
	certificate, _, _, err := loadUpstreamClientCertificateWithHashes(certPath, keyPath)
	return certificate, err
}

func loadUpstreamClientCertificateWithHashes(certPath, keyPath string) (tls.Certificate, [32]byte, [32]byte, error) {
	certData, err := os.ReadFile(certPath)
	if err != nil {
		return tls.Certificate{}, [32]byte{}, [32]byte{}, fmt.Errorf("tls.cert %q: read failed", certPath)
	}
	certHash := sha256.Sum256(certData)
	var keyHash [32]byte
	var certPEM, keyPEM []byte
	if certPath == keyPath {
		certPEM, keyPEM, err = splitCombinedClientPEM(certData)
		keyHash = certHash
	} else {
		certPEM, err = validateClientCertificatePEM(certData)
		if err == nil {
			keyData, readErr := os.ReadFile(keyPath)
			if readErr != nil {
				return tls.Certificate{}, [32]byte{}, [32]byte{}, fmt.Errorf("tls.key %q: read failed", keyPath)
			}
			keyHash = sha256.Sum256(keyData)
			keyPEM, err = validateClientKeyPEM(keyData)
		}
	}
	if err != nil {
		return tls.Certificate{}, [32]byte{}, [32]byte{}, fmt.Errorf("tls.cert %q/tls.key %q: invalid PEM or certificate/key pair", certPath, keyPath)
	}
	certificate, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return tls.Certificate{}, [32]byte{}, [32]byte{}, fmt.Errorf("tls.cert %q/tls.key %q: invalid certificate/key pair", certPath, keyPath)
	}
	return certificate, certHash, keyHash, nil
}

func validateClientCertificatePEM(data []byte) ([]byte, error) {
	remaining := data
	var encoded bytes.Buffer
	count := 0
	for {
		remaining = bytes.TrimLeftFunc(remaining, unicode.IsSpace)
		if len(remaining) == 0 {
			break
		}
		if !bytes.HasPrefix(remaining, []byte("-----BEGIN ")) {
			return nil, fmt.Errorf("invalid certificate PEM")
		}
		block, rest := pem.Decode(remaining)
		if block == nil || block.Type != "CERTIFICATE" {
			return nil, fmt.Errorf("invalid certificate PEM")
		}
		if _, err := x509.ParseCertificate(block.Bytes); err != nil {
			return nil, fmt.Errorf("invalid certificate PEM")
		}
		encoded.Write(pem.EncodeToMemory(block))
		count++
		remaining = rest
	}
	if count == 0 {
		return nil, fmt.Errorf("invalid certificate PEM")
	}
	return encoded.Bytes(), nil
}

func validateClientKeyPEM(data []byte) ([]byte, error) {
	data = bytes.TrimLeftFunc(data, unicode.IsSpace)
	if !bytes.HasPrefix(data, []byte("-----BEGIN ")) {
		return nil, fmt.Errorf("invalid private key PEM")
	}
	block, rest := pem.Decode(data)
	if block == nil || !validClientKeyBlockType(block.Type) || len(bytes.TrimSpace(rest)) != 0 {
		return nil, fmt.Errorf("invalid private key PEM")
	}
	return pem.EncodeToMemory(block), nil
}

func splitCombinedClientPEM(data []byte) ([]byte, []byte, error) {
	remaining := data
	var certificates, key []byte
	seenKey := false
	certCount := 0
	keyCount := 0
	for {
		remaining = bytes.TrimLeftFunc(remaining, unicode.IsSpace)
		if len(remaining) == 0 {
			break
		}
		if !bytes.HasPrefix(remaining, []byte("-----BEGIN ")) {
			return nil, nil, fmt.Errorf("invalid combined PEM")
		}
		block, rest := pem.Decode(remaining)
		if block == nil {
			return nil, nil, fmt.Errorf("invalid combined PEM")
		}
		if validClientKeyBlockType(block.Type) {
			if seenKey || keyCount > 0 {
				return nil, nil, fmt.Errorf("invalid combined PEM")
			}
			seenKey = true
			keyCount++
			key = pem.EncodeToMemory(block)
		} else {
			if seenKey || block.Type != "CERTIFICATE" {
				return nil, nil, fmt.Errorf("invalid combined PEM")
			}
			if _, err := x509.ParseCertificate(block.Bytes); err != nil {
				return nil, nil, fmt.Errorf("invalid combined PEM")
			}
			certificates = append(certificates, pem.EncodeToMemory(block)...)
			certCount++
		}
		remaining = rest
	}
	if certCount == 0 || keyCount != 1 {
		return nil, nil, fmt.Errorf("invalid combined PEM")
	}
	return certificates, key, nil
}

func validClientKeyBlockType(blockType string) bool {
	return blockType == "PRIVATE KEY" || strings.HasSuffix(blockType, " PRIVATE KEY")
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
