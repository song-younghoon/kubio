package main

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httputil"
	"net/netip"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const (
	proxyDialTimeout         = 5 * time.Second
	proxyResponseHeaderLimit = 30 * time.Second
	proxyBufferSize          = 32 * 1024
	proxyMaxIdleConnsPerHost = 32
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

type backend struct {
	targets   []*url.URL
	nextIndex atomic.Uint64
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

func newBackend(rawTargets []string) (*backend, error) {
	if len(rawTargets) == 0 {
		return nil, fmt.Errorf("targets must not be empty")
	}
	targets := make([]*url.URL, len(rawTargets))
	seen := make(map[string]struct{}, len(rawTargets))
	for index, raw := range rawTargets {
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
	return &backend{targets: targets}, nil
}

func (b *backend) nextTarget() *url.URL {
	if len(b.targets) == 1 {
		return b.targets[0]
	}
	for {
		current := b.nextIndex.Load()
		next := current + 1
		if next == uint64(len(b.targets)) {
			next = 0
		}
		if b.nextIndex.CompareAndSwap(current, next) {
			return b.targets[current]
		}
	}
}

func newProxy(raw string, headers, responseHeaders map[string]string, trustProxies []netip.Prefix) (*httputil.ReverseProxy, error) {
	target, err := parseTarget(raw)
	if err != nil {
		return nil, err
	}

	return newReverseProxy(func(request *httputil.ProxyRequest) {
		rewriteProxyRequest(request, target, headers, trustProxies)
	}, responseHeaders), nil
}

func newBackendProxy(backend *backend, headers, responseHeaders map[string]string, trustProxies []netip.Prefix) *httputil.ReverseProxy {
	return newReverseProxy(func(request *httputil.ProxyRequest) {
		rewriteProxyRequest(request, backend.nextTarget(), headers, trustProxies)
	}, responseHeaders)
}

func newReverseProxy(rewrite func(*httputil.ProxyRequest), responseHeaders map[string]string) *httputil.ReverseProxy {
	proxy := &httputil.ReverseProxy{
		Rewrite:      rewrite,
		Transport:    proxyTransport,
		BufferPool:   &proxyBuffers,
		ErrorHandler: proxyErrorHandler,
	}
	if len(responseHeaders) > 0 {
		proxy.ModifyResponse = func(response *http.Response) error {
			replaceResponseHeaders(response, responseHeaders)
			return nil
		}
	}
	return proxy
}

func replaceResponseHeaders(response *http.Response, configured map[string]string) {
	if response.Header == nil {
		response.Header = make(http.Header, len(configured))
	}
	for name, value := range configured {
		if containsHeaderName(response.Trailer, name) {
			continue
		}
		for existing := range response.Header {
			if strings.EqualFold(existing, name) {
				delete(response.Header, existing)
			}
		}
		response.Header[name] = []string{value}
	}
}

func containsHeaderName(headers http.Header, name string) bool {
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
