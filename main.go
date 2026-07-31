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
)

type config struct {
	Listen string       `json:"listen"`
	Sites  []siteConfig `json:"sites"`
}

type siteConfig struct {
	Hosts  []string      `json:"hosts"`
	Target string        `json:"target"`
	Routes []routeConfig `json:"routes"`
}

type routeConfig struct {
	Path   string `json:"path"`
	Target string `json:"target"`
	Strip  bool   `json:"strip"`
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
	exact    string
	prefix   string
	wildcard bool
}

func main() {
	cfg, err := loadConfig("config.json")
	if err != nil {
		log.Fatal(err)
	}

	handler, err := newRouter(cfg)
	if err != nil {
		log.Fatal(err)
	}

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

func newRouter(cfg config) (http.Handler, error) {
	r := &router{sites: make([]site, 0, len(cfg.Sites))}

	for siteIndex, siteConfig := range cfg.Sites {
		if len(siteConfig.Hosts) == 0 {
			return nil, fmt.Errorf("sites[%d].hosts must not be empty", siteIndex)
		}
		if siteConfig.Target == "" {
			return nil, fmt.Errorf("sites[%d].target must be set", siteIndex)
		}

		target, err := parseTarget(siteConfig.Target)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].target: %w", siteIndex, err)
		}

		s := site{
			hosts:  siteConfig.Hosts,
			target: httputil.NewSingleHostReverseProxy(target),
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

			routeTarget, err := parseTarget(routeConfig.Target)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].target: %w", siteIndex, routeIndex, err)
			}

			s.routes = append(s.routes, route{
				pattern: pattern,
				proxy:   httputil.NewSingleHostReverseProxy(routeTarget),
				strip:   routeConfig.Strip,
			})
		}

		r.sites = append(r.sites, s)
	}

	return r, nil
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

func parseTarget(raw string) (*url.URL, error) {
	target, err := url.Parse(raw)
	if err != nil {
		return nil, err
	}
	if target.Host == "" || (target.Scheme != "http" && target.Scheme != "https") {
		return nil, fmt.Errorf("must be an absolute HTTP(S) URL")
	}
	return target, nil
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
		return pathPattern{prefix: prefix, wildcard: true}, nil
	}
	if strings.Contains(raw, "*") {
		return pathPattern{}, fmt.Errorf("only a trailing /* wildcard is supported")
	}

	return pathPattern{exact: raw}, nil
}

func (p pathPattern) match(path string) (string, bool) {
	if !p.wildcard {
		return p.exact, path == p.exact
	}
	if p.prefix == "" || path == p.prefix || strings.HasPrefix(path, p.prefix+"/") {
		return p.prefix, true
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
