package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
)

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
