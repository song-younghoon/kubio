package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httputil"
	"net/netip"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const (
	configPath         = "kubio.json"
	configPollInterval = time.Second

	proxyDialTimeout         = 5 * time.Second
	proxyResponseHeaderLimit = 30 * time.Second
	proxyBufferSize          = 32 * 1024
	proxyMaxIdleConnsPerHost = 32
	routeIndexThreshold      = 8
)

var proxyTransport = func() *http.Transport {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.DialContext = (&net.Dialer{Timeout: proxyDialTimeout}).DialContext
	transport.TLSHandshakeTimeout = proxyDialTimeout
	transport.ResponseHeaderTimeout = proxyResponseHeaderLimit
	transport.MaxIdleConnsPerHost = proxyMaxIdleConnsPerHost
	return transport
}()

var proxyBuffers proxyBufferPool

type proxyBufferPool struct {
	pool sync.Pool
}

func (p *proxyBufferPool) Get() []byte {
	if buffer := p.pool.Get(); buffer != nil {
		return buffer.([]byte)
	}
	return make([]byte, proxyBufferSize)
}

func (p *proxyBufferPool) Put(buffer []byte) {
	if cap(buffer) == proxyBufferSize {
		p.pool.Put(buffer[:proxyBufferSize])
	}
}

type config struct {
	Listen       string       `json:"listen"`
	TrustProxies []string     `json:"trustProxies"`
	Sites        []siteConfig `json:"sites"`
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

type rawConfig struct {
	Listen       *string          `json:"listen"`
	TrustProxies stringArray      `json:"trustProxies"`
	Sites        *[]rawSiteConfig `json:"sites"`
}

type rawSiteConfig struct {
	Hosts   *stringArray `json:"hosts"`
	Target  *string      `json:"target"`
	Headers headerMap    `json:"headers"`
	Routes  rawRoutes    `json:"routes"`
}

type rawRouteConfig struct {
	Path    *string        `json:"path"`
	Target  optionalString `json:"target"`
	Headers headerMap      `json:"headers"`
	Strip   strictBool     `json:"strip"`
}

type headerMap map[string]string

type stringArray []string
type rawRoutes []rawRouteConfig

type optionalString struct {
	set   bool
	value string
}

type strictBool bool

func (a *stringArray) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be an array")
	}

	var raw []json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return fmt.Errorf("must be an array of strings: %w", err)
	}
	values := make(stringArray, len(raw))
	for index, rawValue := range raw {
		if bytes.Equal(bytes.TrimSpace(rawValue), []byte("null")) {
			return fmt.Errorf("item %d must be a string", index)
		}
		if err := json.Unmarshal(rawValue, &values[index]); err != nil {
			return fmt.Errorf("item %d must be a string: %w", index, err)
		}
	}
	*a = values
	return nil
}

func (r *rawRoutes) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be an array")
	}
	var routes []rawRouteConfig
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&routes); err != nil {
		return fmt.Errorf("must be an array: %w", err)
	}
	*r = routes
	return nil
}

func (s *optionalString) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be a string")
	}
	var value string
	if err := json.Unmarshal(data, &value); err != nil {
		return fmt.Errorf("must be a string: %w", err)
	}
	s.set = true
	s.value = value
	return nil
}

func (b *strictBool) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be a boolean")
	}
	var value bool
	if err := json.Unmarshal(data, &value); err != nil {
		return fmt.Errorf("must be a boolean: %w", err)
	}
	*b = strictBool(value)
	return nil
}

type router struct {
	sites         []site
	trustProxies  []netip.Prefix
	exactHosts    map[string]int
	wildcardHosts map[string]int
	starSite      int
}

type site struct {
	hosts          []hostPattern
	target         *httputil.ReverseProxy
	routes         []route
	exactRoutes    map[string]int
	wildcardRoutes map[string]int
}

type route struct {
	pattern pathPattern
	proxy   *httputil.ReverseProxy
	strip   bool
}

type pathPattern struct {
	path     string
	wildcard bool
	depth    int
}

type hostPattern struct {
	value    string
	wildcard bool
	suffix   string
}

type hostScore struct {
	kind         int
	suffixLength int
}

type routeCandidate struct {
	route  *route
	prefix string
	exact  bool
	depth  int
	index  int
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

	cfg, err := decodeConfig(data)
	if err != nil {
		return config{}, fmt.Errorf("parse %s: %w", path, err)
	}
	return cfg, nil
}

func decodeConfig(data []byte) (config, error) {
	if err := validateJSON(data); err != nil {
		return config{}, err
	}

	var raw rawConfig
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&raw); err != nil {
		return config{}, err
	}
	if raw.Listen == nil || strings.TrimSpace(*raw.Listen) == "" {
		return config{}, fmt.Errorf("listen must be set")
	}
	if raw.Sites == nil || len(*raw.Sites) == 0 {
		return config{}, fmt.Errorf("sites must define at least one site")
	}

	cfg := config{
		Listen:       *raw.Listen,
		TrustProxies: append([]string(nil), raw.TrustProxies...),
		Sites:        make([]siteConfig, len(*raw.Sites)),
	}
	for siteIndex, rawSite := range *raw.Sites {
		if rawSite.Hosts == nil || len(*rawSite.Hosts) == 0 {
			return config{}, fmt.Errorf("sites[%d].hosts must not be empty", siteIndex)
		}
		if rawSite.Target == nil || *rawSite.Target == "" {
			return config{}, fmt.Errorf("sites[%d].target must be set", siteIndex)
		}

		site := siteConfig{
			Hosts:   append([]string(nil), (*rawSite.Hosts)...),
			Target:  *rawSite.Target,
			Headers: map[string]string(rawSite.Headers),
			Routes:  make([]routeConfig, len(rawSite.Routes)),
		}
		for routeIndex, rawRoute := range rawSite.Routes {
			if rawRoute.Path == nil || *rawRoute.Path == "" {
				return config{}, fmt.Errorf("sites[%d].routes[%d].path must be set", siteIndex, routeIndex)
			}
			if rawRoute.Target.set && rawRoute.Target.value == "" {
				return config{}, fmt.Errorf("sites[%d].routes[%d].target must not be empty", siteIndex, routeIndex)
			}
			route := routeConfig{
				Path:    *rawRoute.Path,
				Headers: map[string]string(rawRoute.Headers),
				Strip:   bool(rawRoute.Strip),
			}
			if rawRoute.Target.set {
				route.Target = rawRoute.Target.value
			}
			site.Routes[routeIndex] = route
		}
		cfg.Sites[siteIndex] = site
	}

	return cfg, nil
}

func validateJSON(data []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	if err := consumeJSONValue(decoder, jsonRoot); err != nil {
		return err
	}
	if _, err := decoder.Token(); err != io.EOF {
		if err == nil {
			return fmt.Errorf("multiple JSON values")
		}
		return err
	}
	return nil
}

type jsonSchema uint8

const (
	jsonAny jsonSchema = iota
	jsonRoot
	jsonSites
	jsonSite
	jsonRoutes
	jsonRoute
	jsonHeaders
)

func consumeJSONValue(decoder *json.Decoder, schema jsonSchema) error {
	token, err := decoder.Token()
	if err != nil {
		return err
	}

	delim, ok := token.(json.Delim)
	if !ok {
		return nil
	}

	switch delim {
	case '{':
		seen := make(map[string]struct{})
		for decoder.More() {
			keyToken, err := decoder.Token()
			if err != nil {
				return err
			}
			key, ok := keyToken.(string)
			if !ok {
				return fmt.Errorf("object key is not a string")
			}
			if _, exists := seen[key]; exists {
				return fmt.Errorf("duplicate JSON key %q", key)
			}
			seen[key] = struct{}{}
			childSchema, err := childJSONSchema(schema, key)
			if err != nil {
				return err
			}
			if err := consumeJSONValue(decoder, childSchema); err != nil {
				return err
			}
		}
		end, err := decoder.Token()
		if err != nil {
			return err
		}
		if end != json.Delim('}') {
			return fmt.Errorf("invalid JSON object")
		}
	case '[':
		childSchema := arrayItemJSONSchema(schema)
		for decoder.More() {
			if err := consumeJSONValue(decoder, childSchema); err != nil {
				return err
			}
		}
		end, err := decoder.Token()
		if err != nil {
			return err
		}
		if end != json.Delim(']') {
			return fmt.Errorf("invalid JSON array")
		}
	default:
		return fmt.Errorf("invalid JSON delimiter %q", delim)
	}
	return nil
}

func childJSONSchema(schema jsonSchema, key string) (jsonSchema, error) {
	switch schema {
	case jsonRoot:
		switch key {
		case "listen", "trustProxies":
			return jsonAny, nil
		case "sites":
			return jsonSites, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonSite:
		switch key {
		case "hosts", "target":
			return jsonAny, nil
		case "headers":
			return jsonHeaders, nil
		case "routes":
			return jsonRoutes, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonRoute:
		switch key {
		case "path", "target", "strip":
			return jsonAny, nil
		case "headers":
			return jsonHeaders, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	default:
		return jsonAny, nil
	}
}

func arrayItemJSONSchema(schema jsonSchema) jsonSchema {
	switch schema {
	case jsonSites:
		return jsonSite
	case jsonRoutes:
		return jsonRoute
	default:
		return jsonAny
	}
}

func (m *headerMap) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return fmt.Errorf("headers must be an object: %w", err)
	}
	if raw == nil {
		return fmt.Errorf("headers must be an object")
	}

	headers := make(headerMap, len(raw))
	for name, rawValue := range raw {
		if bytes.Equal(bytes.TrimSpace(rawValue), []byte("null")) {
			return fmt.Errorf("header %q value must be a string", name)
		}
		var value string
		if err := json.Unmarshal(rawValue, &value); err != nil {
			return fmt.Errorf("header %q value must be a string", name)
		}
		headers[name] = value
	}
	*m = headers
	return nil
}

func newRouter(cfg config) (*router, error) {
	if len(cfg.Sites) == 0 {
		return nil, fmt.Errorf("sites must define at least one site")
	}
	trustProxies, err := parseTrustedProxies(cfg.TrustProxies)
	if err != nil {
		return nil, fmt.Errorf("trustProxies: %w", err)
	}

	r := &router{
		sites:         make([]site, 0, len(cfg.Sites)),
		trustProxies:  trustProxies,
		exactHosts:    make(map[string]int),
		wildcardHosts: make(map[string]int),
		starSite:      -1,
	}

	for siteIndex, siteConfig := range cfg.Sites {
		if len(siteConfig.Hosts) == 0 {
			return nil, fmt.Errorf("sites[%d].hosts must not be empty", siteIndex)
		}
		if siteConfig.Target == "" {
			return nil, fmt.Errorf("sites[%d].target must be set", siteIndex)
		}

		hosts, err := newHostPatterns(siteConfig.Hosts)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].hosts: %w", siteIndex, err)
		}
		siteHeaders, err := resolveHeaders(siteConfig.Headers)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].headers: %w", siteIndex, err)
		}

		target, err := newProxy(siteConfig.Target, siteHeaders, trustProxies)
		if err != nil {
			return nil, fmt.Errorf("sites[%d].target: %w", siteIndex, err)
		}

		s := site{
			hosts:  hosts,
			target: target,
			routes: make([]route, 0, len(siteConfig.Routes)),
		}

		for routeIndex, routeConfig := range siteConfig.Routes {
			pattern, err := newPathPattern(routeConfig.Path)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].path: %w", siteIndex, routeIndex, err)
			}

			routeHeaders, err := resolveHeaders(routeConfig.Headers)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].headers: %w", siteIndex, routeIndex, err)
			}

			targetURL := siteConfig.Target
			if routeConfig.Target != "" {
				targetURL = routeConfig.Target
			}
			routeTarget, err := newProxy(targetURL, mergeHeaders(siteHeaders, routeHeaders), trustProxies)
			if err != nil {
				return nil, fmt.Errorf("sites[%d].routes[%d].target: %w", siteIndex, routeIndex, err)
			}

			s.routes = append(s.routes, route{
				pattern: pattern,
				proxy:   routeTarget,
				strip:   routeConfig.Strip,
			})
		}
		if len(s.routes) > routeIndexThreshold {
			s.buildRouteIndex()
		}

		siteIndex := len(r.sites)
		r.sites = append(r.sites, s)
		for _, host := range hosts {
			switch {
			case host.value == "*":
				if r.starSite < 0 {
					r.starSite = siteIndex
				}
			case host.wildcard:
				if _, exists := r.wildcardHosts[host.suffix]; !exists {
					r.wildcardHosts[host.suffix] = siteIndex
				}
			default:
				if _, exists := r.exactHosts[host.value]; !exists {
					r.exactHosts[host.value] = siteIndex
				}
			}
		}
	}

	return r, nil
}

func parseTrustedProxies(raw []string) ([]netip.Prefix, error) {
	prefixes := make([]netip.Prefix, 0, len(raw))
	for index, value := range raw {
		if !strings.Contains(value, "/") {
			return nil, fmt.Errorf("trustProxies[%d] must be a CIDR", index)
		}
		prefix, err := netip.ParsePrefix(value)
		if err != nil {
			return nil, fmt.Errorf("trustProxies[%d] %q: %w", index, value, err)
		}
		prefixes = append(prefixes, prefix)
	}
	return prefixes, nil
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

func (r *router) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	host := normalizeHost(req.Host)
	selected := r.selectSite(host)
	if selected == nil {
		http.NotFound(w, req)
		return
	}

	selectedRoute := selected.selectRoute(req.URL.Path)

	if selectedRoute.route == nil {
		selected.target.ServeHTTP(w, req)
		return
	}
	if selectedRoute.route.strip {
		stripRequestPath(req, selectedRoute.prefix)
	}
	selectedRoute.route.proxy.ServeHTTP(w, req)
}

func (s *site) buildRouteIndex() {
	s.exactRoutes = make(map[string]int, len(s.routes))
	s.wildcardRoutes = make(map[string]int, len(s.routes))
	for index := range s.routes {
		pattern := s.routes[index].pattern
		if pattern.wildcard {
			s.wildcardRoutes[pattern.path] = index
		} else {
			s.exactRoutes[pattern.path] = index
		}
	}
}

func (s *site) selectRoute(path string) routeCandidate {
	if s.exactRoutes == nil {
		var selected routeCandidate
		for index := range s.routes {
			prefix, ok := s.routes[index].pattern.match(path)
			if !ok {
				continue
			}
			candidate := s.routeCandidate(index)
			candidate.prefix = prefix
			if selected.route == nil || betterRoute(candidate, selected) {
				selected = candidate
			}
		}
		return selected
	}

	if index, ok := s.exactRoutes[path]; ok {
		return s.routeCandidate(index)
	}

	var selected routeCandidate
	if index, ok := s.wildcardRoutes[""]; ok {
		selected = s.routeCandidate(index)
	}
	for index := 1; index < len(path); index++ {
		if path[index] != '/' {
			continue
		}
		if routeIndex, ok := s.wildcardRoutes[path[:index]]; ok {
			candidate := s.routeCandidate(routeIndex)
			if selected.route == nil || betterRoute(candidate, selected) {
				selected = candidate
			}
		}
	}
	if routeIndex, ok := s.wildcardRoutes[path]; ok {
		candidate := s.routeCandidate(routeIndex)
		if selected.route == nil || betterRoute(candidate, selected) {
			selected = candidate
		}
	}
	return selected
}

func (s *site) routeCandidate(index int) routeCandidate {
	route := &s.routes[index]
	return routeCandidate{
		route:  route,
		prefix: route.pattern.path,
		exact:  !route.pattern.wildcard,
		depth:  route.pattern.depth,
		index:  index,
	}
}

func (r *router) selectSite(host string) *site {
	if len(r.sites) == 1 {
		if _, ok := bestHostMatch(host, r.sites[0].hosts); ok {
			return &r.sites[0]
		}
		return nil
	}
	if r.exactHosts == nil && r.wildcardHosts == nil {
		var selected *site
		var selectedScore hostScore
		for index := range r.sites {
			score, ok := bestHostMatch(host, r.sites[index].hosts)
			if ok && (selected == nil || betterHostScore(score, selectedScore)) {
				selected = &r.sites[index]
				selectedScore = score
			}
		}
		return selected
	}
	if index, ok := r.exactHosts[host]; ok {
		return &r.sites[index]
	}
	if dot := strings.IndexByte(host, '.'); dot > 0 && dot+1 < len(host) {
		if index, ok := r.wildcardHosts[host[dot+1:]]; ok {
			return &r.sites[index]
		}
	}
	if r.starSite >= 0 {
		return &r.sites[r.starSite]
	}
	return nil
}

func betterRoute(candidate, current routeCandidate) bool {
	if candidate.exact != current.exact {
		return candidate.exact
	}
	if candidate.depth != current.depth {
		return candidate.depth > current.depth
	}
	return candidate.index > current.index
}

func newProxy(raw string, headers map[string]string, trustProxies []netip.Prefix) (*httputil.ReverseProxy, error) {
	target, err := parseTarget(raw)
	if err != nil {
		return nil, err
	}

	proxy := &httputil.ReverseProxy{
		Rewrite: func(request *httputil.ProxyRequest) {
			request.SetURL(target)
			request.Out.URL.RawQuery = request.In.URL.RawQuery
			request.Out.Host = request.In.Host
			setForwardedHeaders(request.In, request.Out, trustProxies)
			for name, value := range headers {
				if name == "Host" {
					request.Out.Host = value
					continue
				}
				request.Out.Header.Set(name, value)
			}
		},
		Transport:    proxyTransport,
		BufferPool:   &proxyBuffers,
		ErrorHandler: proxyErrorHandler,
	}
	return proxy, nil
}

func parseTarget(raw string) (*url.URL, error) {
	target, err := url.Parse(raw)
	if err != nil {
		return nil, errors.New("invalid URL")
	}
	if !strings.EqualFold(target.Scheme, "http") && !strings.EqualFold(target.Scheme, "https") {
		return nil, fmt.Errorf("must be an absolute HTTP(S) URL")
	}
	if target.Hostname() == "" {
		return nil, fmt.Errorf("must include a hostname")
	}
	if target.User != nil {
		return nil, fmt.Errorf("userinfo is not allowed")
	}
	if err := validateTargetPort(target); err != nil {
		return nil, err
	}
	if target.Path != "" || target.RawPath != "" || target.RawQuery != "" || target.ForceQuery ||
		strings.Contains(raw, "#") ||
		target.Fragment != "" || target.RawFragment != "" || target.Opaque != "" {
		return nil, fmt.Errorf("path, query, fragment, and userinfo are not allowed")
	}
	target.Scheme = strings.ToLower(target.Scheme)
	return target, nil
}

func validateTargetPort(target *url.URL) error {
	port := target.Port()
	if port == "" && strings.Contains(target.Host, ":") && !strings.HasSuffix(target.Host, "]") {
		return fmt.Errorf("invalid port")
	}
	if port == "" {
		return nil
	}
	return validatePort(port)
}

func validatePort(port string) error {
	if port == "" {
		return fmt.Errorf("invalid port")
	}
	for i := 0; i < len(port); i++ {
		if port[i] < '0' || port[i] > '9' {
			return fmt.Errorf("invalid port")
		}
	}
	value, err := strconv.Atoi(port)
	if err != nil || value < 0 || value > 65535 {
		return fmt.Errorf("invalid port")
	}
	return nil
}

func proxyErrorHandler(w http.ResponseWriter, _ *http.Request, err error) {
	status := http.StatusBadGateway
	var timeout net.Error
	if errors.As(err, &timeout) && timeout.Timeout() || errors.Is(err, context.DeadlineExceeded) {
		status = http.StatusGatewayTimeout
	}
	http.Error(w, http.StatusText(status), status)
}

func setForwardedHeaders(in, out *http.Request, trustProxies []netip.Prefix) {
	peer, peerText := peerAddress(in.RemoteAddr)
	trusted := peer.IsValid() && isTrustedProxy(peer, trustProxies)

	if trusted {
		prior := strings.Join(in.Header.Values("X-Forwarded-For"), ", ")
		if peerText != "" {
			if prior != "" {
				prior += ", "
			}
			prior += peerText
		}
		if prior == "" {
			out.Header.Del("X-Forwarded-For")
		} else {
			out.Header.Set("X-Forwarded-For", prior)
		}

		proto, present := firstHeaderValue(in.Header, "X-Forwarded-Proto")
		if !present {
			proto = requestProtocol(in)
		}
		out.Header.Set("X-Forwarded-Proto", proto)

		host, present := firstHeaderValue(in.Header, "X-Forwarded-Host")
		if !present {
			host = in.Host
		}
		out.Header.Set("X-Forwarded-Host", host)
		return
	}

	if peerText == "" {
		out.Header.Del("X-Forwarded-For")
	} else {
		out.Header.Set("X-Forwarded-For", peerText)
	}
	out.Header.Set("X-Forwarded-Proto", requestProtocol(in))
	out.Header.Set("X-Forwarded-Host", in.Host)
}

func isTrustedProxy(address netip.Addr, prefixes []netip.Prefix) bool {
	for _, prefix := range prefixes {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}

func peerAddress(remote string) (netip.Addr, string) {
	host := remote
	if parsedHost, _, err := net.SplitHostPort(remote); err == nil {
		host = parsedHost
	}
	host = strings.Trim(host, "[]")
	address, err := netip.ParseAddr(host)
	if err != nil {
		return netip.Addr{}, ""
	}
	address = address.WithZone("")
	return address, address.String()
}

func firstHeaderValue(headers http.Header, name string) (string, bool) {
	values := headers.Values(name)
	if len(values) == 0 {
		return "", false
	}
	return values[0], true
}

func requestProtocol(req *http.Request) string {
	if req.TLS != nil {
		return "https"
	}
	return "http"
}

func resolveHeaders(raw map[string]string) (map[string]string, error) {
	if len(raw) == 0 {
		return nil, nil
	}

	headers := make(map[string]string, len(raw))
	for name, value := range raw {
		if !validHeaderName(name) {
			return nil, fmt.Errorf("invalid header name %q", name)
		}

		name = http.CanonicalHeaderKey(name)
		if _, exists := headers[name]; exists {
			return nil, fmt.Errorf("duplicate header name %q", name)
		}
		if restrictedHeader(name) {
			return nil, fmt.Errorf("header %q is managed by the proxy", name)
		}

		value, err := expandEnv(value)
		if err != nil {
			return nil, fmt.Errorf("%q: %w", name, err)
		}
		if !validHeaderValue(value) {
			return nil, fmt.Errorf("header %q contains invalid control characters", name)
		}
		headers[name] = value
	}
	return headers, nil
}

func validHeaderName(name string) bool {
	if name == "" {
		return false
	}
	for i := 0; i < len(name); i++ {
		if !validTokenByte(name[i]) {
			return false
		}
	}
	return true
}

func validTokenByte(value byte) bool {
	if value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9' {
		return true
	}
	switch value {
	case '!', '#', '$', '%', '&', '\'', '+', '-', '.', '^', '_', '\x60', '|', '~':
		return true
	default:
		return false
	}
}

func validHeaderValue(value string) bool {
	for i := 0; i < len(value); i++ {
		if value[i] == '\r' || value[i] == '\n' || value[i] == 0x7f ||
			value[i] < 0x20 && value[i] != '\t' {
			return false
		}
	}
	return true
}

func restrictedHeader(name string) bool {
	switch strings.ToLower(name) {
	case "connection", "proxy-connection", "keep-alive", "transfer-encoding",
		"te", "trailer", "upgrade", "content-length":
		return true
	default:
		return false
	}
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
		if value[i] == '\\' {
			if i+1 < len(value) && value[i+1] == '\\' {
				expanded.WriteByte('\\')
				i += 2
				continue
			}
			if i+2 < len(value) && value[i+1] == '$' && value[i+2] == '{' {
				expanded.WriteString("${")
				i += 3
				continue
			}
		}
		if strings.HasPrefix(value[i:], "${") {
			end := strings.IndexByte(value[i+2:], '}')
			if end < 0 {
				return "", fmt.Errorf("unterminated environment variable")
			}

			name := value[i+2 : i+2+end]
			if !validEnvironmentName(name) {
				return "", fmt.Errorf("invalid environment variable name %q", name)
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

func validEnvironmentName(name string) bool {
	if name == "" || !isEnvironmentNameStart(name[0]) {
		return false
	}
	for i := 1; i < len(name); i++ {
		if !isEnvironmentNameStart(name[i]) && (name[i] < '0' || name[i] > '9') {
			return false
		}
	}
	return true
}

func isEnvironmentNameStart(value byte) bool {
	return value == '_' || value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z'
}

func newPathPattern(raw string) (pathPattern, error) {
	if raw == "" || !strings.HasPrefix(raw, "/") {
		return pathPattern{}, fmt.Errorf("must start with /")
	}
	if strings.IndexFunc(raw, func(r rune) bool {
		return r == '?' || r == '#' || r < 0x20 || r == 0x7f
	}) >= 0 {
		return pathPattern{}, fmt.Errorf("must be a path without query or fragment")
	}

	if raw == "/*" {
		return pathPattern{wildcard: true}, nil
	}
	if strings.HasSuffix(raw, "/*") {
		prefix := strings.TrimSuffix(raw, "/*")
		if strings.Contains(prefix, "*") {
			return pathPattern{}, fmt.Errorf("only a trailing /* wildcard is supported")
		}
		return pathPattern{path: prefix, wildcard: true, depth: pathDepth(prefix)}, nil
	}
	if strings.Contains(raw, "*") {
		return pathPattern{}, fmt.Errorf("only a trailing /* wildcard is supported")
	}

	return pathPattern{path: raw, depth: pathDepth(raw)}, nil
}

func pathDepth(path string) int {
	if path == "" || path == "/" {
		return 0
	}
	return strings.Count(strings.Trim(path, "/"), "/") + 1
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

func stripRequestPath(req *http.Request, prefix string) {
	rawPath := req.URL.RawPath
	req.URL.Path = stripPathPrefix(req.URL.Path, prefix)
	if rawPath == "" || prefix == "" {
		return
	}
	stripped, ok := stripEscapedPathPrefix(rawPath, prefix)
	if !ok {
		req.URL.RawPath = ""
		return
	}
	req.URL.RawPath = stripped
}

func stripEscapedPathPrefix(rawPath, prefix string) (string, bool) {
	decodedIndex := 0
	for index := 0; index < len(rawPath); {
		value, next, ok := escapedPathByte(rawPath, index)
		if !ok || decodedIndex >= len(prefix) || value != prefix[decodedIndex] {
			return "", false
		}
		decodedIndex++
		index = next
		if decodedIndex == len(prefix) {
			return rawPath[index:], true
		}
	}
	return "", false
}

func escapedPathByte(rawPath string, index int) (byte, int, bool) {
	if rawPath[index] != '%' {
		return rawPath[index], index + 1, true
	}
	if index+2 >= len(rawPath) {
		return 0, 0, false
	}
	high, ok := hexDigit(rawPath[index+1])
	if !ok {
		return 0, 0, false
	}
	low, ok := hexDigit(rawPath[index+2])
	if !ok {
		return 0, 0, false
	}
	return high<<4 | low, index + 3, true
}

func hexDigit(value byte) (byte, bool) {
	switch {
	case value >= '0' && value <= '9':
		return value - '0', true
	case value >= 'a' && value <= 'f':
		return value - 'a' + 10, true
	case value >= 'A' && value <= 'F':
		return value - 'A' + 10, true
	default:
		return 0, false
	}
}

func newHostPatterns(raw []string) ([]hostPattern, error) {
	patterns := make([]hostPattern, 0, len(raw))
	for index, value := range raw {
		pattern, err := newHostPattern(value)
		if err != nil {
			return nil, fmt.Errorf("[%d] %w", index, err)
		}
		patterns = append(patterns, pattern)
	}
	return patterns, nil
}

func newHostPattern(raw string) (hostPattern, error) {
	if raw == "" || strings.TrimSpace(raw) != raw {
		return hostPattern{}, fmt.Errorf("host pattern must not be empty or contain whitespace")
	}

	value, err := normalizeConfiguredHost(raw)
	if err != nil {
		return hostPattern{}, err
	}
	if value == "*" {
		return hostPattern{value: value}, nil
	}
	if value == "" {
		return hostPattern{}, fmt.Errorf("host pattern must not be empty")
	}

	if strings.HasPrefix(value, "*.") {
		suffix := strings.TrimPrefix(value, "*.")
		if strings.Contains(suffix, "*") {
			return hostPattern{}, fmt.Errorf("wildcard host may contain only a leading *.")
		}
		if err := validateHostName(suffix); err != nil {
			return hostPattern{}, err
		}
		return hostPattern{value: value, wildcard: true, suffix: suffix}, nil
	}
	if strings.Contains(value, "*") {
		return hostPattern{}, fmt.Errorf("wildcard host must use the *.example.com form")
	}
	if err := validateHostName(value); err != nil {
		return hostPattern{}, err
	}
	return hostPattern{value: value}, nil
}

func normalizeConfiguredHost(raw string) (string, error) {
	if host, port, err := net.SplitHostPort(raw); err == nil {
		if port == "" {
			return "", fmt.Errorf("invalid host pattern %q", raw)
		}
		if err := validatePort(port); err != nil {
			return "", fmt.Errorf("invalid host pattern %q: %w", raw, err)
		}
		raw = host
	} else if strings.Contains(raw, ":") {
		address := strings.Trim(raw, "[]")
		if _, err := netip.ParseAddr(address); err != nil {
			return "", fmt.Errorf("invalid host pattern %q", raw)
		}
	}
	return normalizeHost(raw), nil
}

func validateHostName(value string) error {
	if address, err := netip.ParseAddr(value); err == nil && address.IsValid() {
		return nil
	}
	if strings.ContainsAny(value, "/\\?#:") {
		return fmt.Errorf("invalid host pattern %q", value)
	}
	for _, label := range strings.Split(value, ".") {
		if label == "" || label[0] == '-' || label[len(label)-1] == '-' {
			return fmt.Errorf("invalid host pattern %q", value)
		}
		for i := 0; i < len(label); i++ {
			c := label[i]
			if c != '_' && c != '-' && !(c >= 'a' && c <= 'z') &&
				!(c >= '0' && c <= '9') {
				return fmt.Errorf("invalid host pattern %q", value)
			}
		}
	}
	return nil
}

func bestHostMatch(host string, patterns []hostPattern) (hostScore, bool) {
	var best hostScore
	found := false
	for _, pattern := range patterns {
		score, ok := pattern.match(host)
		if !ok {
			continue
		}
		if !found || betterHostScore(score, best) {
			best = score
			found = true
		}
	}
	return best, found
}

func betterHostScore(candidate, current hostScore) bool {
	if candidate.kind != current.kind {
		return candidate.kind > current.kind
	}
	return candidate.suffixLength > current.suffixLength
}

func (p hostPattern) match(host string) (hostScore, bool) {
	if p.value == "*" {
		return hostScore{kind: 0}, true
	}
	if !p.wildcard {
		if p.value == host {
			return hostScore{kind: 2}, true
		}
		return hostScore{}, false
	}
	if !strings.HasSuffix(host, "."+p.suffix) {
		return hostScore{}, false
	}
	prefix := strings.TrimSuffix(host, "."+p.suffix)
	if prefix == "" || strings.Contains(prefix, ".") {
		return hostScore{}, false
	}
	return hostScore{kind: 1, suffixLength: len(p.suffix)}, true
}

func normalizeHost(raw string) string {
	if strings.Contains(raw, ":") {
		if host, _, err := net.SplitHostPort(raw); err == nil {
			raw = host
		}
	}
	return strings.ToLower(strings.TrimSuffix(strings.Trim(raw, "[]"), "."))
}

func matchesHost(host string, patterns []string) bool {
	parsed, err := newHostPatterns(patterns)
	if err != nil {
		return false
	}
	_, ok := bestHostMatch(normalizeHost(host), parsed)
	return ok
}
