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
	"sync/atomic"
	"time"
)

const (
	configPath         = "kubio.json"
	configPollInterval = time.Second

	proxyDialTimeout         = 5 * time.Second
	proxyResponseHeaderLimit = 30 * time.Second
)

var proxyTransport = func() *http.Transport {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.DialContext = (&net.Dialer{Timeout: proxyDialTimeout}).DialContext
	transport.TLSHandshakeTimeout = proxyDialTimeout
	transport.ResponseHeaderTimeout = proxyResponseHeaderLimit
	return transport
}()

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
	sites        []site
	trustProxies []netip.Prefix
}

type site struct {
	hosts  []hostPattern
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
		sites:        make([]site, 0, len(cfg.Sites)),
		trustProxies: trustProxies,
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

		r.sites = append(r.sites, s)
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

	var selected *site
	var selectedScore hostScore
	for index := range r.sites {
		score, ok := bestHostMatch(host, r.sites[index].hosts)
		if !ok {
			continue
		}
		if selected == nil || betterHostScore(score, selectedScore) {
			selected = &r.sites[index]
			selectedScore = score
		}
	}
	if selected == nil {
		http.NotFound(w, req)
		return
	}

	var selectedRoute routeCandidate
	for index := range selected.routes {
		prefix, ok := selected.routes[index].pattern.match(req.URL.Path)
		if !ok {
			continue
		}
		candidate := routeCandidate{
			route:  &selected.routes[index],
			prefix: prefix,
			exact:  !selected.routes[index].pattern.wildcard,
			depth:  selected.routes[index].pattern.depth,
			index:  index,
		}
		if selectedRoute.route == nil || betterRoute(candidate, selectedRoute) {
			selectedRoute = candidate
		}
	}

	if selectedRoute.route == nil {
		selected.target.ServeHTTP(w, req)
		return
	}
	if selectedRoute.route.strip {
		req.URL.Path = stripPathPrefix(req.URL.Path, selectedRoute.prefix)
		req.URL.RawPath = ""
	}
	selectedRoute.route.proxy.ServeHTTP(w, req)
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
		ErrorHandler: proxyErrorHandler,
	}
	return proxy, nil
}

func parseTarget(raw string) (*url.URL, error) {
	target, err := url.Parse(raw)
	if err != nil {
		return nil, err
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
	switch name {
	case "Connection", "Proxy-Connection", "Keep-Alive", "Transfer-Encoding",
		"TE", "Trailer", "Upgrade", "Content-Length":
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

	value := normalizeHost(raw)
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
	if host, _, err := net.SplitHostPort(raw); err == nil {
		raw = host
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
