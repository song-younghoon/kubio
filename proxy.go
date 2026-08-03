package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"math/rand/v2"
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
	targets         []*url.URL
	schedule        []int
	tries           int
	retryStatuses   map[int]struct{}
	retryMethods    map[string]struct{}
	retryBodyMax    int64
	backoffDelays   []time.Duration
	backoffJitter   bool
	retryDeadline   time.Duration
	budget          *retryBudget
	health          *backendHealth
	transport       *http.Transport
	nextIndex       atomic.Uint64
	probeTargets    []string
	probeCancel     context.CancelFunc
	probeDone       chan struct{}
	probeCancelOnce sync.Once
}

type backendGeneration struct {
	retired atomic.Bool
}

type backendRetryKey struct{}

type backendRetryState struct {
	start         int
	retry         bool
	informational atomic.Bool
}

type backendHealth struct {
	fail       int
	cool       time.Duration
	probe      *backendHealthProbeConfig
	generation *backendGeneration
	targets    []targetHealth
}

type targetHealth struct {
	mu       sync.Mutex
	failures int
	until    time.Time
}

func (h *backendHealth) available(index int) bool {
	target := &h.targets[index]
	target.mu.Lock()
	defer target.mu.Unlock()
	now := time.Now()
	if target.until.IsZero() {
		return true
	}
	if now.Before(target.until) {
		return false
	}
	target.until = time.Time{}
	target.failures = 0
	return true
}

func (h *backendHealth) observe(index int, ctx context.Context, response bool, err error) {
	target := &h.targets[index]
	target.mu.Lock()
	defer target.mu.Unlock()
	if response || err == nil {
		resetTargetHealth(target)
		return
	}
	if ctx.Err() != nil {
		return
	}
	h.observeFailureLocked(target)
}

func (h *backendHealth) observeProbe(index int, generation, probeContext context.Context, responseHealthy bool) {
	target := &h.targets[index]
	target.mu.Lock()
	defer target.mu.Unlock()
	if generation.Err() != nil || h.generation != nil && h.generation.retired.Load() {
		return
	}
	if responseHealthy && probeContext.Err() == nil {
		resetTargetHealth(target)
		return
	}
	h.observeFailureLocked(target)
}

func (h *backendHealth) observeFailureLocked(target *targetHealth) {
	now := time.Now()
	if !target.until.IsZero() {
		if now.Before(target.until) {
			target.until = target.until.Add(h.cool)
			return
		}
		target.until = time.Time{}
		target.failures = 0
	}
	target.failures++
	if target.failures >= h.fail {
		target.failures = 0
		target.until = now.Add(h.cool)
	}
}

func resetTargetHealth(target *targetHealth) {
	target.failures = 0
	target.until = time.Time{}
}

func (b *backend) startProbe() {
	if b.health == nil || b.health.probe == nil {
		return
	}
	ctx, cancel := context.WithCancel(context.Background())
	b.probeCancel = cancel
	b.probeDone = make(chan struct{})
	go func() {
		defer close(b.probeDone)
		b.runProbeLoop(ctx)
	}()
}

func (b *backend) cancelProbe() {
	b.probeCancelOnce.Do(func() {
		if b.probeCancel != nil {
			b.probeCancel()
		}
	})
}

func (b *backend) joinProbe() {
	if b.probeDone != nil {
		<-b.probeDone
	}
}

func (b *backend) runProbeLoop(ctx context.Context) {
	probe := b.health.probe
	timer := time.NewTimer(0)
	defer timer.Stop()
	if !waitProbeTimer(ctx, timer, probe.Every) {
		return
	}
	for {
		started := time.Now()
		for index := range b.targets {
			if ctx.Err() != nil {
				return
			}
			b.probeTarget(ctx, index)
		}
		next := started.Add(probe.Every)
		if finished := time.Now(); finished.After(next) {
			next = finished
		}
		if !waitProbeTimer(ctx, timer, time.Until(next)) {
			return
		}
	}
}

func waitProbeTimer(ctx context.Context, timer *time.Timer, delay time.Duration) bool {
	if !timer.Stop() {
		select {
		case <-timer.C:
		default:
		}
	}
	if delay < 0 {
		delay = 0
	}
	timer.Reset(delay)
	select {
	case <-timer.C:
		return true
	case <-ctx.Done():
		return false
	}
}

func (b *backend) probeTarget(generation context.Context, index int) {
	probe := b.health.probe
	probeContext, cancel := context.WithTimeout(generation, probe.Timeout)
	defer cancel()
	request, err := http.NewRequestWithContext(probeContext, http.MethodGet, b.probeTargets[index], nil)
	if err != nil {
		b.health.observeProbe(index, generation, probeContext, false)
		return
	}
	request.Host = b.targets[index].Host
	response, err := b.transport.RoundTrip(request)
	if response != nil && response.Body != nil {
		_ = response.Body.Close()
	}
	responseHealthy := err == nil && response != nil && response.Body != nil &&
		response.StatusCode >= http.StatusOK && response.StatusCode <= 399
	b.health.observeProbe(index, generation, probeContext, responseHealthy)
}

var errRetryBudgetExceeded = errors.New("retry budget exhausted")

type retryBudget struct {
	mu      sync.Mutex
	max     int
	window  time.Duration
	started time.Time
	used    int
}

func (b *retryBudget) reserve(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	now := time.Now()
	if b.started.IsZero() || now.Sub(b.started) >= b.window {
		b.started = now
		b.used = 0
	}
	if b.used >= b.max {
		return errRetryBudgetExceeded
	}
	b.used++
	return nil
}

func (b *backend) reserveRetry(ctx context.Context) (bool, error) {
	if b.budget == nil {
		return true, nil
	}
	err := b.budget.reserve(ctx)
	if err == errRetryBudgetExceeded {
		return false, nil
	}
	return err == nil, err
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

func validateTargetWeights(weights []int, targetCount int) error {
	if len(weights) == 0 {
		return fmt.Errorf("must not be empty")
	}
	if len(weights) != targetCount {
		return fmt.Errorf("must contain one item per target")
	}
	total := 0
	for index, weight := range weights {
		if weight < 1 || weight > maxTargetWeight {
			return fmt.Errorf("item %d must be between 1 and %d", index, maxTargetWeight)
		}
		if total > maxTargetWeightTotal-weight {
			return fmt.Errorf("sum must not exceed %d", maxTargetWeightTotal)
		}
		total += weight
	}
	return nil
}

func buildTargetSchedule(weights []int) []int {
	total := 0
	for _, weight := range weights {
		total += weight
	}
	current := make([]int, len(weights))
	schedule := make([]int, 0, total)
	for range total {
		selected := 0
		for index, weight := range weights {
			current[index] += weight
			if index == 0 || current[index] > current[selected] {
				selected = index
			}
		}
		current[selected] -= total
		schedule = append(schedule, selected)
	}
	return schedule
}

func validateBackoff(backoff *backendBackoffConfig) error {
	if backoff == nil {
		return nil
	}
	if backoff.Base <= 0 {
		return fmt.Errorf("base must be greater than zero")
	}
	if backoff.Cap < backoff.Base {
		return fmt.Errorf("cap must be greater than or equal to base")
	}
	return nil
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
	var retryMethods map[string]struct{}
	var retryBodyMax int64
	var budget *retryBudget
	if cfg.Retry != nil {
		if tries <= 1 {
			return nil, fmt.Errorf("retry requires tries greater than one")
		}
		if cfg.Retry.Deadline < 0 {
			return nil, fmt.Errorf("retry.deadline must be greater than zero")
		}
		if err := validateBackoff(cfg.Retry.Backoff); err != nil {
			return nil, fmt.Errorf("retry.backoff: %w", err)
		}
		if len(cfg.Retry.Methods) > 0 {
			if err := validateRetryMethods(cfg.Retry.Methods); err != nil {
				return nil, fmt.Errorf("retry.methods: %w", err)
			}
			retryMethods = make(map[string]struct{}, len(cfg.Retry.Methods))
			for _, method := range cfg.Retry.Methods {
				retryMethods[method] = struct{}{}
			}
		}
		if cfg.Retry.Body != nil {
			if cfg.Retry.Body.Max < 1 || cfg.Retry.Body.Max > maxRetryBodyBytes {
				return nil, fmt.Errorf("retry.body.max must be between 1 and %d", maxRetryBodyBytes)
			}
			retryBodyMax = cfg.Retry.Body.Max
		}
		if cfg.Retry.Budget != nil {
			if cfg.Retry.Budget.Max < 1 || cfg.Retry.Budget.Max > maxRetryBudget {
				return nil, fmt.Errorf("retry.budget.max must be between 1 and %d", maxRetryBudget)
			}
			if cfg.Retry.Budget.Window <= 0 {
				return nil, fmt.Errorf("retry.budget.window must be greater than zero")
			}
			budget = &retryBudget{max: cfg.Retry.Budget.Max, window: cfg.Retry.Budget.Window}
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
	var schedule []int
	if cfg.Weights != nil {
		if err := validateTargetWeights(cfg.Weights, len(targets)); err != nil {
			return nil, fmt.Errorf("weights: %w", err)
		}
		schedule = buildTargetSchedule(cfg.Weights)
	}
	var health *backendHealth
	if cfg.Health != nil {
		if cfg.Health.Fail < 1 || cfg.Health.Fail > maxHealthFailures {
			return nil, fmt.Errorf("health.fail must be between 1 and %d", maxHealthFailures)
		}
		if cfg.Health.Cool <= 0 || cfg.Health.Cool > maxHealthCooldown {
			return nil, fmt.Errorf("health.cool must be greater than zero and no greater than %s", maxHealthCooldown)
		}
		health = &backendHealth{
			fail:    cfg.Health.Fail,
			cool:    cfg.Health.Cool,
			probe:   cfg.Health.Probe,
			targets: make([]targetHealth, len(targets)),
		}
	}
	var backoffDelays []time.Duration
	var backoffJitter bool
	var deadline time.Duration
	if cfg.Retry != nil {
		if cfg.Retry.Backoff != nil {
			backoffDelays = buildBackoffDelays(cfg.Retry.Backoff, tries)
			backoffJitter = cfg.Retry.Backoff.Jitter
		}
		deadline = cfg.Retry.Deadline
	}
	backend := &backend{
		targets:       targets,
		schedule:      schedule,
		tries:         tries,
		retryStatuses: retryStatuses,
		retryMethods:  retryMethods,
		retryBodyMax:  retryBodyMax,
		backoffDelays: backoffDelays,
		backoffJitter: backoffJitter,
		retryDeadline: deadline,
		budget:        budget,
		health:        health,
		transport:     newProxyTransport(dialTimeout, responseHeaderTimeout),
	}
	if health != nil && health.probe != nil {
		backend.probeTargets = make([]string, len(targets))
		for index, target := range targets {
			probeTarget := *target
			probeTarget.Path = health.probe.Path.Path
			probeTarget.RawPath = health.probe.Path.RawPath
			probeTarget.RawQuery = health.probe.Path.RawQuery
			probeTarget.ForceQuery = health.probe.Path.ForceQuery
			backend.probeTargets[index] = probeTarget.String()
		}
	}
	return backend, nil
}

func (b *backend) nextScheduledTargetIndex() int {
	count := len(b.targets)
	if len(b.schedule) > 0 {
		count = len(b.schedule)
	}
	if count == 1 {
		if len(b.schedule) > 0 {
			return b.schedule[0]
		}
		return 0
	}
	current := b.nextIndex.Add(1) - 1
	index := int(current % uint64(count))
	if len(b.schedule) > 0 {
		return b.schedule[index]
	}
	return index
}

func (b *backend) nextTargetIndex() int {
	first := b.nextScheduledTargetIndex()
	if b.health == nil {
		return first
	}
	return b.nextHealthyTargetIndex(first)
}

func (b *backend) nextHealthyTargetIndex(first int) int {
	count := len(b.targets)
	if len(b.schedule) > 0 {
		count = len(b.schedule)
	}
	index := first
	if b.health.available(index) {
		return index
	}
	for attempts := 1; attempts < count; attempts++ {
		index = b.nextScheduledTargetIndex()
		if b.health.available(index) {
			return index
		}
	}
	return first
}

func (b *backend) retryTargetIndex(start, attempt int) int {
	index := (start + attempt) % len(b.targets)
	if b.health == nil {
		return index
	}
	first := index
	for range len(b.targets) {
		if b.health.available(index) {
			return index
		}
		index++
		if index == len(b.targets) {
			index = 0
		}
	}
	return first
}

func (b *backend) observeAttempt(index int, ctx context.Context, informational bool, response *http.Response, err error) {
	if b.health != nil {
		b.health.observe(index, ctx, informational || response != nil, err)
	}
}

func (b *backend) nextScheduledTarget() *url.URL {
	return b.targets[b.nextScheduledTargetIndex()]
}

func (b *backend) RoundTrip(request *http.Request) (*http.Response, error) {
	state, _ := request.Context().Value(backendRetryKey{}).(*backendRetryState)
	if state == nil {
		return b.transport.RoundTrip(request)
	}
	if !state.retry {
		response, err := b.transport.RoundTrip(request)
		b.observeAttempt(state.start, request.Context(), state.informational.Load(), response, err)
		return response, err
	}

	if b.retryStatuses == nil {
		targetIndex := state.start
		response, lastErr := b.transport.RoundTrip(request)
		b.observeAttempt(targetIndex, request.Context(), state.informational.Load(), response, lastErr)
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
			targetIndex = b.retryTargetIndex(state.start, attempt)
			target := b.targets[targetIndex]
			requestURL.Scheme = target.Scheme
			requestURL.Host = target.Host
			outgoing.URL = &requestURL

			response, err := b.transport.RoundTrip(outgoing)
			b.observeAttempt(targetIndex, request.Context(), state.informational.Load(), response, err)
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
	replayBody := b.retryBodyMax > 0 && request.Body != nil && request.Body != http.NoBody
	if replayBody {
		if err := prepareRetryBody(request, b.retryBodyMax); err != nil {
			return nil, err
		}
		defer releaseRetryBody(request)
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
	targetIndex := state.start
	for attempt := 0; attempt < b.tries; attempt++ {
		current := request
		if attempt > 0 {
			state.informational.Store(false)
			outgoing, err := cloneRetryRequest(request)
			if err != nil {
				return nil, err
			}
			requestURL := *request.URL
			targetIndex = b.retryTargetIndex(state.start, attempt)
			target := b.targets[targetIndex]
			requestURL.Scheme = target.Scheme
			requestURL.Host = target.Host
			outgoing.URL = &requestURL
			current = outgoing
		}

		response, err := b.transport.RoundTrip(current)
		b.observeAttempt(targetIndex, ctx, state.informational.Load(), response, err)
		if err != nil {
			if state.informational.Load() || ctx.Err() != nil || attempt+1 == b.tries {
				if contextErr := ctx.Err(); contextErr != nil && (b.retryDeadline > 0 || b.budget != nil) {
					return nil, contextErr
				}
				return nil, err
			}
			admitted, admissionErr := b.reserveRetry(ctx)
			if admissionErr != nil {
				return nil, admissionErr
			}
			if !admitted {
				return nil, err
			}
			if b.backoffDelays != nil {
				if err := waitRetryBackoff(ctx, b.backoffDelays, b.backoffJitter, attempt+1); err != nil {
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
		if err := ctx.Err(); err != nil {
			if response.Body != nil {
				_ = response.Body.Close()
			}
			return nil, err
		}
		admitted, admissionErr := b.reserveRetry(ctx)
		if admissionErr != nil {
			if response.Body != nil {
				_ = response.Body.Close()
			}
			return nil, admissionErr
		}
		if !admitted {
			return response, nil
		}
		if response.Body != nil {
			_ = response.Body.Close()
		}
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		if b.backoffDelays != nil {
			if err := waitRetryBackoff(ctx, b.backoffDelays, b.backoffJitter, attempt+1); err != nil {
				return nil, err
			}
		}
	}
	return nil, errors.New("backend retry attempts exhausted")
}

func buildBackoffDelays(backoff *backendBackoffConfig, tries int) []time.Duration {
	delays := make([]time.Duration, tries-1)
	delay := backoff.Base
	for index := range delays {
		delays[index] = delay
		if delay >= backoff.Cap {
			continue
		}
		if delay > backoff.Cap/2 {
			delay = backoff.Cap
			continue
		}
		delay *= 2
	}
	return delays
}

func waitRetryBackoff(ctx context.Context, delays []time.Duration, jitter bool, attempt int) error {
	delay := delays[attempt-1]
	if jitter {
		delay = time.Duration(rand.Int64N(int64(delay)))
	}
	if delay == 0 {
		return ctx.Err()
	}
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-timer.C:
		return ctx.Err()
	case <-ctx.Done():
		return ctx.Err()
	}
}

func withBackendState(request *http.Request, start int, retry bool) *http.Request {
	state := &backendRetryState{start: start, retry: retry}
	trace := &httptrace.ClientTrace{Got1xxResponse: func(int, textproto.MIMEHeader) error {
		state.informational.Store(true)
		return nil
	}}
	ctx := context.WithValue(request.Context(), backendRetryKey{}, state)
	return request.WithContext(httptrace.WithClientTrace(ctx, trace))
}

func withBackendRetry(request *http.Request, start int) *http.Request {
	return withBackendState(request, start, true)
}

func retryableBackendRequest(request *http.Request, methods map[string]struct{}, bodyMax int64) bool {
	if methods == nil && bodyMax == 0 {
		return retryableBodylessRequest(request)
	}
	if request.Method == http.MethodConnect {
		return false
	}
	if methods == nil {
		switch request.Method {
		case http.MethodGet, http.MethodHead, http.MethodOptions, http.MethodTrace:
		default:
			return false
		}
	} else if _, ok := methods[request.Method]; !ok {
		return false
	}
	if upgradeRequest(request) {
		return false
	}
	body := request.Body != nil && request.Body != http.NoBody
	if !body {
		return request.ContentLength == 0 && len(request.TransferEncoding) == 0 && len(request.Trailer) == 0
	}
	return bodyMax > 0 && len(request.Trailer) == 0
}

func retryableBodylessRequest(request *http.Request) bool {
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

var errRetryBodyTooLarge = errors.New("retry request body exceeds configured maximum")

func prepareRetryBody(request *http.Request, max int64) error {
	original := request.Body
	defer func() { _ = original.Close() }()
	fail := func(err error) error {
		request.Body = http.NoBody
		request.GetBody = nil
		return err
	}
	if err := request.Context().Err(); err != nil {
		return fail(err)
	}
	if request.ContentLength > max {
		if err := request.Context().Err(); err != nil {
			return fail(err)
		}
		return fail(errRetryBodyTooLarge)
	}
	capacity := int64(proxyBufferSize)
	if max < capacity {
		capacity = max
	}
	if request.ContentLength > 0 && request.ContentLength < capacity {
		capacity = request.ContentLength
	}
	payload := make([]byte, 0, int(capacity))
	chunk := make([]byte, proxyBufferSize)
	emptyReads := 0
	for {
		if err := request.Context().Err(); err != nil {
			return fail(err)
		}
		remaining := max - int64(len(payload))
		readSize := len(chunk)
		if remaining+1 < int64(readSize) {
			readSize = int(remaining + 1)
		}
		n, err := original.Read(chunk[:readSize])
		if contextErr := request.Context().Err(); contextErr != nil {
			return fail(contextErr)
		}
		if int64(n) > remaining {
			return fail(errRetryBodyTooLarge)
		}
		if n > 0 {
			payload = append(payload, chunk[:n]...)
		}
		if n == 0 && err == nil {
			emptyReads++
			if emptyReads == 100 {
				return fail(io.ErrNoProgress)
			}
		} else {
			emptyReads = 0
		}
		if err != nil {
			if errors.Is(err, io.EOF) {
				break
			}
			return fail(err)
		}
	}
	request.Body = io.NopCloser(bytes.NewReader(payload))
	request.GetBody = func() (io.ReadCloser, error) {
		return io.NopCloser(bytes.NewReader(payload)), nil
	}
	request.ContentLength = int64(len(payload))
	request.TransferEncoding = nil
	request.Trailer = nil
	return nil
}

func releaseRetryBody(request *http.Request) {
	if request.Body != nil && request.Body != http.NoBody {
		_ = request.Body.Close()
	}
	request.Body = http.NoBody
	request.GetBody = nil
}

func cloneRetryRequest(request *http.Request) (*http.Request, error) {
	outgoing := request.Clone(request.Context())
	if request.GetBody == nil {
		return outgoing, nil
	}
	body, err := request.GetBody()
	if err != nil {
		return nil, err
	}
	outgoing.Body = body
	return outgoing, nil
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
		start := backend.nextScheduledTargetIndex()
		if backend.health != nil {
			start = backend.nextHealthyTargetIndex(start)
		}
		rewriteProxyRequest(request, backend.targets[start], headers, trustProxies)
		retry := retryableBackendRequest(request.In, backend.retryMethods, backend.retryBodyMax)
		if retry || backend.health != nil {
			if retry && backend.retryBodyMax > 0 && request.In.Body != nil && request.In.Body != http.NoBody {
				request.Out.Body = request.In.Body
			}
			request.Out = withBackendState(request.Out, start, retry)
		}
	}
	transport := http.RoundTripper(backend)
	if backend.tries == 1 && backend.health == nil {
		rewrite = func(request *httputil.ProxyRequest) {
			rewriteProxyRequest(request, backend.nextScheduledTarget(), headers, trustProxies)
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
	if errors.Is(err, errRetryBodyTooLarge) {
		status = http.StatusRequestEntityTooLarge
	}
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
