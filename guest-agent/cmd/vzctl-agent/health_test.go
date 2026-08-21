package main

import (
	"testing"
	"time"
)

func TestHealthDegradedOnExecBacklog(t *testing.T) {
	tracker := &healthTracker{tokenReady: func() bool { return true }}
	tracker.beginExec()
	tracker.beginExec()
	snapshot := tracker.snapshot()
	if snapshot.status != "degraded" || snapshot.queueDepth != 2 {
		t.Fatalf("snapshot = %+v", snapshot)
	}
}

func TestHealthDegradedOnHighP99(t *testing.T) {
	tracker := &healthTracker{tokenReady: func() bool { return true }}
	now := time.Now()
	tracker.endExec(now.Add(-6*time.Second), "")
	snapshot := tracker.snapshot()
	if snapshot.status != "degraded" || snapshot.p99ExecMS == nil || *snapshot.p99ExecMS < healthDegradedP99MS {
		t.Fatalf("snapshot = %+v", snapshot)
	}
}

func TestHealthDownWhenTokenInsecure(t *testing.T) {
	tracker := &healthTracker{tokenReady: func() bool { return false }}
	snapshot := tracker.snapshot()
	if snapshot.status != "down" {
		t.Fatalf("status = %s", snapshot.status)
	}
	result := snapshot.result()
	if result["status"] != "down" {
		t.Fatalf("result = %#v", result)
	}
}

func TestHealthOkWithoutSamples(t *testing.T) {
	tracker := &healthTracker{tokenReady: func() bool { return true }}
	snapshot := tracker.snapshot()
	if snapshot.status != "ok" || snapshot.p99ExecMS != nil {
		t.Fatalf("snapshot = %+v", snapshot)
	}
}
