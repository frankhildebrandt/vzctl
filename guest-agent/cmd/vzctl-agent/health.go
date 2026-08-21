package main

import (
	"os"
	"sort"
	"sync"
	"time"
)

const (
	healthExecSampleSize     = 32
	healthDegradedQueueDepth = 2
	healthDegradedP99MS      = 5000
)

type healthTracker struct {
	mu         sync.Mutex
	inFlight   int
	latencies  []time.Duration
	lastError  string
	tokenReady func() bool
}

func newHealthTracker() *healthTracker {
	return &healthTracker{
		tokenReady: func() bool {
			info, err := os.Stat(defaultToken)
			if err != nil {
				return os.IsNotExist(err)
			}
			return info.Mode().Perm()&0o077 == 0
		},
	}
}

func (t *healthTracker) beginExec() {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.inFlight++
	t.mu.Unlock()
}

func (t *healthTracker) endExec(started time.Time, errMessage string) {
	if t == nil {
		return
	}
	elapsed := time.Since(started)
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.inFlight > 0 {
		t.inFlight--
	}
	t.latencies = append(t.latencies, elapsed)
	if len(t.latencies) > healthExecSampleSize {
		t.latencies = t.latencies[len(t.latencies)-healthExecSampleSize:]
	}
	if errMessage != "" {
		t.lastError = errMessage
	}
}

func (t *healthTracker) snapshot() healthSnapshot {
	if t == nil {
		return healthSnapshot{status: "ok"}
	}
	t.mu.Lock()
	queue := t.inFlight
	lastError := t.lastError
	p99 := percentileMS(t.latencies, 99)
	t.mu.Unlock()

	tokenOK := true
	if t.tokenReady != nil {
		tokenOK = t.tokenReady()
	}
	status := "ok"
	if !tokenOK {
		status = "down"
	} else if queue >= healthDegradedQueueDepth || (p99 != nil && *p99 > healthDegradedP99MS) {
		status = "degraded"
	}
	return healthSnapshot{
		status:     status,
		queueDepth: queue,
		p99ExecMS:  p99,
		lastError:  lastError,
		tokenOK:    tokenOK,
	}
}

type healthSnapshot struct {
	status     string
	queueDepth int
	p99ExecMS  *int64
	lastError  string
	tokenOK    bool
}

func (s healthSnapshot) result() map[string]any {
	result := map[string]any{
		"status":      s.status,
		"uptime_ms":   time.Since(startedAt).Milliseconds(),
		"queue_depth": s.queueDepth,
		"checks": map[string]any{
			"service":    map[string]any{"ok": true},
			"token_file": map[string]any{"ok": s.tokenOK},
			"exec": map[string]any{
				"ok":      s.status != "degraded" && s.status != "down",
				"message": execHealthMessage(s),
			},
		},
	}
	if s.p99ExecMS != nil {
		result["p99_exec_ms"] = *s.p99ExecMS
	} else {
		result["p99_exec_ms"] = nil
	}
	if s.lastError != "" {
		result["last_error"] = s.lastError
	}
	return result
}

func execHealthMessage(snapshot healthSnapshot) string {
	switch snapshot.status {
	case "down":
		return "agent token is missing or insecure"
	case "degraded":
		if snapshot.queueDepth >= healthDegradedQueueDepth {
			return "exec backlog"
		}
		return "exec p99 above threshold"
	default:
		return ""
	}
}

func percentileMS(samples []time.Duration, percentile int) *int64 {
	if len(samples) == 0 {
		return nil
	}
	ordered := append([]time.Duration(nil), samples...)
	sort.Slice(ordered, func(i, j int) bool { return ordered[i] < ordered[j] })
	index := (percentile*len(ordered) + 99) / 100
	if index < 1 {
		index = 1
	}
	if index > len(ordered) {
		index = len(ordered)
	}
	value := ordered[index-1].Milliseconds()
	return &value
}
