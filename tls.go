package main

import (
	"crypto/tls"
	"fmt"
)

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
