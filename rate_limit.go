package main

import (
	"sync/atomic"
	"time"
)

const nanotokensPerToken int64 = 1_000_000_000

const maxInt64 = int64(^uint64(0) >> 1)

type rateLimiter struct {
	state    atomic.Pointer[rateLimitState]
	fallback *limitConfig
	current  *atomic.Pointer[runtimeGeneration]
}

type rateLimitState struct {
	config   *limitConfig
	rate     int64
	capacity int64
	tokens   int64
	last     time.Time
}

func newRateLimiter(config *limitConfig) *rateLimiter {
	return &rateLimiter{fallback: config}
}

func newRateLimiterAt(config *limitConfig, now time.Time) *rateLimiter {
	limiter := &rateLimiter{fallback: config}
	if config != nil {
		limiter.state.Store(newRateLimitState(config, now))
	}
	return limiter
}

func newRateLimitState(config *limitConfig, now time.Time) *rateLimitState {
	state := newRateLimitStateValue(config, now)
	if config == nil {
		return nil
	}
	return &state
}

func newRateLimitStateValue(config *limitConfig, now time.Time) rateLimitState {
	if config == nil {
		return rateLimitState{}
	}
	capacity := int64(config.Burst) * nanotokensPerToken
	return rateLimitState{
		config:   config,
		rate:     int64(config.Rate),
		capacity: capacity,
		tokens:   capacity,
		last:     now,
	}
}

func (l *rateLimiter) bindCurrent(current *atomic.Pointer[runtimeGeneration]) {
	l.current = current
}

func (l *rateLimiter) allow() bool {
	return l.admit()
}

func (l *rateLimiter) admit() bool {
	if l.current == nil {
		if l.fallback == nil {
			return true
		}
		return l.reserve(l.fallback, time.Now())
	}
	_, allowed := l.admitGeneration(l.current.Load())
	return allowed
}

func (l *rateLimiter) admitGeneration(generation *runtimeGeneration) (*runtimeGeneration, bool) {
	if l.current == nil {
		if generation == nil || generation.router == nil || generation.router.limit == nil {
			return generation, true
		}
		return generation, l.reserve(generation.router.limit, time.Now())
	}
	for {
		if generation == nil {
			generation = l.current.Load()
		}
		if generation == nil || generation.router == nil || generation.router.limit == nil {
			return generation, true
		}
		bucket, allowed := reserveBucket(generation.bucket, generation.router.limit, time.Now())
		next := &runtimeGeneration{
			router:      generation.router,
			certificate: generation.certificate,
			bucket:      bucket,
		}
		if l.current.CompareAndSwap(generation, next) {
			return next, allowed
		}
		generation = l.current.Load()
	}
}

func (l *rateLimiter) allowAt(now time.Time) bool {
	config := l.fallback
	if l.current != nil {
		_, allowed := l.admitGeneration(l.current.Load())
		return allowed
	}
	if config == nil {
		return true
	}
	return l.reserve(config, now)
}

func (l *rateLimiter) reserve(config *limitConfig, now time.Time) bool {
	for {
		previous := l.state.Load()
		var previousValue rateLimitState
		if previous != nil {
			previousValue = *previous
		}
		next, allowed := reserveBucket(previousValue, config, now)
		if l.state.CompareAndSwap(previous, &next) {
			return allowed
		}
	}
}

func reserveBucket(previous rateLimitState, config *limitConfig, now time.Time) (rateLimitState, bool) {
	if previous.config != config {
		next := newRateLimitStateValue(config, now)
		next.tokens -= nanotokensPerToken
		return next, true
	}

	next := previous
	if !now.Before(previous.last) {
		elapsed := now.Sub(previous.last).Nanoseconds()
		next.tokens = saturatingAdd(previous.tokens, saturatingMultiply(elapsed, previous.rate))
		if next.tokens > previous.capacity {
			next.tokens = previous.capacity
		}
		next.last = now
	}

	allowed := next.tokens >= nanotokensPerToken
	if allowed {
		next.tokens -= nanotokensPerToken
	}
	return next, allowed
}

func saturatingAdd(left, right int64) int64 {
	if right > maxInt64-left {
		return maxInt64
	}
	return left + right
}

func saturatingMultiply(left, right int64) int64 {
	if left <= 0 || right <= 0 {
		return 0
	}
	if left > maxInt64/right {
		return maxInt64
	}
	return left * right
}
