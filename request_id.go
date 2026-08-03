package main

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"strings"
)

const requestIDHeader = "X-Request-ID"

type requestIDContextKey struct{}

type requestIDWriter struct {
	http.ResponseWriter
	id                    string
	committed             bool
	hijacked              bool
	upgradeHeaderPrepared bool
}

func newRequestIDWriter(w http.ResponseWriter, id string) *requestIDWriter {
	writer := &requestIDWriter{ResponseWriter: w, id: id}
	writer.enforce()
	return writer
}

func (w *requestIDWriter) Header() http.Header {
	header := w.ResponseWriter.Header()
	if w.committed {
		if !w.hijacked || !w.upgradeHeaderPrepared {
			removeRequestID(header)
			removeRequestIDTrailer(header)
			if w.hijacked {
				w.upgradeHeaderPrepared = true
			}
		} else {
			removeRequestID(header)
			removeRequestIDTrailer(header)
			header.Set(requestIDHeader, w.id)
		}
	}
	return header
}

func (w *requestIDWriter) enforce() {
	header := w.ResponseWriter.Header()
	removeRequestID(header)
	removeRequestIDTrailer(header)
	header.Set(requestIDHeader, w.id)
}

func (w *requestIDWriter) WriteHeader(status int) {
	w.enforce()
	if status == http.StatusSwitchingProtocols || status >= http.StatusOK {
		w.committed = true
	}
	w.ResponseWriter.WriteHeader(status)
}

func (w *requestIDWriter) Write(data []byte) (int, error) {
	w.enforce()
	w.committed = true
	return w.ResponseWriter.Write(data)
}

func (w *requestIDWriter) FlushError() error {
	w.enforce()
	err := http.NewResponseController(w.ResponseWriter).Flush()
	if err == nil {
		w.committed = true
	}
	return err
}

func (w *requestIDWriter) Flush() {
	_ = w.FlushError()
}

func (w *requestIDWriter) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	w.enforce()
	connection, buffered, err := http.NewResponseController(w.ResponseWriter).Hijack()
	if err == nil {
		w.committed = true
		w.hijacked = true
		w.upgradeHeaderPrepared = false
	}
	return connection, buffered, err
}

func (w *requestIDWriter) Unwrap() http.ResponseWriter {
	return w.ResponseWriter
}

func requestIDFor(req *http.Request) (string, error) {
	var value string
	count := 0
	for name, values := range req.Header {
		if !strings.EqualFold(name, requestIDHeader) {
			continue
		}
		count += len(values)
		if len(values) == 1 {
			value = values[0]
		}
	}
	if count == 1 && len(value) >= 1 && len(value) <= 128 && validMethod(value) {
		return value, nil
	}

	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", fmt.Errorf("generate request ID: %w", err)
	}
	return hex.EncodeToString(random), nil
}

func setRequestID(req *http.Request, id string) *http.Request {
	if req.Header == nil {
		req.Header = make(http.Header)
	}
	removeRequestID(req.Header)
	req.Header.Set(requestIDHeader, id)
	return req.WithContext(context.WithValue(req.Context(), requestIDContextKey{}, id))
}

func requestIDFromRequest(req *http.Request) string {
	id, _ := req.Context().Value(requestIDContextKey{}).(string)
	return id
}

func removeRequestID(header http.Header) {
	for name := range header {
		if strings.EqualFold(name, requestIDHeader) ||
			(strings.HasPrefix(name, http.TrailerPrefix) && strings.EqualFold(strings.TrimPrefix(name, http.TrailerPrefix), requestIDHeader)) {
			delete(header, name)
		}
	}
}

func removeRequestIDTrailer(header http.Header) {
	for name, values := range header {
		if !strings.EqualFold(name, "Trailer") {
			continue
		}
		kept := make([]string, 0, len(values))
		for _, value := range values {
			tokens := strings.Split(value, ",")
			filtered := tokens[:0]
			for _, token := range tokens {
				if !strings.EqualFold(strings.TrimSpace(token), requestIDHeader) {
					filtered = append(filtered, strings.TrimSpace(token))
				}
			}
			if len(filtered) > 0 {
				kept = append(kept, strings.Join(filtered, ", "))
			}
		}
		if len(kept) == 0 {
			delete(header, name)
		} else {
			header[name] = kept
		}
	}
}

func enableRequestIDProxy(proxy *httputil.ReverseProxy) {
	previousRewrite := proxy.Rewrite
	proxy.Rewrite = func(request *httputil.ProxyRequest) {
		previousRewrite(request)
		if id := requestIDFromRequest(request.In); id != "" {
			request.Out.Header.Set(requestIDHeader, id)
		}
	}
	previous := proxy.ModifyResponse
	proxy.ModifyResponse = func(response *http.Response) error {
		if previous != nil {
			if err := previous(response); err != nil {
				return err
			}
		}
		removeRequestID(response.Header)
		removeRequestIDTrailer(response.Header)
		removeRequestID(response.Trailer)
		if response.Request != nil {
			if id := requestIDFromRequest(response.Request); id != "" {
				response.Header.Set(requestIDHeader, id)
			}
		}
		if response.Body != nil {
			if body, ok := response.Body.(io.ReadWriteCloser); ok {
				response.Body = &requestIDReadWriteBody{ReadWriteCloser: body, response: response}
			} else {
				response.Body = &requestIDBody{ReadCloser: response.Body, response: response}
			}
		}
		return nil
	}
}

type requestIDBody struct {
	io.ReadCloser
	response *http.Response
}

func (b *requestIDBody) Close() error {
	err := b.ReadCloser.Close()
	removeRequestID(b.response.Trailer)
	return err
}

type requestIDReadWriteBody struct {
	io.ReadWriteCloser
	response *http.Response
}

func (b *requestIDReadWriteBody) Close() error {
	err := b.ReadWriteCloser.Close()
	removeRequestID(b.response.Trailer)
	return err
}
