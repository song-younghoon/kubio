package main

import (
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
	current atomic.Pointer[router]
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

	initial, err := newRouter(cfg)
	if err != nil {
		log.Fatal(err)
	}

	handler := newReloadableRouter(initial)
	go watchConfig(configPath, cfg.Listen, initialState, handler)
	server := &http.Server{
		Addr:                         cfg.Listen,
		Handler:                      handler,
		DisableGeneralOptionsHandler: true,
	}
	log.Fatal(server.ListenAndServe())
}

func newReloadableRouter(initial *router) *reloadableRouter {
	r := &reloadableRouter{}
	r.current.Store(initial)
	return r
}

func (r *reloadableRouter) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	r.current.Load().ServeHTTP(w, req)
}

func (r *reloadableRouter) Store(next *router) {
	r.current.Store(next)
}

func watchConfig(path, listen string, last fileState, handler *reloadableRouter) {
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

		next, err := newRouter(cfg)
		if err != nil {
			log.Printf("reload %s failed; keeping current config: %v", path, err)
			continue
		}
		handler.Store(next)
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
