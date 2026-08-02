package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"
)

func TestResponseHeadersConfigIsStrict(t *testing.T) {
	t.Setenv("KUBIO_V04_VALUE", "expanded")
	withSite := func(fields string) string {
		return `{"listen":":8080","sites":[{"hosts":["*"],"target":"http://localhost:3000",` + fields + `}]}`
	}
	validate := func(data string) error {
		cfg, err := decodeConfig([]byte(data))
		if err != nil {
			return err
		}
		_, err = newRouter(cfg)
		return err
	}

	valid := withSite(`"response":{"remove":["X-Old"],"set":{"X-Site":"${KUBIO_V04_VALUE}","X-Shared":["one","two"]},"add":{"x-shared":"three","X-Literal":["\\${KUBIO_V04_VALUE}"]}},"routes":[{"path":"/*","response":{"remove":["X-Route"],"add":{"x-route":"route"}}}]`)
	if err := validate(valid); err != nil {
		t.Fatalf("valid response rejected: %v", err)
	}
	decoded, err := decodeConfig([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	resolved, err := resolveResponseHeaders(decoded.Sites[0].ResponseHeaders)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(resolved.Set["X-Shared"], []string{"one", "two"}) ||
		!slices.Equal(resolved.Add["X-Shared"], []string{"three"}) ||
		!slices.Equal(resolved.Add["X-Literal"], []string{"${KUBIO_V04_VALUE}"}) ||
		!slices.Equal(resolved.Remove, []string{"X-Old"}) {
		t.Fatalf("resolved policy = %+v", resolved)
	}

	invalid := map[string]string{
		"outside site or route": `{"listen":":8080","response":{"set":{"X-Test":"x"}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"backend field":         `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"response":{"set":{"X-Test":"x"}}}},"sites":[{"hosts":["*"],"backend":"app"}]}`,
		"old responseHeaders":   withSite(`"responseHeaders":{"set":{"X-Test":"x"}}`),
		"old and new fields":    withSite(`"response":{"set":{"X-Test":"x"}},"responseHeaders":{"set":{"X-Test":"y"}}`),
		"old route field":       withSite(`"routes":[{"path":"/*","responseHeaders":{"set":{"X-Test":"x"}}}]`),
		"null object":           withSite(`"response":null`),
		"non-object":            withSite(`"response":[]`),
		"empty object":          withSite(`"response":{}`),
		"null set":              withSite(`"response":{"set":null}`),
		"empty set":             withSite(`"response":{"set":{}}`),
		"non-object set":        withSite(`"response":{"set":[]}`),
		"non-string value":      withSite(`"response":{"set":{"X-Test":1}}`),
		"empty value array":     withSite(`"response":{"set":{"X-Test":[]}}`),
		"null array item":       withSite(`"response":{"set":{"X-Test":["x",null]}}`),
		"nested value array":    withSite(`"response":{"set":{"X-Test":[["x"]]}}`),
		"null add":              withSite(`"response":{"add":null}`),
		"empty add":             withSite(`"response":{"add":{}}`),
		"non-object add":        withSite(`"response":{"add":[]}`),
		"duplicate add case":    withSite(`"response":{"add":{"X-Test":"x","x-test":"y"}}`),
		"null remove":           withSite(`"response":{"remove":null}`),
		"empty remove":          withSite(`"response":{"remove":[]}`),
		"non-array remove":      withSite(`"response":{"remove":"X-Test"}`),
		"non-string remove":     withSite(`"response":{"remove":[1]}`),
		"duplicate remove case": withSite(`"response":{"remove":["X-Test","x-test"]}`),
		"unknown field":         withSite(`"response":{"set":{"X-Test":"x"},"extra":true}`),
		"duplicate field":       withSite(`"response":{"set":{"X-Test":"x"}},"response":{"set":{"X-Test":"y"}}`),
		"duplicate set key":     withSite(`"response":{"set":{"X-Test":"x","X-Test":"y"}}`),
		"duplicate header case": withSite(`"response":{"set":{"X-Test":"x","x-test":"y"}}`),
		"invalid header name":   withSite(`"response":{"set":{"X Test":"x"}}`),
		"unset environment":     withSite(`"response":{"set":{"X-Test":"${KUBIO_V04_MISSING_7E83F4}"}}`),
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if err := validate(data); err == nil {
				t.Fatal("invalid response accepted")
			}
		})
	}

	for _, name := range []string{
		"Connection", "Proxy-Connection", "Keep-Alive", "Proxy-Authenticate",
		"Proxy-Authorization", "Transfer-Encoding", "TE", "Trailer", "Upgrade", "Content-Length",
	} {
		for operation, value := range map[string]string{
			"set":    `{"` + strings.ToLower(name) + `":"x"}`,
			"add":    `{"` + strings.ToLower(name) + `":"x"}`,
			"remove": `["` + strings.ToLower(name) + `"]`,
		} {
			t.Run("managed "+operation+" "+name, func(t *testing.T) {
				data := withSite(`"response":{"` + operation + `":` + value + `}`)
				if err := validate(data); err == nil {
					t.Fatal("managed response header accepted")
				}
			})
		}
	}

	if _, err := newRouter(config{Sites: []siteConfig{{
		Hosts: []string{"*"}, Target: "http://localhost:3000",
		Headers: map[string]string{"Proxy-Authenticate": "request-ok", "Proxy-Authorization": "request-ok"},
	}}}); err != nil {
		t.Fatalf("response-only restrictions changed request headers: %v", err)
	}
}

func TestResponseHeadersInheritanceOverrideAndEnvironment(t *testing.T) {
	t.Setenv("KUBIO_V04_VALUE", "expanded")
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header()["X-Shared"] = []string{"upstream-a", "upstream-b"}
		w.Header().Set("X-Site", "upstream")
		w.Header().Set("X-Route", "upstream")
		w.Header().Set("X-Unselected", "upstream")
		w.WriteHeader(http.StatusTeapot)
		_, _ = io.WriteString(w, "body")
	}))
	defer backend.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: backend.URL,
		ResponseHeaders: responseHeaderPolicy{Set: map[string][]string{
			"X-Site":     {"site"},
			"x-shared":   {"site"},
			"X-Expanded": {"before", "${KUBIO_V04_VALUE}", "after"},
			"X-Literal":  {`\${KUBIO_V04_VALUE}`},
			"X-Empty":    {""},
		}},
		Routes: []routeConfig{
			{Path: "/api/*", Methods: []string{http.MethodPost}, ResponseHeaders: responseHeaderPolicy{Set: map[string][]string{"X-Unselected": {"post"}}}},
			{Path: "/api/*", Methods: []string{http.MethodGet}, ResponseHeaders: responseHeaderPolicy{Set: map[string][]string{"X-SHARED": {"route"}, "X-Route": {"route"}}}},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}

	request := func(method string) *httptest.ResponseRecorder {
		res := httptest.NewRecorder()
		handler.ServeHTTP(res, httptest.NewRequest(method, "http://proxy/api/value", nil))
		return res
	}

	get := request(http.MethodGet)
	if get.Code != http.StatusTeapot || get.Body.String() != "body" {
		t.Fatalf("response = %d %q", get.Code, get.Body.String())
	}
	if got := get.Header().Values("X-Shared"); len(got) != 1 || got[0] != "route" {
		t.Fatalf("route override = %q", got)
	}
	if get.Header().Get("X-Site") != "site" || get.Header().Get("X-Route") != "route" ||
		get.Header().Get("X-Unselected") != "upstream" ||
		get.Header().Get("X-Literal") != "${KUBIO_V04_VALUE}" {
		t.Fatalf("GET headers = %v", get.Header())
	}
	if got, want := get.Header().Values("X-Expanded"), []string{"before", "expanded", "after"}; !slices.Equal(got, want) {
		t.Fatalf("expanded values = %q, want %q", got, want)
	}
	if values, ok := get.Header()["X-Empty"]; !ok || len(values) != 1 || values[0] != "" {
		t.Fatalf("empty header = %q, present=%t", values, ok)
	}

	other := request(http.MethodDelete)
	if got := other.Header().Values("X-Shared"); len(got) != 1 || got[0] != "site" {
		t.Fatalf("site-only override = %q", got)
	}
	if other.Header().Get("X-Route") != "upstream" || other.Header().Get("X-Unselected") != "upstream" {
		t.Fatalf("method-ineligible route applied: %v", other.Header())
	}

	t.Setenv("KUBIO_V04_BAD", "secret\nvalue")
	_, err = newRouter(config{Sites: []siteConfig{{
		Hosts: []string{"*"}, Target: backend.URL,
		ResponseHeaders: responseHeaderPolicy{Add: map[string][]string{"X-Test": {"safe", "${KUBIO_V04_BAD}"}}},
	}}})
	if err == nil || strings.Contains(err.Error(), "secret") || strings.Contains(err.Error(), "safe") {
		t.Fatalf("expanded invalid value leaked or was accepted: %v", err)
	}
}

func TestResponseHeaderOperationOrdering(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header()["x-order"] = []string{"upstream-1", "upstream-2"}
		w.Header()["x-append"] = []string{"upstream-1", "upstream-2"}
		w.Header().Set("X-Site-Remove", "upstream")
		w.Header().Set("X-Route-Remove", "upstream")
		w.Header().Set("X-Unselected", "upstream")
		w.Header()["Set-Cookie"] = []string{"upstream=1", "upstream=2"}
		w.WriteHeader(http.StatusCreated)
	}))
	defer backend.Close()

	handler, err := newRouter(config{Sites: []siteConfig{{
		Hosts:  []string{"*"},
		Target: backend.URL,
		ResponseHeaders: responseHeaderPolicy{
			Remove: []string{"X-Site-Remove", "X-Order"},
			Set: map[string][]string{
				"X-Order":    {"site-set-1", "site-set-2"},
				"Set-Cookie": {"site=1", "site=2"},
			},
			Add: map[string][]string{
				"X-Append":   {"site-add"},
				"X-Order":    {"site-add"},
				"Set-Cookie": {"site=3"},
			},
		},
		Routes: []routeConfig{
			{
				Path:    "/api/*",
				Methods: []string{http.MethodPost},
				ResponseHeaders: responseHeaderPolicy{Add: map[string][]string{
					"X-Unselected": {"post"},
				}},
			},
			{
				Path:    "/api/*",
				Methods: []string{http.MethodGet},
				ResponseHeaders: responseHeaderPolicy{
					Remove: []string{"X-Route-Remove", "X-Order"},
					Set:    map[string][]string{"X-Order": {"route-set"}},
					Add: map[string][]string{
						"X-Append":   {"route-add"},
						"X-Order":    {"route-add-1", "route-add-2"},
						"Set-Cookie": {"route=1"},
					},
				},
			},
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}

	request := func(method string) *httptest.ResponseRecorder {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(method, "http://proxy/api/value", nil))
		return response
	}

	get := request(http.MethodGet)
	if get.Code != http.StatusCreated {
		t.Fatalf("status = %d", get.Code)
	}
	if got, want := get.Header().Values("X-Order"), []string{"route-set", "route-add-1", "route-add-2"}; !slices.Equal(got, want) {
		t.Fatalf("X-Order = %q, want %q", got, want)
	}
	if got, want := get.Header().Values("Set-Cookie"), []string{"site=1", "site=2", "site=3", "route=1"}; !slices.Equal(got, want) {
		t.Fatalf("Set-Cookie = %q, want %q", got, want)
	}
	if got, want := get.Header().Values("X-Append"), []string{"upstream-1", "upstream-2", "site-add", "route-add"}; !slices.Equal(got, want) {
		t.Fatalf("X-Append = %q, want %q", got, want)
	}
	if get.Header().Get("X-Site-Remove") != "" || get.Header().Get("X-Route-Remove") != "" {
		t.Fatalf("removed headers remain: %v", get.Header())
	}
	if got := get.Header().Values("X-Unselected"); len(got) != 1 || got[0] != "upstream" {
		t.Fatalf("method-ineligible policy applied: %q", got)
	}

	other := request(http.MethodDelete)
	if got, want := other.Header().Values("X-Order"), []string{"site-set-1", "site-set-2", "site-add"}; !slices.Equal(got, want) {
		t.Fatalf("site-only X-Order = %q, want %q", got, want)
	}
	if other.Header().Get("X-Route-Remove") != "upstream" {
		t.Fatalf("unselected route removed header: %v", other.Header())
	}
}

func TestResponseHeadersRespectTrailerAnnouncements(t *testing.T) {
	response := &http.Response{
		Header: http.Header{
			"X-Collision": {"initial-a", "initial-b"},
			"X-Replace":   {"upstream-a", "upstream-b"},
		},
		Trailer: http.Header{"x-collision": nil},
	}
	applyResponseHeaders(response, responseHeaderPolicy{
		Remove: []string{"X-Collision"},
		Set: map[string][]string{
			"X-Collision": {"site-set"},
			"X-Replace":   {""},
			"X-Late":      {"configured"},
		},
		Add: map[string][]string{"X-Collision": {"site-add"}},
	})
	applyResponseHeaders(response, responseHeaderPolicy{
		Remove: []string{"X-Collision"},
		Set:    map[string][]string{"X-Collision": {"route-set"}},
		Add:    map[string][]string{"X-Collision": {"route-add"}},
	})

	if got := response.Header.Values("X-Collision"); len(got) != 2 || got[0] != "initial-a" || got[1] != "initial-b" {
		t.Fatalf("announced trailer collision changed initial values: %q", got)
	}
	matchingKeys := 0
	for name := range response.Header {
		if strings.EqualFold(name, "X-Replace") {
			matchingKeys++
		}
	}
	if got := response.Header.Values("X-Replace"); matchingKeys != 1 || len(got) != 1 || got[0] != "" {
		t.Fatalf("replacement keys = %d, values = %q", matchingKeys, got)
	}
	response.Trailer["X-Late"] = []string{"late"}
	if response.Header.Get("X-Late") != "configured" || response.Trailer.Get("X-Late") != "late" {
		t.Fatalf("unannounced trailer changed prior decision: header=%q trailer=%q", response.Header.Get("X-Late"), response.Trailer.Get("X-Late"))
	}
}

func TestResponseHeadersApplyOnlyToForwardedResponses(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("X-Policy", "upstream")
		w.WriteHeader(http.StatusBadGateway)
		_, _ = io.WriteString(w, "upstream error")
	}))
	defer backend.Close()

	build := func(host, target string) *router {
		handler, err := newRouter(config{Sites: []siteConfig{{
			Hosts: []string{host}, Target: target,
			ResponseHeaders: responseHeaderPolicy{Set: map[string][]string{"X-Policy": {"configured"}}},
		}}})
		if err != nil {
			t.Fatal(err)
		}
		return handler
	}

	forwarded := httptest.NewRecorder()
	build("*", backend.URL).ServeHTTP(forwarded, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if forwarded.Code != http.StatusBadGateway || forwarded.Header().Get("X-Policy") != "configured" || forwarded.Body.String() != "upstream error" {
		t.Fatalf("forwarded response = %d %q %q", forwarded.Code, forwarded.Header().Get("X-Policy"), forwarded.Body.String())
	}

	closed := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	closedURL := closed.URL
	closed.Close()
	generated := httptest.NewRecorder()
	build("*", closedURL).ServeHTTP(generated, httptest.NewRequest(http.MethodGet, "http://proxy/", nil))
	if generated.Code != http.StatusBadGateway || generated.Header().Get("X-Policy") != "" {
		t.Fatalf("proxy-generated response = %d %q", generated.Code, generated.Header().Get("X-Policy"))
	}

	notFound := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodGet, "http://proxy/", nil)
	request.Host = "other.example"
	build("example.com", backend.URL).ServeHTTP(notFound, request)
	if notFound.Code != http.StatusNotFound || notFound.Header().Get("X-Policy") != "" {
		t.Fatalf("not-found response = %d %q", notFound.Code, notFound.Header().Get("X-Policy"))
	}
}
