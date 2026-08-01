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
)

type config struct {
	Listen       string                   `json:"listen"`
	TrustProxies []string                 `json:"trustProxies"`
	Backends     map[string]backendConfig `json:"backends"`
	Sites        []siteConfig             `json:"sites"`
}

type backendConfig struct {
	Targets []string `json:"targets"`
}

type siteConfig struct {
	Hosts   []string          `json:"hosts"`
	Target  string            `json:"target"`
	Backend string            `json:"backend"`
	Headers map[string]string `json:"headers"`
	Routes  []routeConfig     `json:"routes"`
}

type routeConfig struct {
	Path    string            `json:"path"`
	Target  string            `json:"target"`
	Backend string            `json:"backend"`
	Headers map[string]string `json:"headers"`
	Strip   bool              `json:"strip"`
}

type rawConfig struct {
	Listen       *string          `json:"listen"`
	TrustProxies stringArray      `json:"trustProxies"`
	Backends     rawBackends      `json:"backends"`
	Sites        *[]rawSiteConfig `json:"sites"`
}

type rawBackendConfig struct {
	Targets *stringArray `json:"targets"`
}

type rawSiteConfig struct {
	Hosts   *stringArray   `json:"hosts"`
	Target  optionalString `json:"target"`
	Backend optionalString `json:"backend"`
	Headers headerMap      `json:"headers"`
	Routes  rawRoutes      `json:"routes"`
}

type rawRouteConfig struct {
	Path    *string        `json:"path"`
	Target  optionalString `json:"target"`
	Backend optionalString `json:"backend"`
	Headers headerMap      `json:"headers"`
	Strip   strictBool     `json:"strip"`
}

type headerMap map[string]string

type stringArray []string
type rawBackends map[string]rawBackendConfig
type rawRoutes []rawRouteConfig

type optionalString struct {
	set   bool
	value string
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
		cfg.Backends[name] = backendConfig{Targets: append([]string(nil), (*rawBackend.Targets)...)}
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
			Hosts:   append([]string(nil), (*rawSite.Hosts)...),
			Headers: map[string]string(rawSite.Headers),
			Routes:  make([]routeConfig, len(rawSite.Routes)),
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
				Path:    *rawRoute.Path,
				Headers: map[string]string(rawRoute.Headers),
				Strip:   bool(rawRoute.Strip),
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
		case "backends":
			return jsonBackends, nil
		case "sites":
			return jsonSites, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackends:
		return jsonBackend, nil
	case jsonBackend:
		if key == "targets" {
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonSite:
		switch key {
		case "hosts", "target", "backend":
			return jsonAny, nil
		case "headers":
			return jsonHeaders, nil
		case "routes":
			return jsonRoutes, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonRoute:
		switch key {
		case "path", "target", "backend", "strip":
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
