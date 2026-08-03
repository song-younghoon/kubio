package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	maxRetryBodyBytes       int64 = 64 << 20
	maxRetryBudget                = 1_000_000
	maxTargetWeight               = 1_000
	maxTargetWeightTotal          = 10_000
	maxHealthFailures             = 1_000
	maxHealthCooldown             = 24 * time.Hour
	maxHealthProbePathBytes       = 2_048
	maxHealthProbeDuration        = 24 * time.Hour
	maxDirectTimeout              = 24 * time.Hour
)

type config struct {
	Listen       string                   `json:"listen"`
	Log          bool                     `json:"log"`
	TLS          *tlsConfig               `json:"tls"`
	TrustProxies []string                 `json:"trustProxies"`
	Backends     map[string]backendConfig `json:"backends"`
	Sites        []siteConfig             `json:"sites"`
}

type tlsConfig struct {
	Cert string `json:"cert"`
	Key  string `json:"key"`
}

type backendConfig struct {
	Targets []string             `json:"targets"`
	Weights []int                `json:"weights"`
	Tries   int                  `json:"tries"`
	Timeout backendTimeout       `json:"timeout"`
	TLS     *upstreamTLSConfig   `json:"tls"`
	Health  *backendHealthConfig `json:"health"`
	Retry   *backendRetryConfig  `json:"retry"`
}

type backendTimeout struct {
	Dial   time.Duration
	Header time.Duration
}

type directTimeout struct {
	Dial   time.Duration
	Header time.Duration
	Body   time.Duration
}

type backendRetryConfig struct {
	Status   []int
	Methods  []string
	Body     *backendBodyConfig
	Backoff  *backendBackoffConfig
	Deadline time.Duration
	Budget   *backendBudgetConfig
}

type backendBodyConfig struct {
	Max int64
}

type backendBudgetConfig struct {
	Max    int
	Window time.Duration
}

type backendBackoffConfig struct {
	Base   time.Duration
	Cap    time.Duration
	Jitter bool
}

type backendHealthConfig struct {
	Fail  int
	Cool  time.Duration
	Probe *backendHealthProbeConfig
}

type backendHealthProbeConfig struct {
	Path    url.URL
	Every   time.Duration
	Timeout time.Duration
}

type siteConfig struct {
	Hosts           []string             `json:"hosts"`
	Target          string               `json:"target"`
	Backend         string               `json:"backend"`
	Timeout         *directTimeout       `json:"timeout"`
	TLS             *upstreamTLSConfig   `json:"tls"`
	Headers         map[string]string    `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Routes          []routeConfig        `json:"routes"`
}

type routeConfig struct {
	Path            string               `json:"path"`
	Methods         []string             `json:"methods"`
	Match           *routeMatchConfig    `json:"match"`
	Target          string               `json:"target"`
	Backend         string               `json:"backend"`
	Timeout         *directTimeout       `json:"timeout"`
	TLS             *upstreamTLSConfig   `json:"tls"`
	Headers         map[string]string    `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Strip           bool                 `json:"strip"`
}

type routeMatchConfig struct {
	Header map[string][]string
	Query  map[string][]string
}

type responseHeaderPolicy struct {
	Set    map[string][]string
	Add    map[string][]string
	Remove []string
}

type rawConfig struct {
	Listen       *string          `json:"listen"`
	Log          strictBool       `json:"log"`
	TLS          rawTLSConfig     `json:"tls"`
	TrustProxies stringArray      `json:"trustProxies"`
	Backends     rawBackends      `json:"backends"`
	Sites        *[]rawSiteConfig `json:"sites"`
}

type rawTLSConfig struct {
	set  bool
	Cert string
	Key  string
}

type rawBackendConfig struct {
	Targets *stringArray      `json:"targets"`
	Weights optionalIntArray  `json:"weights"`
	Tries   optionalInt       `json:"tries"`
	Timeout rawBackendTimeout `json:"timeout"`
	TLS     rawUpstreamTLS    `json:"tls"`
	Health  rawBackendHealth  `json:"health"`
	Retry   rawBackendRetry   `json:"retry"`
}

type rawBackendRetry struct {
	set      bool
	Status   optionalIntArray    `json:"status"`
	Methods  optionalStringArray `json:"methods"`
	Body     rawBackendBody      `json:"body"`
	Backoff  rawBackendBackoff   `json:"backoff"`
	Deadline optionalDuration    `json:"deadline"`
	Budget   rawBackendBudget    `json:"budget"`
}

type rawBackendBody struct {
	set bool
	Max optionalInt64 `json:"max"`
}

type rawBackendBudget struct {
	set    bool
	Max    optionalInt      `json:"max"`
	Window optionalDuration `json:"window"`
}

type rawBackendHealth struct {
	set   bool
	Fail  optionalInt           `json:"fail"`
	Cool  optionalDuration      `json:"cool"`
	Probe rawBackendHealthProbe `json:"probe"`
}

type rawBackendHealthProbe struct {
	set     bool
	Path    url.URL
	Every   time.Duration
	Timeout time.Duration
}

type rawBackendBackoff struct {
	set    bool
	Base   optionalDuration `json:"base"`
	Cap    optionalDuration `json:"cap"`
	Jitter optionalString   `json:"jitter"`
}

type rawBackendTimeout struct {
	Dial   optionalDuration `json:"dial"`
	Header optionalDuration `json:"header"`
}

type rawDirectTimeout struct {
	set    bool
	Dial   optionalDuration `json:"dial"`
	Header optionalDuration `json:"header"`
	Body   optionalDuration `json:"body"`
}

type rawSiteConfig struct {
	Hosts           *stringArray         `json:"hosts"`
	Target          optionalString       `json:"target"`
	Backend         optionalString       `json:"backend"`
	Timeout         rawDirectTimeout     `json:"timeout"`
	TLS             rawUpstreamTLS       `json:"tls"`
	Headers         headerMap            `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Routes          rawRoutes            `json:"routes"`
}

type rawRouteConfig struct {
	Path            *string              `json:"path"`
	Methods         optionalStringArray  `json:"methods"`
	Match           rawRouteMatch        `json:"match"`
	Target          optionalString       `json:"target"`
	Backend         optionalString       `json:"backend"`
	Timeout         rawDirectTimeout     `json:"timeout"`
	TLS             rawUpstreamTLS       `json:"tls"`
	Headers         headerMap            `json:"headers"`
	ResponseHeaders responseHeaderPolicy `json:"response"`
	Strip           strictBool           `json:"strip"`
}

type rawUpstreamTLS struct {
	set  bool
	CA   optionalString `json:"ca"`
	Name optionalString `json:"name"`
}

type rawRouteMatch struct {
	set    bool
	Header optionalConditionValues
	Query  optionalConditionValues
}

type headerMap map[string]string
type headerValues map[string][]string
type conditionValues map[string][]string

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

type optionalConditionValues struct {
	set    bool
	values conditionValues
}

type optionalInt struct {
	set   bool
	value int
}

type optionalInt64 struct {
	set   bool
	value int64
}

type optionalIntArray struct {
	set    bool
	values []int
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

func (t *rawTLSConfig) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Cert optionalString `json:"cert"`
		Key  optionalString `json:"key"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Cert.set || decoded.Cert.value == "" {
		return fmt.Errorf("cert must be a non-empty string")
	}
	if !decoded.Key.set || decoded.Key.value == "" {
		return fmt.Errorf("key must be a non-empty string")
	}
	*t = rawTLSConfig{set: true, Cert: decoded.Cert.value, Key: decoded.Key.value}
	return nil
}

func (t *rawUpstreamTLS) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		CA   optionalString `json:"ca"`
		Name optionalString `json:"name"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.CA.set && !decoded.Name.set {
		return fmt.Errorf("must contain ca or name")
	}
	if decoded.CA.set && decoded.CA.value == "" {
		return fmt.Errorf("ca must be a non-empty string")
	}
	if decoded.Name.set {
		if decoded.Name.value == "" {
			return fmt.Errorf("name must be a non-empty string")
		}
		if err := validateUpstreamServerName(decoded.Name.value); err != nil {
			return fmt.Errorf("name: %w", err)
		}
	}
	*t = rawUpstreamTLS{set: true, CA: decoded.CA, Name: decoded.Name}
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

func (t *rawDirectTimeout) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Dial   optionalDuration `json:"dial"`
		Header optionalDuration `json:"header"`
		Body   optionalDuration `json:"body"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Dial.set && !decoded.Header.set && !decoded.Body.set {
		return fmt.Errorf("must contain dial, header, or body")
	}
	if decoded.Dial.set && decoded.Dial.value > maxDirectTimeout {
		return fmt.Errorf("dial must be no greater than %s", maxDirectTimeout)
	}
	if decoded.Header.set && decoded.Header.value > maxDirectTimeout {
		return fmt.Errorf("header must be no greater than %s", maxDirectTimeout)
	}
	if decoded.Body.set && decoded.Body.value > maxDirectTimeout {
		return fmt.Errorf("body must be no greater than %s", maxDirectTimeout)
	}
	*t = rawDirectTimeout{set: true, Dial: decoded.Dial, Header: decoded.Header, Body: decoded.Body}
	return nil
}

func decodeDirectTimeout(raw rawDirectTimeout) *directTimeout {
	if !raw.set {
		return nil
	}
	return &directTimeout{Dial: raw.Dial.value, Header: raw.Header.value, Body: raw.Body.value}
}

func (r *rawBackendRetry) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Status   optionalIntArray    `json:"status"`
		Methods  optionalStringArray `json:"methods"`
		Body     rawBackendBody      `json:"body"`
		Backoff  rawBackendBackoff   `json:"backoff"`
		Deadline optionalDuration    `json:"deadline"`
		Budget   rawBackendBudget    `json:"budget"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Status.set || len(decoded.Status.values) == 0 {
		return fmt.Errorf("status must be a non-empty array")
	}
	*r = rawBackendRetry{set: true, Status: decoded.Status, Methods: decoded.Methods, Body: decoded.Body, Backoff: decoded.Backoff, Deadline: decoded.Deadline, Budget: decoded.Budget}
	return nil
}

func (b *rawBackendBody) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Max optionalInt64 `json:"max"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Max.set || decoded.Max.value < 1 || decoded.Max.value > maxRetryBodyBytes {
		return fmt.Errorf("max must be an integer between 1 and %d", maxRetryBodyBytes)
	}
	*b = rawBackendBody{set: true, Max: decoded.Max}
	return nil
}

func (b *rawBackendBudget) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Max    optionalInt      `json:"max"`
		Window optionalDuration `json:"window"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Max.set || decoded.Max.value < 1 || decoded.Max.value > maxRetryBudget {
		return fmt.Errorf("max must be an integer between 1 and %d", maxRetryBudget)
	}
	if !decoded.Window.set {
		return fmt.Errorf("window must be set")
	}
	*b = rawBackendBudget{set: true, Max: decoded.Max, Window: decoded.Window}
	return nil
}

func (h *rawBackendHealth) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Fail  optionalInt           `json:"fail"`
		Cool  optionalDuration      `json:"cool"`
		Probe rawBackendHealthProbe `json:"probe"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Fail.set || decoded.Fail.value < 1 || decoded.Fail.value > maxHealthFailures {
		return fmt.Errorf("fail must be an integer between 1 and %d", maxHealthFailures)
	}
	if !decoded.Cool.set || decoded.Cool.value > maxHealthCooldown {
		return fmt.Errorf("cool must be greater than zero and no greater than %s", maxHealthCooldown)
	}
	*h = rawBackendHealth{set: true, Fail: decoded.Fail, Cool: decoded.Cool, Probe: decoded.Probe}
	return nil
}

func (p *rawBackendHealthProbe) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Path    optionalString   `json:"path"`
		Every   optionalDuration `json:"every"`
		Timeout optionalDuration `json:"timeout"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Path.set {
		return fmt.Errorf("path must be set")
	}
	path, err := parseHealthProbePath(decoded.Path.value)
	if err != nil {
		return fmt.Errorf("path: %w", err)
	}
	if !decoded.Every.set || decoded.Every.value > maxHealthProbeDuration {
		return fmt.Errorf("every must be no greater than %s", maxHealthProbeDuration)
	}
	if !decoded.Timeout.set {
		return fmt.Errorf("timeout must be set")
	}
	if decoded.Timeout.value > decoded.Every.value {
		return fmt.Errorf("timeout must be no greater than every")
	}
	*p = rawBackendHealthProbe{
		set:     true,
		Path:    path,
		Every:   decoded.Every.value,
		Timeout: decoded.Timeout.value,
	}
	return nil
}

func parseHealthProbePath(raw string) (url.URL, error) {
	if raw == "" {
		return url.URL{}, fmt.Errorf("must not be empty")
	}
	if len(raw) > maxHealthProbePathBytes {
		return url.URL{}, fmt.Errorf("must not exceed %d bytes", maxHealthProbePathBytes)
	}
	if !strings.HasPrefix(raw, "/") || strings.HasPrefix(raw, "//") || strings.Contains(raw, "#") {
		return url.URL{}, fmt.Errorf("must be an origin-form request-target")
	}
	for index := 0; index < len(raw); index++ {
		if raw[index] <= ' ' || raw[index] == 0x7f {
			return url.URL{}, fmt.Errorf("must not contain spaces or control characters")
		}
	}
	parsed, err := url.ParseRequestURI(raw)
	if err != nil {
		return url.URL{}, fmt.Errorf("must be a valid request-target: %w", err)
	}
	if parsed.IsAbs() || parsed.Host != "" || parsed.Fragment != "" || parsed.RawFragment != "" || parsed.Opaque != "" {
		return url.URL{}, fmt.Errorf("must be an origin-form request-target")
	}
	return *parsed, nil
}

func (b *rawBackendBackoff) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Base   optionalDuration `json:"base"`
		Cap    optionalDuration `json:"cap"`
		Jitter optionalString   `json:"jitter"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Base.set || !decoded.Cap.set {
		return fmt.Errorf("base and cap must be set")
	}
	if decoded.Cap.value < decoded.Base.value {
		return fmt.Errorf("cap must be greater than or equal to base")
	}
	jitter := "none"
	if decoded.Jitter.set {
		jitter = decoded.Jitter.value
	}
	if jitter != "none" && jitter != "full" {
		return fmt.Errorf("jitter must be none or full")
	}
	*b = rawBackendBackoff{set: true, Base: decoded.Base, Cap: decoded.Cap, Jitter: optionalString{set: true, value: jitter}}
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

func (m *rawRouteMatch) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var decoded struct {
		Header optionalConditionValues `json:"header"`
		Query  optionalConditionValues `json:"query"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return err
	}
	if !decoded.Header.set && !decoded.Query.set {
		return fmt.Errorf("must contain header or query")
	}
	if decoded.Header.set && len(decoded.Header.values) == 0 {
		return fmt.Errorf("header must not be empty")
	}
	if decoded.Query.set && len(decoded.Query.values) == 0 {
		return fmt.Errorf("query must not be empty")
	}
	*m = rawRouteMatch{set: true, Header: decoded.Header, Query: decoded.Query}
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

func (v *optionalConditionValues) UnmarshalJSON(data []byte) error {
	var values conditionValues
	if err := json.Unmarshal(data, &values); err != nil {
		return err
	}
	v.set = true
	v.values = values
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

func (i *optionalInt64) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if bytes.Equal(data, []byte("null")) || bytes.ContainsAny(data, ".eE") {
		return fmt.Errorf("must be an integer without a decimal point or exponent")
	}
	var value int64
	if err := json.Unmarshal(data, &value); err != nil {
		return fmt.Errorf("must be an integer without a decimal point or exponent")
	}
	i.set = true
	i.value = value
	return nil
}

func (a *optionalIntArray) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("must be an array of integers")
	}
	var raw []json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return fmt.Errorf("must be an array of integers")
	}
	values := make([]int, len(raw))
	for index, rawValue := range raw {
		var value optionalInt
		if err := value.UnmarshalJSON(rawValue); err != nil {
			return fmt.Errorf("item %d must be an integer without a decimal point or exponent", index)
		}
		values[index] = value.value
	}
	a.set = true
	a.values = values
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
	if raw.TLS.set {
		cfg.TLS = &tlsConfig{Cert: raw.TLS.Cert, Key: raw.TLS.Key}
	}
	for name, rawBackend := range raw.Backends {
		if name == "" {
			return config{}, fmt.Errorf("backend name must not be empty")
		}
		if rawBackend.Targets == nil || len(*rawBackend.Targets) == 0 {
			return config{}, fmt.Errorf("backends[%q].targets must not be empty", name)
		}
		var weights []int
		if rawBackend.Weights.set {
			weights = append([]int{}, rawBackend.Weights.values...)
			if err := validateTargetWeights(weights, len(*rawBackend.Targets)); err != nil {
				return config{}, fmt.Errorf("backends[%q].weights: %w", name, err)
			}
		}
		tries := 1
		if rawBackend.Tries.set {
			tries = rawBackend.Tries.value
			if tries < 1 || tries > len(*rawBackend.Targets) {
				return config{}, fmt.Errorf("backends[%q].tries must be between 1 and the target count", name)
			}
		}
		var retry *backendRetryConfig
		if rawBackend.Retry.set {
			if !rawBackend.Tries.set || tries <= 1 {
				return config{}, fmt.Errorf("backends[%q].retry requires tries greater than one", name)
			}
			if err := validateRetryStatuses(rawBackend.Retry.Status.values); err != nil {
				return config{}, fmt.Errorf("backends[%q].retry.status: %w", name, err)
			}
			if rawBackend.Retry.Methods.set {
				if err := validateRetryMethods(rawBackend.Retry.Methods.values); err != nil {
					return config{}, fmt.Errorf("backends[%q].retry.methods: %w", name, err)
				}
			}
			retry = &backendRetryConfig{
				Status:   append([]int(nil), rawBackend.Retry.Status.values...),
				Methods:  append([]string(nil), rawBackend.Retry.Methods.values...),
				Deadline: rawBackend.Retry.Deadline.value,
			}
			if rawBackend.Retry.Body.set {
				retry.Body = &backendBodyConfig{Max: rawBackend.Retry.Body.Max.value}
			}
			if rawBackend.Retry.Budget.set {
				retry.Budget = &backendBudgetConfig{Max: rawBackend.Retry.Budget.Max.value, Window: rawBackend.Retry.Budget.Window.value}
			}
			if rawBackend.Retry.Backoff.set {
				retry.Backoff = &backendBackoffConfig{
					Base:   rawBackend.Retry.Backoff.Base.value,
					Cap:    rawBackend.Retry.Backoff.Cap.value,
					Jitter: rawBackend.Retry.Backoff.Jitter.value == "full",
				}
			}
		}
		backendTLS, err := decodeUpstreamTLS(rawBackend.TLS)
		if err != nil {
			return config{}, fmt.Errorf("backends[%q].tls: %w", name, err)
		}
		cfg.Backends[name] = backendConfig{
			Targets: append([]string(nil), (*rawBackend.Targets)...),
			Weights: weights,
			Tries:   tries,
			Timeout: backendTimeout{
				Dial:   rawBackend.Timeout.Dial.value,
				Header: rawBackend.Timeout.Header.value,
			},
			TLS: backendTLS,
			Health: func() *backendHealthConfig {
				if !rawBackend.Health.set {
					return nil
				}
				health := &backendHealthConfig{Fail: rawBackend.Health.Fail.value, Cool: rawBackend.Health.Cool.value}
				if rawBackend.Health.Probe.set {
					health.Probe = &backendHealthProbeConfig{
						Path:    rawBackend.Health.Probe.Path,
						Every:   rawBackend.Health.Probe.Every,
						Timeout: rawBackend.Health.Probe.Timeout,
					}
				}
				return health
			}(),
			Retry: retry,
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
		if rawSite.Backend.set && rawSite.Timeout.set {
			return config{}, fmt.Errorf("sites[%d].timeout is not allowed with backend", siteIndex)
		}
		siteTLS, err := decodeUpstreamTLS(rawSite.TLS)
		if err != nil {
			return config{}, fmt.Errorf("sites[%d].tls: %w", siteIndex, err)
		}

		site := siteConfig{
			Hosts:           append([]string(nil), (*rawSite.Hosts)...),
			Timeout:         decodeDirectTimeout(rawSite.Timeout),
			TLS:             siteTLS,
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
			usesBackend := rawRoute.Backend.set || (!rawRoute.Target.set && rawSite.Backend.set)
			if rawRoute.Timeout.set && usesBackend {
				return config{}, fmt.Errorf("sites[%d].routes[%d].timeout is not allowed with backend", siteIndex, routeIndex)
			}
			routeTLS, err := decodeUpstreamTLS(rawRoute.TLS)
			if err != nil {
				return config{}, fmt.Errorf("sites[%d].routes[%d].tls: %w", siteIndex, routeIndex, err)
			}
			route := routeConfig{
				Path:            *rawRoute.Path,
				Methods:         append([]string(nil), rawRoute.Methods.values...),
				Timeout:         decodeDirectTimeout(rawRoute.Timeout),
				TLS:             routeTLS,
				Headers:         map[string]string(rawRoute.Headers),
				ResponseHeaders: rawRoute.ResponseHeaders,
				Strip:           bool(rawRoute.Strip),
			}
			if rawRoute.Match.set {
				route.Match = &routeMatchConfig{
					Header: cloneConditionValues(rawRoute.Match.Header.values),
					Query:  cloneConditionValues(rawRoute.Match.Query.values),
				}
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
	jsonBackendRetry
	jsonBackendBody
	jsonBackendBudget
	jsonBackendHealth
	jsonBackendProbe
	jsonBackendBackoff
	jsonSites
	jsonSite
	jsonRoutes
	jsonRoute
	jsonHeaders
	jsonResponseHeaders
	jsonBackendTimeout
	jsonDirectTimeout
	jsonRouteMatch
	jsonConditions
	jsonTLS
	jsonUpstreamTLS
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
		case "tls":
			return jsonTLS, nil
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
		case "targets", "weights", "tries":
			return jsonAny, nil
		case "tls":
			return jsonUpstreamTLS, nil
		case "retry":
			return jsonBackendRetry, nil
		case "timeout":
			return jsonBackendTimeout, nil
		case "health":
			return jsonBackendHealth, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendRetry:
		switch key {
		case "status", "methods", "deadline":
			return jsonAny, nil
		case "body":
			return jsonBackendBody, nil
		case "budget":
			return jsonBackendBudget, nil
		case "backoff":
			return jsonBackendBackoff, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendBody:
		if key == "max" {
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendBudget:
		switch key {
		case "max", "window":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendHealth:
		switch key {
		case "fail", "cool":
			return jsonAny, nil
		case "probe":
			return jsonBackendProbe, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendProbe:
		switch key {
		case "path", "every", "timeout":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendBackoff:
		switch key {
		case "base", "cap", "jitter":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonBackendTimeout:
		switch key {
		case "dial", "header":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonDirectTimeout:
		switch key {
		case "dial", "header", "body":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonSite:
		switch key {
		case "hosts", "target", "backend":
			return jsonAny, nil
		case "tls":
			return jsonUpstreamTLS, nil
		case "timeout":
			return jsonDirectTimeout, nil
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
		case "tls":
			return jsonUpstreamTLS, nil
		case "timeout":
			return jsonDirectTimeout, nil
		case "match":
			return jsonRouteMatch, nil
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
	case jsonRouteMatch:
		switch key {
		case "header", "query":
			return jsonConditions, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonConditions:
		return jsonAny, nil
	case jsonTLS:
		switch key {
		case "cert", "key":
			return jsonAny, nil
		}
		return jsonAny, fmt.Errorf("unknown field %q", key)
	case jsonUpstreamTLS:
		switch key {
		case "ca", "name":
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

func (m *conditionValues) UnmarshalJSON(data []byte) error {
	data = bytes.TrimSpace(data)
	if len(data) == 0 || data[0] != '{' {
		return fmt.Errorf("must be an object")
	}
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return fmt.Errorf("must be an object: %w", err)
	}
	values := make(conditionValues, len(raw))
	for name, encoded := range raw {
		encoded = bytes.TrimSpace(encoded)
		if len(encoded) > 0 && encoded[0] == '"' {
			var value string
			if err := json.Unmarshal(encoded, &value); err != nil {
				return fmt.Errorf("property %q must be a string or non-empty array of strings", name)
			}
			values[name] = []string{value}
			continue
		}

		var alternatives stringArray
		if err := json.Unmarshal(encoded, &alternatives); err != nil {
			return fmt.Errorf("property %q must be a string or non-empty array of strings: %w", name, err)
		}
		if len(alternatives) == 0 {
			return fmt.Errorf("property %q alternative array must not be empty", name)
		}
		seen := make(map[string]struct{}, len(alternatives))
		for index, alternative := range alternatives {
			if _, exists := seen[alternative]; exists {
				return fmt.Errorf("property %q contains duplicate alternative at index %d", name, index)
			}
			seen[alternative] = struct{}{}
		}
		values[name] = []string(alternatives)
	}
	*m = values
	return nil
}

func cloneConditionValues(values conditionValues) map[string][]string {
	if len(values) == 0 {
		return nil
	}
	cloned := make(map[string][]string, len(values))
	for name, alternatives := range values {
		cloned[name] = append([]string(nil), alternatives...)
	}
	return cloned
}

func resolveRouteMatch(raw *routeMatchConfig) (routeMatchConfig, error) {
	if raw == nil {
		return routeMatchConfig{}, nil
	}
	if raw.Header != nil && len(raw.Header) == 0 {
		return routeMatchConfig{}, fmt.Errorf("header must not be empty")
	}
	if raw.Query != nil && len(raw.Query) == 0 {
		return routeMatchConfig{}, fmt.Errorf("query must not be empty")
	}
	if len(raw.Header) == 0 && len(raw.Query) == 0 {
		return routeMatchConfig{}, fmt.Errorf("must contain header or query")
	}

	match := routeMatchConfig{}
	if len(raw.Header) > 0 {
		match.Header = make(map[string][]string, len(raw.Header))
		for name, alternatives := range raw.Header {
			if !validHeaderName(name) {
				return routeMatchConfig{}, fmt.Errorf("header property %q is not a valid field name", name)
			}
			name = http.CanonicalHeaderKey(name)
			if _, exists := match.Header[name]; exists {
				return routeMatchConfig{}, fmt.Errorf("header contains duplicate field name %q", name)
			}
			if err := validateMatchAlternatives(alternatives, true); err != nil {
				return routeMatchConfig{}, fmt.Errorf("header property %q: %w", name, err)
			}
			match.Header[name] = append([]string(nil), alternatives...)
		}
	}
	if len(raw.Query) > 0 {
		match.Query = make(map[string][]string, len(raw.Query))
		for name, alternatives := range raw.Query {
			if name == "" {
				return routeMatchConfig{}, fmt.Errorf("query property name must not be empty")
			}
			if err := validateMatchAlternatives(alternatives, false); err != nil {
				return routeMatchConfig{}, fmt.Errorf("query property %q: %w", name, err)
			}
			match.Query[name] = append([]string(nil), alternatives...)
		}
	}
	return match, nil
}

func validateMatchAlternatives(alternatives []string, header bool) error {
	if len(alternatives) == 0 {
		return fmt.Errorf("alternative array must not be empty")
	}
	seen := make(map[string]struct{}, len(alternatives))
	for index, alternative := range alternatives {
		if header && !validHeaderValue(alternative) {
			return fmt.Errorf("alternative %d contains invalid control characters", index)
		}
		if _, exists := seen[alternative]; exists {
			return fmt.Errorf("duplicate alternative at index %d", index)
		}
		seen[alternative] = struct{}{}
	}
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

func validateRetryMethods(methods []string) error {
	if err := validateMethods(methods); err != nil {
		return err
	}
	for index, method := range methods {
		if strings.Contains(method, "*") {
			return fmt.Errorf("item %d must not contain *", index)
		}
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
