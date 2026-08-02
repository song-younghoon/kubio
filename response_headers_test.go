package main

import (
	"io"
	"net/http"
	"net/http/httptest"
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

	valid := withSite(`"responseHeaders":{"set":{"X-Site":"${KUBIO_V04_VALUE}"}},"routes":[{"path":"/*","responseHeaders":{"set":{"X-Route":"\\${KUBIO_V04_VALUE}"}}}]`)
	if err := validate(valid); err != nil {
		t.Fatalf("valid responseHeaders rejected: %v", err)
	}

	invalid := map[string]string{
		"outside site or route": `{"listen":":8080","responseHeaders":{"set":{"X-Test":"x"}},"sites":[{"hosts":["*"],"target":"http://localhost:3000"}]}`,
		"backend field":         `{"listen":":8080","backends":{"app":{"targets":["http://localhost:3000"],"responseHeaders":{"set":{"X-Test":"x"}}}},"sites":[{"hosts":["*"],"backend":"app"}]}`,
		"null object":           withSite(`"responseHeaders":null`),
		"non-object":            withSite(`"responseHeaders":[]`),
		"missing set":           withSite(`"responseHeaders":{}`),
		"null set":              withSite(`"responseHeaders":{"set":null}`),
		"empty set":             withSite(`"responseHeaders":{"set":{}}`),
		"non-object set":        withSite(`"responseHeaders":{"set":[]}`),
		"non-string value":      withSite(`"responseHeaders":{"set":{"X-Test":1}}`),
		"unknown field":         withSite(`"responseHeaders":{"set":{"X-Test":"x"},"extra":true}`),
		"duplicate field":       withSite(`"responseHeaders":{"set":{"X-Test":"x"}},"responseHeaders":{"set":{"X-Test":"y"}}`),
		"duplicate set key":     withSite(`"responseHeaders":{"set":{"X-Test":"x","X-Test":"y"}}`),
		"duplicate header case": withSite(`"responseHeaders":{"set":{"X-Test":"x","x-test":"y"}}`),
		"invalid header name":   withSite(`"responseHeaders":{"set":{"X Test":"x"}}`),
		"unset environment":     withSite(`"responseHeaders":{"set":{"X-Test":"${KUBIO_V04_MISSING_7E83F4}"}}`),
	}
	for name, data := range invalid {
		t.Run(name, func(t *testing.T) {
			if err := validate(data); err == nil {
				t.Fatal("invalid responseHeaders accepted")
			}
		})
	}

	for _, name := range []string{
		"Connection", "Proxy-Connection", "Keep-Alive", "Proxy-Authenticate",
		"Proxy-Authorization", "Transfer-Encoding", "TE", "Trailer", "Upgrade", "Content-Length",
	} {
		t.Run("managed "+name, func(t *testing.T) {
			data := withSite(`"responseHeaders":{"set":{"` + strings.ToLower(name) + `":"x"}}`)
			if err := validate(data); err == nil {
				t.Fatal("managed response header accepted")
			}
		})
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
		ResponseHeaders: map[string]string{
			"X-Site":     "site",
			"x-shared":   "site",
			"X-Expanded": "${KUBIO_V04_VALUE}",
			"X-Literal":  `\${KUBIO_V04_VALUE}`,
			"X-Empty":    "",
		},
		Routes: []routeConfig{
			{Path: "/api/*", Methods: []string{http.MethodPost}, ResponseHeaders: map[string]string{"X-Unselected": "post"}},
			{Path: "/api/*", Methods: []string{http.MethodGet}, ResponseHeaders: map[string]string{"X-SHARED": "route", "X-Route": "route"}},
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
		get.Header().Get("X-Unselected") != "upstream" || get.Header().Get("X-Expanded") != "expanded" ||
		get.Header().Get("X-Literal") != "${KUBIO_V04_VALUE}" {
		t.Fatalf("GET headers = %v", get.Header())
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
		ResponseHeaders: map[string]string{"X-Test": "${KUBIO_V04_BAD}"},
	}}})
	if err == nil || strings.Contains(err.Error(), "secret") {
		t.Fatalf("expanded invalid value leaked or was accepted: %v", err)
	}
}

func TestResponseHeadersRespectTrailerAnnouncements(t *testing.T) {
	response := &http.Response{
		Header: http.Header{
			"X-Collision": {"initial-a", "initial-b"},
			"X-Replace":   {"upstream-a", "upstream-b"},
			"x-replace":   {"upstream-lowercase"},
		},
		Trailer: http.Header{"x-collision": nil},
	}
	replaceResponseHeaders(response, map[string]string{
		"X-Collision": "configured",
		"X-Replace":   "",
		"X-Late":      "configured",
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
			ResponseHeaders: map[string]string{"X-Policy": "configured"},
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
