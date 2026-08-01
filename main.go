package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"strings"
	"sync/atomic"
	"time"
)

const (
	configPath         = "config.json"
	configPollInterval = time.Second
)

type config struct {
	Listen string       `json:"listen"`
	Sites  []siteConfig `json:"sites"`
}

type siteConfig struct {
	Hosts   []string          `json:"hosts"`
	Target  string            `json:"target"`
	Headers map[string]string `json:"headers"`
	Routes  []routeConfig     `json:"routes"`
}

type routeConfig struct {
	Path    string            `json:"path"`
	Target  string            `json:"target"`
	Headers map[string]string `json:"headers"`
	Strip   bool              `json:"strip"`
}

type router struct {
	sites []site
}

type site struct {
	hosts  []string
	target *httputil.ReverseProxy
	routes []route
}

type route struct {
	pattern pathPattern
	proxy   *httputil.ReverseProxy
	strip   bool
}

type pathPattern struct {
	path     string
	wildcard bool
}

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
	log.Fatal(http.ListenAndServe(cfg.Listen, handler))
}

func loadConfig(path string) (config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return config{}, fmt.Errorf("read %s: %w", path, err)
	}

	var cfg config
	if err := json.Unmarshal(data, &cfg); err != nil {
		return config{}, fmt.Errorf("parse %s: %w", path, err)
	}
	if cfg.Listen == "" {
		return config{}, fmt.Errorf("%s must set listen", path)
	}
	if len(cfg.Sites) == 0 {
		return config{}, fmt.Errorf("%s must define at least one site", path)
	}

	return cfg, nil
}

func newRouter(cfg config) (*router, error) {
	r := &router{sites: make([]site, 0, len(cfg.Sites))}

	for siteIndex, siteConfig := range cfg.Sites {
		if len(siteConfig.Hosts) == 0 {
			return nil, fmt.Errorf("sites[%d].hosts must not be empty", siteIndex)
		}
		if siteConfig.Target == "" {
			return nil, fmt.Errorf("sites[%d].target must be set", siteIndex)
		}

		siteHeaders, err := resolveHeaders(siteConfig.Headers)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].headers: %w", siteIndex, err)
		}

		target, err := newProxy(siteConfig.Target, siteHeaders)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].target: %w", siteIndex, err)
		}

		s := site{
			hosts:  siteConfig.Hosts,
			target: target,
			routes: make([]route, 0, len(siteConfig.Routes)),
		}

		for routeIndex, routeConfig := range siteConfig.Routes {
			pattern, err := newPathPattern(routeConfig.Path)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].path: %w", siteIndex, routeIndex, err)
			}
			if routeConfig.Target == "" {
				return nil, fmt.Errorf("sites[%d].routes[%d].target must be set", siteIndex, routeIndex)
			}

			routeHeaders, err := resolveHeaders(routeConfig.Headers)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].headers: %w", siteIndex, routeIndex, err)
			}

			routeTarget, err := newProxy(routeConfig.Target, mergeHeaders(siteHeaders, routeHeaders))
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].target: %w", siteIndex, routeIndex, err)
			}

			s.routes = append(s.routes, route{
				pattern: pattern,
				proxy:   routeTarget,
				strip:   routeConfig.Strip,
			})
		}

		r.sites = append(r.sites, s)
	}

	return r, nil
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
			log.Printf("reload %s: listen changed to %q; keeping %q until restart", path, cfg.Listen, listen)
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

func (r *router) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	host := normalizeHost(req.Host)

	for _, site := range r.sites {
		if !matchesHost(host, site.hosts) {
			continue
		}

		for _, route := range site.routes {
			prefix, ok := route.pattern.match(req.URL.Path)
			if !ok {
				continue
			}
			if route.strip {
				req.URL.Path = stripPathPrefix(req.URL.Path, prefix)
				req.URL.RawPath = ""
			}
			route.proxy.ServeHTTP(w, req)
			return
		}

		site.target.ServeHTTP(w, req)
		return
	}

	http.NotFound(w, req)
}

func newProxy(raw string, headers map[string]string) (*httputil.ReverseProxy, error) {
	target, err := url.Parse(raw)
	if err != nil {
		return nil, err
	}
	if target.Host == "" || (target.Scheme != "http" && target.Scheme != "https") {
		return nil, fmt.Errorf("must be an absolute HTTP(S) URL")
	}
	proxy := httputil.NewSingleHostReverseProxy(target)
	if len(headers) == 0 {
		return proxy, nil
	}

	director := proxy.Director
	proxy.Director = func(req *http.Request) {
		director(req)
		for name, value := range headers {
			if name == "Host" {
				req.Host = value
				continue
			}
			req.Header.Set(name, value)
		}
	}
	return proxy, nil
}

func resolveHeaders(raw map[string]string) (map[string]string, error) {
	if len(raw) == 0 {
		return nil, nil
	}

	headers := make(map[string]string, len(raw))
	for name, value := range raw {
		if name == "" {
			return nil, fmt.Errorf("header name must not be empty")
		}

		name = http.CanonicalHeaderKey(name)
		if _, exists := headers[name]; exists {
			return nil, fmt.Errorf("duplicate header name %q", name)
		}

		value, err := expandEnv(value)
		if err != nil {
			return nil, fmt.Errorf("%q: %w", name, err)
		}
		headers[name] = value
	}
	return headers, nil
}

func mergeHeaders(base, override map[string]string) map[string]string {
	if len(base) == 0 && len(override) == 0 {
		return nil
	}

	headers := make(map[string]string, len(base)+len(override))
	for name, value := range base {
		headers[name] = value
	}
	for name, value := range override {
		headers[name] = value
	}
	return headers
}

func expandEnv(value string) (string, error) {
	var expanded strings.Builder
	for i := 0; i < len(value); {
		if value[i] == '\\' && i+1 < len(value) && (value[i+1] == '\\' || value[i+1] == '$') {
			expanded.WriteByte(value[i+1])
			i += 2
			continue
		}
		if strings.HasPrefix(value[i:], "${") {
			end := strings.IndexByte(value[i+2:], '}')
			if end < 0 {
				return "", fmt.Errorf("unterminated environment variable")
			}

			name := value[i+2 : i+2+end]
			if name == "" {
				return "", fmt.Errorf("environment variable name must not be empty")
			}
			env, ok := os.LookupEnv(name)
			if !ok {
				return "", fmt.Errorf("environment variable %q is not set", name)
			}
			expanded.WriteString(env)
			i += 2 + end + 1
			continue
		}
		expanded.WriteByte(value[i])
		i++
	}
	return expanded.String(), nil
}

func newPathPattern(raw string) (pathPattern, error) {
	if raw == "" || !strings.HasPrefix(raw, "/") {
		return pathPattern{}, fmt.Errorf("must start with /")
	}

	if raw == "/*" {
		return pathPattern{wildcard: true}, nil
	}
	if strings.HasSuffix(raw, "/*") {
		prefix := strings.TrimSuffix(raw, "/*")
		if strings.Contains(prefix, "*") {
			return pathPattern{}, fmt.Errorf("only a trailing /* wildcard is supported")
		}
		return pathPattern{path: prefix, wildcard: true}, nil
	}
	if strings.Contains(raw, "*") {
		return pathPattern{}, fmt.Errorf("only a trailing /* wildcard is supported")
	}

	return pathPattern{path: raw}, nil
}

func (p pathPattern) match(path string) (string, bool) {
	if !p.wildcard {
		return p.path, path == p.path
	}
	if p.path == "" || path == p.path || strings.HasPrefix(path, p.path+"/") {
		return p.path, true
	}
	return "", false
}

func stripPathPrefix(path, prefix string) string {
	if prefix == "" {
		return path
	}

	path = strings.TrimPrefix(path, prefix)
	if path == "" {
		return "/"
	}
	return path
}

func normalizeHost(raw string) string {
	if host, _, err := net.SplitHostPort(raw); err == nil {
		raw = host
	}
	return strings.ToLower(strings.TrimSuffix(strings.Trim(raw, "[]"), "."))
}

func matchesHost(host string, patterns []string) bool {
	for _, pattern := range patterns {
		pattern = normalizeHost(pattern)
		if pattern == "*" || pattern == host {
			return true
		}
		if strings.HasPrefix(pattern, "*.") {
			prefix, ok := strings.CutSuffix(host, pattern[1:])
			if ok && prefix != "" && !strings.Contains(prefix, ".") {
				return true
			}
		}
	}
	return false
}
