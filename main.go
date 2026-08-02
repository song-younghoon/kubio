package main

import (
	"crypto/tls"
	"fmt"
	"log"
	"net/http"
	"os"
	"sync/atomic"
	"time"
)

const (
	configPath         = "kubio.json"
	configPollInterval = time.Second
)

type reloadableRouter struct {
	current atomic.Pointer[runtimeGeneration]
}

type runtimeGeneration struct {
	router      *router
	certificate *tls.Certificate
}

type fileState struct {
	exists  bool
	size    int64
	modTime time.Time
}

func main() {
	initialState, err := statConfig(configPath)
	if err != nil {
		log.Fatal(err)
	}

	cfg, err := loadConfig(configPath)
	if err != nil {
		log.Fatal(err)
	}
	initial, err := buildRuntimeGeneration(cfg)
	if err != nil {
		log.Fatal(err)
	}

	handler := newReloadableRouter(initial.router, initial.certificate)
	go watchConfig(configPath, cfg.Listen, cfg.TLS != nil, initialState, handler)
	server := &http.Server{
		Addr:                         cfg.Listen,
		Handler:                      handler,
		DisableGeneralOptionsHandler: true,
	}
	if cfg.TLS != nil {
		server.TLSConfig = &tls.Config{
			MinVersion:     tls.VersionTLS12,
			GetCertificate: handler.GetCertificate,
		}
		log.Fatal(server.ListenAndServeTLS("", ""))
		return
	}
	log.Fatal(server.ListenAndServe())
}

func newReloadableRouter(initial *router, certificate *tls.Certificate) *reloadableRouter {
	r := &reloadableRouter{}
	r.current.Store(&runtimeGeneration{router: initial, certificate: certificate})
	return r
}

func buildRuntimeGeneration(cfg config) (*runtimeGeneration, error) {
	certificate, err := loadTLSCertificate(cfg.TLS)
	if err != nil {
		return nil, err
	}
	router, err := newRouter(cfg)
	if err != nil {
		return nil, err
	}
	return &runtimeGeneration{router: router, certificate: certificate}, nil
}

func (r *reloadableRouter) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	r.current.Load().router.ServeHTTP(w, req)
}

func (r *reloadableRouter) Store(next *router) {
	r.StoreGeneration(next, r.current.Load().certificate)
}

func (r *reloadableRouter) StoreGeneration(next *router, certificate *tls.Certificate) {
	r.current.Store(&runtimeGeneration{router: next, certificate: certificate})
}

func (r *reloadableRouter) GetCertificate(*tls.ClientHelloInfo) (*tls.Certificate, error) {
	certificate := r.current.Load().certificate
	if certificate == nil {
		return nil, fmt.Errorf("tls certificate is unavailable")
	}
	return certificate, nil
}

func watchConfig(path, listen string, tlsEnabled bool, last fileState, handler *reloadableRouter) {
	ticker := time.NewTicker(configPollInterval)
	defer ticker.Stop()

	for range ticker.C {
		current, err := statConfig(path)
		if err != nil {
			log.Printf("watch %s: %v", path, err)
			continue
		}
		if current == last {
			continue
		}
		last = current

		cfg, err := loadConfig(path)
		if err != nil {
			log.Printf("reload %s failed; keeping current config: %v", path, err)
			continue
		}
		if cfg.Listen != listen {
			log.Printf("reload %s rejected: listen changed to %q; restart required", path, cfg.Listen)
			continue
		}
		if (cfg.TLS != nil) != tlsEnabled {
			log.Printf("reload %s rejected: tls listener mode changed; restart required", path)
			continue
		}
		next, err := buildRuntimeGeneration(cfg)
		if err != nil {
			log.Printf("reload %s failed; keeping current config: %v", path, err)
			continue
		}
		handler.StoreGeneration(next.router, next.certificate)
		log.Printf("reloaded %s", path)
	}
}

func statConfig(path string) (fileState, error) {
	// ponytail: poll mtime and size to keep the request path cheap; use a filesystem watcher if exact event delivery is needed.
	info, err := os.Stat(path)
	if os.IsNotExist(err) {
		return fileState{}, nil
	}
	if err != nil {
		return fileState{}, fmt.Errorf("stat %s: %w", path, err)
	}
	return fileState{exists: true, size: info.Size(), modTime: info.ModTime()}, nil
}
