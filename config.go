package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
)

type config struct {
	Listen       string                   `json:"listen"`
	Log          bool                     `json:"log"`
	TrustProxies []string                 `json:"trustProxies"`
	Backends     map[string]backendConfig `json:"backends"`
	Sites        []siteConfig             `json:"sites"`
}

type backendConfig struct {
	Targets []string       `json:"targets"`
	Tries   int            `json:"tries"`
	Timeout backendTimeout `json:"timeout"`
}

type backendTimeout struct {
	Dial   time.Duration
	Header time.Duration
}

type siteConfig struct {
	Hosts           []string             `json:"hosts"`
	Target          string               `json:"target"`
	Backend         string               `json:"backend"`
	Headers         map[string]string    `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Routes          []routeConfig        `json:"routes"`
}

type routeConfig struct {
	Path            string               `json:"path"`
	Methods         []string             `json:"methods"`
	Target          string               `json:"target"`
	Backend         string               `json:"backend"`
	Headers         map[string]string    `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Strip           bool                 `json:"strip"`
}

type responseHeaderPolicy struct {
	Set    map[string][]string
	Add    map[string][]string
	Remove []string
}

type rawConfig struct {
	Listen       *string          `json:"listen"`
	Log          strictBool       `json:"log"`
	TrustProxies stringArray      `json:"trustProxies"`
	Backends     rawBackends      `json:"backends"`
	Sites        *[]rawSiteConfig `json:"sites"`
}

type rawBackendConfig struct {
	Targets *stringArray      `json:"targets"`
	Tries   optionalInt       `json:"tries"`
	Timeout rawBackendTimeout `json:"timeout"`
}

type rawBackendTimeout struct {
	Dial   optionalDuration `json:"dial"`
	Header optionalDuration `json:"header"`
}

type rawSiteConfig struct {
	Hosts           *stringArray         `json:"hosts"`
	Target          optionalString       `json:"target"`
	Backend         optionalString       `json:"backend"`
	Headers         headerMap            `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Routes          rawRoutes            `json:"routes"`
}

type rawRouteConfig struct {
	Path            *string              `json:"path"`
	Methods         optionalStringArray  `json:"methods"`
	Target          optionalString       `json:"target"`
	Backend         optionalString       `json:"backend"`
	Headers         headerMap            `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Strip           strictBool           `json:"strip"`
}

type headerMap map[string]string
type headerValues map[string][]string

type stringArray []string
type rawBackends map[string]rawBackendConfig
type rawRoutes []rawRouteConfig

type optionalString struct {
	set   bool
	value string
}

type optionalStringArray struct {
	set    bool
	values stringArray
}

type optionalHeaderValues struct {
	set    bool
	values headerValues
}

type optionalInt struct {
	set   bool
	value int
}

type optionalDuration struct {
	set   bool
	value time.Duration
}

type strictBool bool

func (b *rawBackends) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be an object")
	}
	var backends map[string]rawBackendConfig
	if err := json.Unmarshal(data, &backends); err != nil {
		return fmt.Errorf("must be an object: %w", err)
	}
	*b = backends
	return nil
}

func (b *rawBackendConfig) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	type plain rawBackendConfig
	return json.Unmarshal(data, (*plain)(b))
}

func (t *rawBackendTimeout) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Dial   optionalDuration `json:"dial"`
		Header optionalDuration `json:"header"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Dial.set && !decoded.Header.set {
		return fmt.Errorf("must contain dial or header")
	}
	*t = rawBackendTimeout{Dial: decoded.Dial, Header: decoded.Header}
	return nil
}

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

func (h *responseHeaderPolicy) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Set    optionalHeaderValues `json:"set"`
		Add    optionalHeaderValues `json:"add"`
		Remove optionalStringArray  `json:"remove"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Set.set && !decoded.Add.set && !decoded.Remove.set {
		return fmt.Errorf("must contain at least one of set, add, or remove")
	}
	if decoded.Set.set && len(decoded.Set.values) == 0 {
		return fmt.Errorf("set must not be empty")
	}
	if decoded.Add.set && len(decoded.Add.values) == 0 {
		return fmt.Errorf("add must not be empty")
	}
	if decoded.Remove.set && len(decoded.Remove.values) == 0 {
		return fmt.Errorf("remove must not be empty")
	}
	*h = responseHeaderPolicy{
		Set:    map[string][]string(decoded.Set.values),
		Add:    map[string][]string(decoded.Add.values),
		Remove: []string(decoded.Remove.values),
	}
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

func (a *optionalStringArray) UnmarshalJSON(data []byte) error {
	var values stringArray
	if err := json.Unmarshal(data, &values); err != nil {
		return err
	}
	a.set = true
	a.values = values
	return nil
}

func (h *optionalHeaderValues) UnmarshalJSON(data []byte) error {
	var values headerValues
	if err := json.Unmarshal(data, &values); err != nil {
		return err
	}
	h.set = true
	h.values = values
	return nil
}

func (i *optionalInt) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if bytes.Equal(data, []byte("null")) || bytes.ContainsAny(data, ".eE") {
		return fmt.Errorf("must be an integer without a decimal point or exponent")
	}
	var value int
	if err := json.Unmarshal(data, &value); err != nil {
		return fmt.Errorf("must be an integer without a decimal point or exponent")
	}
	i.set = true
	i.value = value
	return nil
}

func (d *optionalDuration) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be a positive Go duration string")
	}
	var value string
	if err := json.Unmarshal(data, &value); err != nil {
		return fmt.Errorf("must be a positive Go duration string")
	}
	duration, err := time.ParseDuration(value)
	if err != nil || duration <= 0 {
		return fmt.Errorf("must be a positive Go duration string")
	}
	d.set = true
	d.value = duration
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
		Log:          bool(raw.Log),
		TrustProxies: append([]string(nil), raw.TrustProxies...),
		Backends:     make(map[string]backendConfig, len(raw.Backends)),
		Sites:        make([]siteConfig, len(*raw.Sites)),
	}
	for name, rawBackend := range raw.Backends {
		if name == "" {
			return config{}, fmt.Errorf("backend name must not be empty")
		}
		if rawBackend.Targets == nil || len(*rawBackend.Targets) == 0 {
			return config{}, fmt.Errorf("backends[%q].targets must not be empty", name)
		}
		tries := 1
		if rawBackend.Tries.set {
			tries = rawBackend.Tries.value
			if tries < 1 || tries > len(*rawBackend.Targets) {
				return config{}, fmt.Errorf("backends[%q].tries must be between 1 and the target count", name)
			}
		}
		cfg.Backends[name] = backendConfig{
			Targets: append([]string(nil), (*rawBackend.Targets)...),
			Tries:   tries,
			Timeout: backendTimeout{
				Dial:   rawBackend.Timeout.Dial.value,
				Header: rawBackend.Timeout.Header.value,
			},
		}
	}
	for siteIndex, rawSite := range *raw.Sites {
		if rawSite.Hosts == nil || len(*rawSite.Hosts) == 0 {
			return config{}, fmt.Errorf("sites[%d].hosts must not be empty", siteIndex)
		}
		if rawSite.Target.set == rawSite.Backend.set {
			return config{}, fmt.Errorf("sites[%d] must set exactly one of target or backend", siteIndex)
		}
		if rawSite.Target.set && rawSite.Target.value == "" {
			return config{}, fmt.Errorf("sites[%d].target must not be empty", siteIndex)
		}
		if rawSite.Backend.set && rawSite.Backend.value == "" {
			return config{}, fmt.Errorf("sites[%d].backend must not be empty", siteIndex)
		}

		site := siteConfig{
			Hosts:           append([]string(nil), (*rawSite.Hosts)...),
			Headers:         map[string]string(rawSite.Headers),
			ResponseHeaders: rawSite.ResponseHeaders,
			Routes:          make([]routeConfig, len(rawSite.Routes)),
		}
		if rawSite.Target.set {
			site.Target = rawSite.Target.value
		} else {
			site.Backend = rawSite.Backend.value
		}
		for routeIndex, rawRoute := range rawSite.Routes {
			if rawRoute.Path == nil || *rawRoute.Path == "" {
				return config{}, fmt.Errorf("sites[%d].routes[%d].path must be set", siteIndex, routeIndex)
			}
			if rawRoute.Methods.set {
				if err := validateMethods(rawRoute.Methods.values); err != nil {
					return config{}, fmt.Errorf("sites[%d].routes[%d].methods: %w", siteIndex, routeIndex, err)
				}
			}
			if rawRoute.Target.set && rawRoute.Backend.set {
				return config{}, fmt.Errorf("sites[%d].routes[%d] cannot set both target and backend", siteIndex, routeIndex)
			}
			if rawRoute.Target.set && rawRoute.Target.value == "" {
				return config{}, fmt.Errorf("sites[%d].routes[%d].target must not be empty", siteIndex, routeIndex)
			}
			if rawRoute.Backend.set && rawRoute.Backend.value == "" {
				return config{}, fmt.Errorf("sites[%d].routes[%d].backend must not be empty", siteIndex, routeIndex)
			}
			route := routeConfig{
				Path:            *rawRoute.Path,
				Methods:         append([]string(nil), rawRoute.Methods.values...),
				Headers:         map[string]string(rawRoute.Headers),
				ResponseHeaders: rawRoute.ResponseHeaders,
				Strip:           bool(rawRoute.Strip),
			}
			if rawRoute.Target.set {
				route.Target = rawRoute.Target.value
			} else if rawRoute.Backend.set {
				route.Backend = rawRoute.Backend.value
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
	jsonBackends
	jsonBackend
	jsonSites
	jsonSite
	jsonRoutes
	jsonRoute
	jsonHeaders
	jsonResponseHeaders
	jsonBackendTimeout
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
		case "listen", "log", "trustProxies":
			return jsonAny, nil
		case "backends":
			return jsonBackends, nil
		case "sites":
			return jsonSites, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackends:
		return jsonBackend, nil
	case jsonBackend:
		switch key {
		case "targets", "tries":
			return jsonAny, nil
		case "timeout":
			return jsonBackendTimeout, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendTimeout:
		switch key {
		case "dial", "header":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonSite:
		switch key {
		case "hosts", "target", "backend":
			return jsonAny, nil
		case "headers":
			return jsonHeaders, nil
		case "response":
			return jsonResponseHeaders, nil
		case "routes":
			return jsonRoutes, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonRoute:
		switch key {
		case "path", "methods", "target", "backend", "strip":
			return jsonAny, nil
		case "headers":
			return jsonHeaders, nil
		case "response":
			return jsonResponseHeaders, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonResponseHeaders:
		switch key {
		case "set", "add":
			return jsonHeaders, nil
		case "remove":
			return jsonAny, nil
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

func (m *headerValues) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return fmt.Errorf("headers must be an object: %w", err)
	}
	if raw == nil {
		return fmt.Errorf("headers must be an object")
	}

	values := make(headerValues, len(raw))
	for name, encoded := range raw {
		encoded = bytes.TrimSpace(encoded)
		if len(encoded) > 0 && encoded[0] == '"' {
			var value string
			if err := json.Unmarshal(encoded, &value); err != nil {
				return fmt.Errorf("header %q value must be a string or non-empty array of strings", name)
			}
			values[name] = []string{value}
			continue
		}

		var array stringArray
		if err := json.Unmarshal(encoded, &array); err != nil {
			return fmt.Errorf("header %q value must be a string or non-empty array of strings: %w", name, err)
		}
		if len(array) == 0 {
			return fmt.Errorf("header %q value array must not be empty", name)
		}
		values[name] = []string(array)
	}
	*m = values
	return nil
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

func resolveResponseHeaders(raw responseHeaderPolicy) (responseHeaderPolicy, error) {
	set, err := resolveResponseHeaderValues(raw.Set)
	if err != nil {
		return responseHeaderPolicy{}, fmt.Errorf("set: %w", err)
	}
	add, err := resolveResponseHeaderValues(raw.Add)
	if err != nil {
		return responseHeaderPolicy{}, fmt.Errorf("add: %w", err)
	}
	remove, err := resolveResponseHeaderNames(raw.Remove)
	if err != nil {
		return responseHeaderPolicy{}, fmt.Errorf("remove: %w", err)
	}
	return responseHeaderPolicy{Set: set, Add: add, Remove: remove}, nil
}

func resolveResponseHeaderValues(raw map[string][]string) (map[string][]string, error) {
	if len(raw) == 0 {
		return nil, nil
	}

	headers := make(map[string][]string, len(raw))
	for rawName, rawValues := range raw {
		name, err := responseHeaderName(rawName)
		if err != nil {
			return nil, err
		}
		if _, exists := headers[name]; exists {
			return nil, fmt.Errorf("duplicate header name %q", name)
		}
		if len(rawValues) == 0 {
			return nil, fmt.Errorf("header %q value array must not be empty", name)
		}

		values := make([]string, len(rawValues))
		for index, value := range rawValues {
			value, err = expandEnv(value)
			if err != nil {
				return nil, fmt.Errorf("header %q value %d: %w", name, index, err)
			}
			if !validHeaderValue(value) {
				return nil, fmt.Errorf("header %q value %d contains invalid control characters", name, index)
			}
			values[index] = value
		}
		headers[name] = values
	}
	return headers, nil
}

func resolveResponseHeaderNames(raw []string) ([]string, error) {
	names := make([]string, len(raw))
	seen := make(map[string]struct{}, len(raw))
	for index, rawName := range raw {
		name, err := responseHeaderName(rawName)
		if err != nil {
			return nil, fmt.Errorf("item %d: %w", index, err)
		}
		if _, exists := seen[name]; exists {
			return nil, fmt.Errorf("contains duplicate header name %q", name)
		}
		seen[name] = struct{}{}
		names[index] = name
	}
	return names, nil
}

func responseHeaderName(name string) (string, error) {
	if !validHeaderName(name) {
		return "", fmt.Errorf("invalid header name %q", name)
	}
	name = http.CanonicalHeaderKey(name)
	if restrictedHeader(name) || name == "Proxy-Authenticate" || name == "Proxy-Authorization" {
		return "", fmt.Errorf("header %q is managed by the proxy", name)
	}
	return name, nil
}

func validHeaderName(name string) bool {
	if name == "" {
		return false
	}
	for index := 0; index < len(name); index++ {
		if !validHeaderNameByte(name[index]) {
			return false
		}
	}
	return true
}

func validateMethods(methods []string) error {
	if len(methods) == 0 {
		return fmt.Errorf("must not be empty")
	}
	seen := make(map[string]struct{}, len(methods))
	for index, method := range methods {
		if !validMethod(method) {
			return fmt.Errorf("item %d must be a non-empty ASCII HTTP token", index)
		}
		if _, exists := seen[method]; exists {
			return fmt.Errorf("contains duplicate %q", method)
		}
		seen[method] = struct{}{}
	}
	return nil
}

func validMethod(method string) bool {
	if method == "" {
		return false
	}
	for index := 0; index < len(method); index++ {
		if method[index] != '*' && !validHeaderNameByte(method[index]) {
			return false
		}
	}
	return true
}

func validHeaderNameByte(value byte) bool {
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
