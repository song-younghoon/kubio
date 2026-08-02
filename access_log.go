package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"time"
)

var stdoutAccessLogger = log.New(os.Stdout, "", 0)

type accessRecord struct {
	Time       string `json:"time"`
	Method     string `json:"method"`
	Host       string `json:"host"`
	Path       string `json:"path"`
	Proto      string `json:"proto"`
	Peer       string `json:"peer"`
	Status     int    `json:"status"`
	Bytes      int64  `json:"bytes"`
	DurationUs int64  `json:"durationUs"`
}

type accessResponseWriter struct {
	http.ResponseWriter
	status int
	bytes  int64
}

func (r *router) serveLogged(w http.ResponseWriter, req *http.Request) {
	method, host, proto := req.Method, req.Host, req.Proto
	path := req.URL.EscapedPath()
	if path == "" {
		path = "/"
	}
	_, peer := peerAddress(req.RemoteAddr)
	start := time.Now()
	observed := &accessResponseWriter{ResponseWriter: w}
	defer func() {
		panicValue := recover()
		if observed.status == 0 {
			if panicValue != nil {
				observed.status = http.StatusInternalServerError
			} else {
				observed.status = http.StatusOK
			}
		}
		end := time.Now()
		record := accessRecord{
			Time:       end.UTC().Format(time.RFC3339Nano),
			Method:     method,
			Host:       host,
			Path:       path,
			Proto:      proto,
			Peer:       peer,
			Status:     observed.status,
			Bytes:      observed.bytes,
			DurationUs: end.Sub(start).Microseconds(),
		}
		if data, err := json.Marshal(record); err == nil {
			_ = r.accessLogger.Output(2, string(data))
		}
		if panicValue != nil {
			panic(panicValue)
		}
	}()
	r.serveHTTP(observed, req)
}

func (w *accessResponseWriter) WriteHeader(status int) {
	if w.status == 0 && (status == http.StatusSwitchingProtocols || status >= 200) {
		w.status = status
	}
	w.ResponseWriter.WriteHeader(status)
}

func (w *accessResponseWriter) Write(data []byte) (int, error) {
	if w.status == 0 {
		w.status = http.StatusOK
	}
	written, err := w.ResponseWriter.Write(data)
	w.bytes += int64(written)
	return written, err
}

func (w *accessResponseWriter) FlushError() error {
	err := http.NewResponseController(w.ResponseWriter).Flush()
	if w.status == 0 && !errors.Is(err, http.ErrNotSupported) {
		w.status = http.StatusOK
	}
	return err
}

func (w *accessResponseWriter) Flush() {
	_ = w.FlushError()
}

func (w *accessResponseWriter) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	connection, buffered, err := http.NewResponseController(w.ResponseWriter).Hijack()
	if err == nil && w.status == 0 {
		w.status = http.StatusSwitchingProtocols
	}
	return connection, buffered, err
}

func (w *accessResponseWriter) Unwrap() http.ResponseWriter {
	return w.ResponseWriter
}
