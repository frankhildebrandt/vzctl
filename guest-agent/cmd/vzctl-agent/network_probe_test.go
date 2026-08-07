package main

import (
	"context"
	"crypto/tls"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptrace"
	"strings"
	"testing"
	"time"
)

func TestNetworkProbeClassifiesOnlineAndCaptiveWithoutFollowingRedirect(t *testing.T) {
	result := runNetworkProbeWithTransport(
		context.Background(),
		"https://probe.invalid/",
		time.Second,
		probeRoundTripper{status: http.StatusNoContent},
	)
	if result.Classification != "online" || result.StatusCode != http.StatusNoContent {
		t.Fatalf("online result = %#v", result)
	}

	result = runNetworkProbeWithTransport(
		context.Background(),
		"https://probe.invalid/",
		time.Second,
		probeRoundTripper{status: http.StatusForbidden},
	)
	if result.Classification != "captive" {
		t.Fatalf("captive result = %#v", result)
	}
}

func TestNetworkProbeClassifiesDNSAndTLSErrorsByPhase(t *testing.T) {
	tests := []struct {
		name  string
		phase string
		err   error
		code  string
	}{
		{name: "dns", phase: "dns", err: &net.DNSError{Err: "no such host"}, code: "dns"},
		{name: "tls", phase: "tls", err: tls.RecordHeaderError{}, code: "tls"},
		{name: "tcp", phase: "tcp", err: errors.New("refused"), code: "connect"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			result := runNetworkProbeWithTransport(
				context.Background(), "https://probe.invalid/", time.Second,
				probeErrorRoundTripper{phase: test.phase, err: test.err},
			)
			if result.Classification != "offline" || result.Phase != test.phase || result.ErrorCode != test.code {
				t.Fatalf("result = %#v", result)
			}
		})
	}
}

func TestNetworkProbeTimeoutIsBoundedAndSanitized(t *testing.T) {
	result := runNetworkProbeWithTransport(
		context.Background(), "https://probe.invalid/private?token=secret", 20*time.Millisecond,
		probeTimeoutRoundTripper{},
	)
	if result.Classification != "offline" || result.Phase != "http" || result.ErrorCode != "timeout" {
		t.Fatalf("timeout result = %#v", result)
	}
}

type probeRoundTripper struct{ status int }

func (p probeRoundTripper) RoundTrip(request *http.Request) (*http.Response, error) {
	return &http.Response{
		StatusCode: p.status,
		Header:     make(http.Header),
		Body:       io.NopCloser(strings.NewReader("")),
		Request:    request,
	}, nil
}

type probeErrorRoundTripper struct {
	phase string
	err   error
}

func (p probeErrorRoundTripper) RoundTrip(request *http.Request) (*http.Response, error) {
	trace := httptrace.ContextClientTrace(request.Context())
	if trace != nil {
		switch p.phase {
		case "dns":
			trace.DNSStart(httptrace.DNSStartInfo{})
		case "tcp":
			trace.ConnectStart("tcp", "")
		case "tls":
			trace.TLSHandshakeStart()
		}
	}
	return nil, p.err
}

type probeTimeoutRoundTripper struct{}

func (probeTimeoutRoundTripper) RoundTrip(request *http.Request) (*http.Response, error) {
	if trace := httptrace.ContextClientTrace(request.Context()); trace != nil {
		trace.WroteRequest(httptrace.WroteRequestInfo{})
	}
	<-request.Context().Done()
	return nil, request.Context().Err()
}

func TestNetworkProbeValidationRejectsCredentialsAndUnboundedTimeout(t *testing.T) {
	for _, params := range []networkProbeParams{
		{URL: "file:///tmp/a"},
		{URL: "https://user:secret@example.com/"},
		{URL: "https://example.com/", TimeoutMS: int64Pointer(30_001)},
	} {
		if _, err := validateNetworkProbeParams(params); err == nil {
			t.Fatalf("expected validation error for %#v", params)
		}
	}
}

func int64Pointer(value int64) *int64 { return &value }
