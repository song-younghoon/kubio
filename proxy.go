package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptrace"
	"net/http/httputil"
	"net/netip"
	"net/textproto"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const (
	proxyDialTimeout           = 5 * time.Second
	proxyResponseHeaderTimeout = 30 * time.Second
	proxyBufferSize            = 32 * 1024
	proxyMaxIdleConnsPerHost   = 32
)

var proxyTransport = newProxyTransport(proxyDialTimeout, proxyResponseHeaderTimeout)

func newProxyTransport(dialTimeout, responseHeaderTimeout time.Duration) *http.Transport {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.DialContext = (&net.Dialer{Timeout: dialTimeout}).DialContext
	transport.TLSHandshakeTimeout = dialTimeout
	transport.ResponseHeaderTimeout = responseHeaderTimeout
	transport.MaxIdleConnsPerHost = proxyMaxIdleConnsPerHost
	return transport
}

var proxyBuffers proxyBufferPool

type proxyBufferPool struct {
	pool sync.Pool
}

type backend struct {
	targets       []*url.URL
	tries         int
	retryStatuses map[int]struct{}
	retryDelay    time.Duration
	retryDeadline time.Duration
	transport     *http.Transport
	nextIndex     atomic.Uint64
}

type backendRetryKey struct{}

type backendRetryState struct {
	start         int
	informational atomic.Bool
}

type deadlineBody struct {
	io.ReadCloser
	ctx    context.Context
	cancel func()
	once   sync.Once
}

func (b *deadlineBody) Read(p []byte) (int, error) {
	n, err := b.ReadCloser.Read(p)
	if err != nil {
		b.once.Do(b.cancel)
	}
	return n, err
}

func (b *deadlineBody) Close() error {
	err := b.ReadCloser.Close()
	b.once.Do(b.cancel)
	return err
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

func validateRetryStatuses(statuses []int) error {
	if len(statuses) == 0 {
		return fmt.Errorf("must not be empty")
	}
	var seen [200]bool
	for index, status := range statuses {
		if status < 400 || status > 599 {
			return fmt.Errorf("item %d must be between 400 and 599", index)
		}
		statusIndex := status - 400
		if seen[statusIndex] {
			return fmt.Errorf("item %d duplicates status %d", index, status)
		}
		seen[statusIndex] = true
	}
	return nil
}

func buildRetryStatuses(statuses []int) (map[int]struct{}, error) {
	if err := validateRetryStatuses(statuses); err != nil {
		return nil, err
	}
	set := make(map[int]struct{}, len(statuses))
	for _, status := range statuses {
		set[status] = struct{}{}
	}
	return set, nil
}

func newBackend(cfg backendConfig) (*backend, error) {
	if len(cfg.Targets) == 0 {
		return nil, fmt.Errorf("targets must not be empty")
	}
	tries := cfg.Tries
	if tries == 0 {
		tries = 1
	}
	if tries < 1 || tries > len(cfg.Targets) {
		return nil, fmt.Errorf("tries must be between 1 and the target count")
	}
	var retryStatuses map[int]struct{}
	if cfg.Retry != nil {
		if tries <= 1 {
			return nil, fmt.Errorf("retry requires tries greater than one")
		}
		if cfg.Retry.Delay < 0 {
			return nil, fmt.Errorf("retry.delay must be greater than zero")
		}
		if cfg.Retry.Deadline < 0 {
			return nil, fmt.Errorf("retry.deadline must be greater than zero")
		}
		var err error
		retryStatuses, err = buildRetryStatuses(cfg.Retry.Status)
		if err != nil {
			return nil, fmt.Errorf("retry.status: %w", err)
		}
	}
	dialTimeout := cfg.Timeout.Dial
	if dialTimeout == 0 {
		dialTimeout = proxyDialTimeout
	} else if dialTimeout < 0 {
		return nil, fmt.Errorf("timeout.dial must be greater than zero")
	}
	responseHeaderTimeout := cfg.Timeout.Header
	if responseHeaderTimeout == 0 {
		responseHeaderTimeout = proxyResponseHeaderTimeout
	} else if responseHeaderTimeout < 0 {
		return nil, fmt.Errorf("timeout.header must be greater than zero")
	}

	targets := make([]*url.URL, len(cfg.Targets))
	seen := make(map[string]struct{}, len(cfg.Targets))
	for index, raw := range cfg.Targets {
		if _, exists := seen[raw]; exists {
			return nil, fmt.Errorf("targets contains duplicate %q", raw)
		}
		seen[raw] = struct{}{}
		target, err := parseTarget(raw)
		if err != nil {
			return nil, fmt.Errorf("targets[%d]: %w", index, err)
		}
		targets[index] = target
	}
	var delay, deadline time.Duration
	if cfg.Retry != nil {
		delay = cfg.Retry.Delay
		deadline = cfg.Retry.Deadline
	}
	return &backend{
		targets:       targets,
		tries:         tries,
		retryStatuses: retryStatuses,
		retryDelay:    delay,
		retryDeadline: deadline,
		transport:     newProxyTransport(dialTimeout, responseHeaderTimeout),
	}, nil
}

func (b *backend) nextTargetIndex() int {
	if len(b.targets) == 1 {
		return 0
	}
	for {
		current := b.nextIndex.Load()
		next := current + 1
		if next == uint64(len(b.targets)) {
			next = 0
		}
		if b.nextIndex.CompareAndSwap(current, next) {
			return int(current)
		}
	}
}

func (b *backend) nextTarget() *url.URL {
	return b.targets[b.nextTargetIndex()]
}

func (b *backend) RoundTrip(request *http.Request) (*http.Response, error) {
	state, _ := request.Context().Value(backendRetryKey{}).(*backendRetryState)
	if state == nil {
		return b.transport.RoundTrip(request)
	}

	if b.retryStatuses == nil {
		response, lastErr := b.transport.RoundTrip(request)
		if lastErr == nil {
			return response, nil
		}
		if state.informational.Load() || request.Context().Err() != nil {
			return nil, lastErr
		}

		for attempt := 1; attempt < b.tries; attempt++ {
			state.informational.Store(false)
			outgoing := request.Clone(request.Context())
			requestURL := *request.URL
			target := b.targets[(state.start+attempt)%len(b.targets)]
			requestURL.Scheme = target.Scheme
			requestURL.Host = target.Host
			outgoing.URL = &requestURL

			response, err := b.transport.RoundTrip(outgoing)
			if err == nil {
				return response, nil
			}
			lastErr = err
			if state.informational.Load() || request.Context().Err() != nil {
				break
			}
		}
		return nil, lastErr
	}

	if b.retryDeadline == 0 {
		return b.roundTripWithStatusRetry(request, state)
	}
	ctx, cancel := context.WithTimeout(request.Context(), b.retryDeadline)
	response, err := b.roundTripWithStatusRetry(request.WithContext(ctx), state)
	if err != nil {
		cancel()
		if contextErr := ctx.Err(); contextErr != nil {
			return nil, contextErr
		}
		return nil, err
	}
	if response.Body == nil {
		cancel()
		return response, nil
	}
	response.Body = &deadlineBody{ReadCloser: response.Body, ctx: ctx, cancel: cancel}
	return response, nil
}

func (b *backend) roundTripWithStatusRetry(request *http.Request, state *backendRetryState) (*http.Response, error) {
	ctx := request.Context()
	for attempt := 0; attempt < b.tries; attempt++ {
		current := request
		if attempt > 0 {
			state.informational.Store(false)
			outgoing := request.Clone(request.Context())
			requestURL := *request.URL
			target := b.targets[(state.start+attempt)%len(b.targets)]
			requestURL.Scheme = target.Scheme
			requestURL.Host = target.Host
			outgoing.URL = &requestURL
			current = outgoing
		}

		response, err := b.transport.RoundTrip(current)
		if err != nil {
			if state.informational.Load() || ctx.Err() != nil || attempt+1 == b.tries {
				if contextErr := ctx.Err(); contextErr != nil && b.retryDeadline > 0 {
					return nil, contextErr
				}
				return nil, err
			}
			if b.retryDelay > 0 {
				if err := waitRetryDelay(ctx, b.retryDelay); err != nil {
					return nil, err
				}
			}
			continue
		}

		if attempt+1 == b.tries || state.informational.Load() {
			return response, nil
		}
		if _, retry := b.retryStatuses[response.StatusCode]; !retry {
			return response, nil
		}
		if response.Body != nil {
			_ = response.Body.Close()
		}
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		if b.retryDelay > 0 {
			if err := waitRetryDelay(ctx, b.retryDelay); err != nil {
				return nil, err
			}
		}
	}
	return nil, errors.New("backend retry attempts exhausted")
}

func waitRetryDelay(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-timer.C:
		return ctx.Err()
	case <-ctx.Done():
		return ctx.Err()
	}
}

func withBackendRetry(request *http.Request, start int) *http.Request {
	state := &backendRetryState{start: start}
	trace := &httptrace.ClientTrace{Got1xxResponse: func(int, textproto.MIMEHeader) error {
		state.informational.Store(true)
		return nil
	}}
	ctx := context.WithValue(request.Context(), backendRetryKey{}, state)
	return request.WithContext(httptrace.WithClientTrace(ctx, trace))
}

func retryableBackendRequest(request *http.Request) bool {
	switch request.Method {
	case http.MethodGet, http.MethodHead, http.MethodOptions, http.MethodTrace:
	default:
		return false
	}
	if request.ContentLength != 0 || len(request.TransferEncoding) != 0 || len(request.Trailer) != 0 ||
		request.Body != nil && request.Body != http.NoBody {
		return false
	}
	return !upgradeRequest(request)
}

func upgradeRequest(request *http.Request) bool {
	protocol := false
	for _, value := range request.Header.Values("Upgrade") {
		if strings.TrimSpace(value) != "" {
			protocol = true
			break
		}
	}
	if !protocol {
		return false
	}
	for _, value := range request.Header.Values("Connection") {
		for token := range strings.SplitSeq(value, ",") {
			if strings.EqualFold(strings.TrimSpace(token), "upgrade") {
				return true
			}
		}
	}
	return false
}

func newProxy(
	raw string,
	headers map[string]string,
	siteResponseHeaders, routeResponseHeaders responseHeaderPolicy,
	trustProxies []netip.Prefix,
) (*httputil.ReverseProxy, error) {
	target, err := parseTarget(raw)
	if err != nil {
		return nil, err
	}

	return newReverseProxy(func(request *httputil.ProxyRequest) {
		rewriteProxyRequest(request, target, headers, trustProxies)
	}, proxyTransport, siteResponseHeaders, routeResponseHeaders, 0), nil
}

func newBackendProxy(
	backend *backend,
	headers map[string]string,
	siteResponseHeaders, routeResponseHeaders responseHeaderPolicy,
	trustProxies []netip.Prefix,
) *httputil.ReverseProxy {
	rewrite := func(request *httputil.ProxyRequest) {
		start := backend.nextTargetIndex()
		rewriteProxyRequest(request, backend.targets[start], headers, trustProxies)
		if retryableBackendRequest(request.In) {
			request.Out = withBackendRetry(request.Out, start)
		}
	}
	transport := http.RoundTripper(backend)
	if backend.tries == 1 {
		rewrite = func(request *httputil.ProxyRequest) {
			rewriteProxyRequest(request, backend.nextTarget(), headers, trustProxies)
		}
		transport = backend.transport
	}
	return newReverseProxy(rewrite, transport, siteResponseHeaders, routeResponseHeaders, backend.retryDeadline)
}

func newReverseProxy(
	rewrite func(*httputil.ProxyRequest),
	transport http.RoundTripper,
	siteResponseHeaders, routeResponseHeaders responseHeaderPolicy,
	deadline time.Duration,
) *httputil.ReverseProxy {
	proxy := &httputil.ReverseProxy{
		Rewrite:      rewrite,
		Transport:    transport,
		BufferPool:   &proxyBuffers,
		ErrorHandler: proxyErrorHandler,
	}
	if deadline > 0 {
		proxy.ModifyResponse = func(response *http.Response) error {
			body, _ := response.Body.(*deadlineBody)
			if body != nil {
				if err := body.ctx.Err(); err != nil {
					return err
				}
			}
			applyResponseHeaderPolicies(response, siteResponseHeaders, routeResponseHeaders)
			if body != nil {
				return body.ctx.Err()
			}
			return nil
		}
	} else if !emptyResponseHeaderPolicy(siteResponseHeaders) || !emptyResponseHeaderPolicy(routeResponseHeaders) {
		proxy.ModifyResponse = func(response *http.Response) error {
			applyResponseHeaderPolicies(response, siteResponseHeaders, routeResponseHeaders)
			return nil
		}
	}
	return proxy
}

func emptyResponseHeaderPolicy(policy responseHeaderPolicy) bool {
	return len(policy.Set) == 0 && len(policy.Add) == 0 && len(policy.Remove) == 0
}

func applyResponseHeaders(response *http.Response, policy responseHeaderPolicy) {
	for _, name := range policy.Remove {
		if containsHeaderName(response.Trailer, name) {
			continue
		}
		delete(response.Header, name)
	}
	if response.Header == nil && (len(policy.Set) > 0 || len(policy.Add) > 0) {
		response.Header = make(http.Header, len(policy.Set)+len(policy.Add))
	}
	for name, values := range policy.Set {
		if containsHeaderName(response.Trailer, name) {
			continue
		}
		response.Header[name] = append([]string(nil), values...)
	}
	for name, values := range policy.Add {
		if containsHeaderName(response.Trailer, name) {
			continue
		}
		current := response.Header[name]
		combined := make([]string, 0, len(current)+len(values))
		combined = append(combined, current...)
		response.Header[name] = append(combined, values...)
	}
}

func applyResponseHeaderPolicies(response *http.Response, site, route responseHeaderPolicy) {
	// net/http.Transport and resolveResponseHeaders canonicalize these names.
	applyResponseHeaders(response, site)
	applyResponseHeaders(response, route)
}

func containsHeaderName(headers http.Header, name string) bool {
	if _, exists := headers[name]; exists {
		return true
	}
	for existing := range headers {
		if strings.EqualFold(existing, name) {
			return true
		}
	}
	return false
}

func rewriteProxyRequest(request *httputil.ProxyRequest, target *url.URL, headers map[string]string, trustProxies []netip.Prefix) {
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
