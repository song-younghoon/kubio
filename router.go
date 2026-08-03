package main

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"net/netip"
	"net/url"
	"strings"
	"sync"
	"time"
)

const routeIndexThreshold = 8

type router struct {
	accessLogger     *accessLogger
	backends         map[string]*backend
	generation       *backendGeneration
	directTransports []*http.Transport
	sites            []site
	trustProxies     []netip.Prefix
	exactHosts       map[string]int
	wildcardHosts    map[string]int
	starSite         int
	closeOnce        sync.Once
}

type site struct {
	hosts          []hostPattern
	proxy          *httputil.ReverseProxy
	routes         []route
	exactRoutes    map[string][]int
	wildcardRoutes map[string][]int
}

type route struct {
	pattern pathPattern
	methods []string
	match   routeMatch
	proxy   *httputil.ReverseProxy
	strip   bool
}

type routeMatch struct {
	header       []routeCondition
	query        []routeCondition
	alternatives int
}

type routeCondition struct {
	name         string
	alternatives []string
}

type routeMatchState struct {
	req         *http.Request
	query       url.Values
	queryParsed bool
	queryValid  bool
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

func newRouter(cfg config) (*router, error) {
	if len(cfg.Sites) == 0 {
		return nil, fmt.Errorf("sites must define at least one site")
	}
	trustProxies, err := parseTrustedProxies(cfg.TrustProxies)
	if err != nil {
		return nil, fmt.Errorf("trustProxies: %w", err)
	}
	backends, err := newBackends(cfg.Backends)
	if err != nil {
		return nil, err
	}
	generation := &backendGeneration{}
	for _, backend := range backends {
		if backend.health != nil {
			backend.health.generation = generation
		}
	}

	r := &router{
		backends:      backends,
		generation:    generation,
		sites:         make([]site, 0, len(cfg.Sites)),
		trustProxies:  trustProxies,
		exactHosts:    make(map[string]int),
		wildcardHosts: make(map[string]int),
		starSite:      -1,
	}
	transports := newDirectTransportCache()
	for siteIndex, siteConfig := range cfg.Sites {
		s, err := newSite(siteConfig, backends, trustProxies, transports)
		if err != nil {
			closeDirectTransports(transports.all)
			return nil, fmt.Errorf("sites[%d]: %w", siteIndex, err)
		}

		r.sites = append(r.sites, s)
		for _, host := range s.hosts {
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
	if cfg.Log {
		prepareAccessLog()
		r.accessLogger = stdoutAccessLogger
	}
	for _, backend := range backends {
		backend.startProbe()
	}
	r.directTransports = transports.all

	return r, nil
}

func (r *router) close() {
	r.closeOnce.Do(func() {
		if r.generation != nil {
			r.generation.retired.Store(true)
		}
		for _, backend := range r.backends {
			backend.cancelProbe()
		}
		for _, backend := range r.backends {
			backend.joinProbe()
		}
		closeDirectTransports(r.directTransports)
	})
}

func newSite(cfg siteConfig, backends map[string]*backend, trustProxies []netip.Prefix, transports *directTransportCache) (site, error) {
	if len(cfg.Hosts) == 0 {
		return site{}, fmt.Errorf("hosts must not be empty")
	}
	hosts, err := newHostPatterns(cfg.Hosts)
	if err != nil {
		return site{}, fmt.Errorf("hosts: %w", err)
	}
	siteHeaders, err := resolveHeaders(cfg.Headers)
	if err != nil {
		return site{}, fmt.Errorf("headers: %w", err)
	}
	siteResponseHeaders, err := resolveResponseHeaders(cfg.ResponseHeaders)
	if err != nil {
		return site{}, fmt.Errorf("response: %w", err)
	}

	proxy, err := newProxyForSelection(
		cfg.Target,
		cfg.Backend,
		cfg.Timeout,
		siteHeaders,
		siteResponseHeaders,
		responseHeaderPolicy{},
		trustProxies,
		backends,
		transports,
	)
	if err != nil {
		return site{}, err
	}
	s := site{
		hosts:  hosts,
		proxy:  proxy,
		routes: make([]route, 0, len(cfg.Routes)),
	}
	for routeIndex, routeConfig := range cfg.Routes {
		pattern, err := newPathPattern(routeConfig.Path)
		if err != nil {
			return s, fmt.Errorf("routes[%d].path: %w", routeIndex, err)
		}
		if routeConfig.Methods != nil {
			if err := validateMethods(routeConfig.Methods); err != nil {
				return s, fmt.Errorf("routes[%d].methods: %w", routeIndex, err)
			}
		}
		match, err := resolveRouteMatch(routeConfig.Match)
		if err != nil {
			return s, fmt.Errorf("routes[%d].match: %w", routeIndex, err)
		}
		routeHeaders, err := resolveHeaders(routeConfig.Headers)
		if err != nil {
			return s, fmt.Errorf("routes[%d].headers: %w", routeIndex, err)
		}
		routeResponseHeaders, err := resolveResponseHeaders(routeConfig.ResponseHeaders)
		if err != nil {
			return s, fmt.Errorf("routes[%d].response: %w", routeIndex, err)
		}

		target, backendName := cfg.Target, cfg.Backend
		if routeConfig.Target != "" || routeConfig.Backend != "" {
			target, backendName = routeConfig.Target, routeConfig.Backend
		}
		timeout := routeConfig.Timeout
		if timeout == nil && routeConfig.Target == "" && routeConfig.Backend == "" && target != "" {
			timeout = cfg.Timeout
		}
		routeProxy, err := newProxyForSelection(
			target,
			backendName,
			timeout,
			mergeHeaders(siteHeaders, routeHeaders),
			siteResponseHeaders,
			routeResponseHeaders,
			trustProxies,
			backends,
			transports,
		)
		if err != nil {
			return s, fmt.Errorf("routes[%d]: %w", routeIndex, err)
		}
		s.routes = append(s.routes, route{
			pattern: pattern,
			methods: append([]string(nil), routeConfig.Methods...),
			match:   compileRouteMatch(match),
			proxy:   routeProxy,
			strip:   routeConfig.Strip,
		})
	}
	if len(s.routes) > routeIndexThreshold {
		s.buildRouteIndex()
	}
	return s, nil
}

func compileRouteMatch(config routeMatchConfig) routeMatch {
	var match routeMatch
	if len(config.Header) > 0 {
		match.header = make([]routeCondition, 0, len(config.Header))
	}
	if len(config.Query) > 0 {
		match.query = make([]routeCondition, 0, len(config.Query))
	}
	for name, alternatives := range config.Header {
		match.header = append(match.header, routeCondition{name: name, alternatives: alternatives})
		match.alternatives += len(alternatives)
	}
	for name, alternatives := range config.Query {
		match.query = append(match.query, routeCondition{name: name, alternatives: alternatives})
		match.alternatives += len(alternatives)
	}
	return match
}

func newBackends(configs map[string]backendConfig) (map[string]*backend, error) {
	if len(configs) == 0 {
		return nil, nil
	}
	backends := make(map[string]*backend, len(configs))
	for name, cfg := range configs {
		if name == "" {
			return nil, fmt.Errorf("backend name must not be empty")
		}
		backend, err := newBackend(cfg)
		if err != nil {
			return nil, fmt.Errorf("backends[%q]: %w", name, err)
		}
		backends[name] = backend
	}
	return backends, nil
}

func newProxyForSelection(
	target, backendName string,
	timeout *directTimeout,
	headers map[string]string,
	siteResponseHeaders, routeResponseHeaders responseHeaderPolicy,
	trustProxies []netip.Prefix,
	backends map[string]*backend,
	transports *directTransportCache,
) (*httputil.ReverseProxy, error) {
	if err := validateDirectTimeout(timeout); err != nil {
		return nil, err
	}
	hasTarget, hasBackend := target != "", backendName != ""
	if hasTarget == hasBackend {
		return nil, fmt.Errorf("must set exactly one of target or backend")
	}
	if hasTarget {
		proxy, err := newProxy(target, timeout, headers, siteResponseHeaders, routeResponseHeaders, trustProxies, transports)
		if err != nil {
			return nil, fmt.Errorf("target: %w", err)
		}
		return proxy, nil
	}
	if timeout != nil {
		return nil, fmt.Errorf("timeout is not allowed with backend")
	}
	backend, ok := backends[backendName]
	if !ok {
		return nil, fmt.Errorf("backend %q is not defined", backendName)
	}
	return newBackendProxy(backend, headers, siteResponseHeaders, routeResponseHeaders, trustProxies), nil
}

func validateDirectTimeout(timeout *directTimeout) error {
	if timeout == nil {
		return nil
	}
	for name, duration := range map[string]time.Duration{
		"dial":   timeout.Dial,
		"header": timeout.Header,
		"body":   timeout.Body,
	} {
		if duration < 0 || duration > maxDirectTimeout {
			return fmt.Errorf("timeout.%s must not be negative and no greater than %s", name, maxDirectTimeout)
		}
	}
	return nil
}

func closeDirectTransports(transports []*http.Transport) {
	for _, transport := range transports {
		transport.CloseIdleConnections()
	}
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

func (r *router) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	if r.accessLogger != nil {
		r.serveLogged(w, req)
		return
	}
	r.serveHTTP(w, req)
}

func (r *router) serveHTTP(w http.ResponseWriter, req *http.Request) {
	if req.Method == http.MethodOptions && req.RequestURI == "*" {
		serveGeneralOptions(w, req)
		return
	}
	host := normalizeHost(req.Host)
	selected := r.selectSite(host)
	if selected == nil {
		http.NotFound(w, req)
		return
	}

	selectedRoute := selected.selectRoute(req)

	if selectedRoute.route == nil {
		selected.proxy.ServeHTTP(w, req)
		return
	}
	if selectedRoute.route.strip {
		stripRequestPath(req, selectedRoute.prefix)
	}
	selectedRoute.route.proxy.ServeHTTP(w, req)
}

func serveGeneralOptions(w http.ResponseWriter, req *http.Request) {
	w.Header().Set("Content-Length", "0")
	if req.ContentLength == 0 {
		return
	}
	limitWriter := w
	if unwrapper, ok := w.(interface{ Unwrap() http.ResponseWriter }); ok {
		limitWriter = unwrapper.Unwrap()
	}
	body := http.MaxBytesReader(limitWriter, req.Body, 4<<10)
	_, _ = io.Copy(io.Discard, body)
}

func (s *site) buildRouteIndex() {
	s.exactRoutes = make(map[string][]int, len(s.routes))
	s.wildcardRoutes = make(map[string][]int, len(s.routes))
	for index := range s.routes {
		pattern := s.routes[index].pattern
		if pattern.wildcard {
			s.wildcardRoutes[pattern.path] = append(s.wildcardRoutes[pattern.path], index)
		} else {
			s.exactRoutes[pattern.path] = append(s.exactRoutes[pattern.path], index)
		}
	}
}

func (s *site) selectRoute(req *http.Request) routeCandidate {
	path := req.URL.Path
	state := routeMatchState{req: req}
	if s.exactRoutes == nil {
		var selected routeCandidate
		for index := range s.routes {
			prefix, ok := s.routes[index].pattern.match(path)
			if !ok || !state.matches(&s.routes[index]) {
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

	var selected routeCandidate
	for _, index := range s.exactRoutes[path] {
		if !state.matches(&s.routes[index]) {
			continue
		}
		candidate := s.routeCandidate(index)
		if selected.route == nil || betterRoute(candidate, selected) {
			selected = candidate
		}
	}
	if selected.route != nil {
		return selected
	}

	for _, index := range s.wildcardRoutes[""] {
		if state.matches(&s.routes[index]) {
			candidate := s.routeCandidate(index)
			if selected.route == nil || betterRoute(candidate, selected) {
				selected = candidate
			}
		}
	}
	for index := 1; index < len(path); index++ {
		if path[index] != '/' {
			continue
		}
		for _, routeIndex := range s.wildcardRoutes[path[:index]] {
			if !state.matches(&s.routes[routeIndex]) {
				continue
			}
			candidate := s.routeCandidate(routeIndex)
			if selected.route == nil || betterRoute(candidate, selected) {
				selected = candidate
			}
		}
	}
	for _, routeIndex := range s.wildcardRoutes[path] {
		if !state.matches(&s.routes[routeIndex]) {
			continue
		}
		candidate := s.routeCandidate(routeIndex)
		if selected.route == nil || betterRoute(candidate, selected) {
			selected = candidate
		}
	}
	return selected
}

func (m *routeMatchState) matches(route *route) bool {
	if !route.matchesMethod(m.req.Method) {
		return false
	}
	for _, condition := range route.match.header {
		if !matchesRequestHeader(m.req, condition) {
			return false
		}
	}
	if len(route.match.query) == 0 {
		return true
	}
	if !m.queryParsed {
		var err error
		m.query, err = url.ParseQuery(m.req.URL.RawQuery)
		m.queryValid = err == nil
		m.queryParsed = true
	}
	if !m.queryValid {
		return false
	}
	for _, condition := range route.match.query {
		if !matchesAny(m.query[condition.name], condition.alternatives) {
			return false
		}
	}
	return true
}

func matchesRequestHeader(req *http.Request, condition routeCondition) bool {
	if condition.name == "Host" {
		return req.Host != "" && matchesValue(req.Host, condition.alternatives)
	}
	if matchesAny(req.Header[condition.name], condition.alternatives) {
		return true
	}
	for name, values := range req.Header {
		if name != condition.name && strings.EqualFold(name, condition.name) && matchesAny(values, condition.alternatives) {
			return true
		}
	}
	return false
}

func matchesValue(value string, alternatives []string) bool {
	for _, alternative := range alternatives {
		if value == alternative {
			return true
		}
	}
	return false
}

func matchesAny(values, alternatives []string) bool {
	for _, value := range values {
		for _, alternative := range alternatives {
			if value == alternative {
				return true
			}
		}
	}
	return false
}

func (r *route) matchesMethod(method string) bool {
	if len(r.methods) == 0 {
		return true
	}
	if len(r.methods) == 1 {
		return method == r.methods[0]
	}
	for _, allowed := range r.methods {
		if method == allowed {
			return true
		}
	}
	return false
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
	candidateConditions, currentConditions := candidate.route.conditionCount(), current.route.conditionCount()
	if candidateConditions != currentConditions {
		return candidateConditions > currentConditions
	}
	candidateAlternatives, currentAlternatives := candidate.route.alternativeCount(), current.route.alternativeCount()
	if candidateAlternatives != currentAlternatives {
		return candidateAlternatives < currentAlternatives
	}
	return candidate.index > current.index
}

func (r *route) conditionCount() int {
	count := len(r.match.header) + len(r.match.query)
	if len(r.methods) > 0 {
		count++
	}
	return count
}

func (r *route) alternativeCount() int {
	return len(r.methods) + r.match.alternatives
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
