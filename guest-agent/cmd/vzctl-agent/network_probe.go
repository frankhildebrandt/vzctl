package main

import (
	"context"
	"crypto/tls"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptrace"
	"net/url"
	"strings"
	"sync"
	"time"
)

const (
	defaultNetworkProbeTimeout = 5 * time.Second
	maxNetworkProbeTimeout     = 30 * time.Second
	maxNetworkProbeURLBytes    = 2_048
	maxNetworkProbeBodyBytes   = 4_096
)

type networkProbeParams struct {
	URL       string `json:"url"`
	TimeoutMS *int64 `json:"timeout_ms,omitempty"`
}

type networkProbeResult struct {
	Classification string `json:"classification"`
	Phase          string `json:"phase"`
	StatusCode     int    `json:"status_code,omitempty"`
	LatencyMS      int64  `json:"latency_ms"`
	Redirected     bool   `json:"redirected"`
	ErrorCode      string `json:"error_code,omitempty"`
}

func handleNetworkProbe(parent context.Context, req request) response {
	var params networkProbeParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid network_probe parameters", nil)
	}
	timeout, validationError := validateNetworkProbeParams(params)
	if validationError != nil {
		return errorResponse(req.ID, "proto", validationError.Error(), nil)
	}
	return successResponse(req.ID, runNetworkProbe(parent, params.URL, timeout))
}

func validateNetworkProbeParams(params networkProbeParams) (time.Duration, error) {
	if params.URL == "" || len(params.URL) > maxNetworkProbeURLBytes {
		return 0, errors.New("network_probe url must be 1...2048 bytes")
	}
	parsed, err := url.Parse(params.URL)
	if err != nil || parsed.Host == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		return 0, errors.New("network_probe url must be an http or https URL")
	}
	if parsed.User != nil {
		return 0, errors.New("network_probe url must not contain credentials")
	}
	timeout := defaultNetworkProbeTimeout
	if params.TimeoutMS != nil {
		if *params.TimeoutMS < 100 || *params.TimeoutMS > maxNetworkProbeTimeout.Milliseconds() {
			return 0, errors.New("network_probe timeout_ms must be 100...30000")
		}
		timeout = time.Duration(*params.TimeoutMS) * time.Millisecond
	}
	return timeout, nil
}

func runNetworkProbe(parent context.Context, target string, timeout time.Duration) networkProbeResult {
	return runNetworkProbeWithTransport(parent, target, timeout, nil)
}

func runNetworkProbeWithTransport(
	parent context.Context,
	target string,
	timeout time.Duration,
	injected http.RoundTripper,
) networkProbeResult {
	started := time.Now()
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()

	phase := "dns"
	var phaseMu sync.Mutex
	setPhase := func(value string) {
		phaseMu.Lock()
		phase = value
		phaseMu.Unlock()
	}
	currentPhase := func() string {
		phaseMu.Lock()
		defer phaseMu.Unlock()
		return phase
	}
	trace := &httptrace.ClientTrace{
		DNSStart:          func(httptrace.DNSStartInfo) { setPhase("dns") },
		ConnectStart:      func(_, _ string) { setPhase("tcp") },
		TLSHandshakeStart: func() { setPhase("tls") },
		WroteRequest:      func(httptrace.WroteRequestInfo) { setPhase("http") },
	}
	request, err := http.NewRequestWithContext(httptrace.WithClientTrace(ctx, trace), http.MethodGet, target, nil)
	if err != nil {
		return failedNetworkProbe(started, "http", "request")
	}
	request.Header.Set("User-Agent", "vzctl-agent-network-probe/1")

	var transport http.RoundTripper = injected
	if transport == nil {
		configured := http.DefaultTransport.(*http.Transport).Clone()
		configured.Proxy = http.ProxyFromEnvironment
		configured.DialContext = (&net.Dialer{Timeout: timeout, KeepAlive: 0}).DialContext
		configured.TLSHandshakeTimeout = timeout
		configured.DisableKeepAlives = true
		transport = configured
	}
	client := &http.Client{
		Transport: transport,
		Timeout:   timeout,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	response, err := client.Do(request)
	if err != nil {
		return failedNetworkProbe(started, currentPhase(), classifyNetworkProbeError(err))
	}
	defer response.Body.Close()
	_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, maxNetworkProbeBodyBytes))
	redirected := response.StatusCode >= 300 && response.StatusCode < 400
	classification := "online"
	if redirected || response.StatusCode < 200 || response.StatusCode >= 300 {
		classification = "captive"
	}
	return networkProbeResult{
		Classification: classification,
		Phase:          "http",
		StatusCode:     response.StatusCode,
		LatencyMS:      time.Since(started).Milliseconds(),
		Redirected:     redirected,
	}
}

func failedNetworkProbe(started time.Time, phase, code string) networkProbeResult {
	return networkProbeResult{
		Classification: "offline",
		Phase:          phase,
		LatencyMS:      time.Since(started).Milliseconds(),
		ErrorCode:      code,
	}
}

func classifyNetworkProbeError(err error) string {
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return "timeout"
	}
	var dnsError *net.DNSError
	if errors.As(err, &dnsError) {
		return "dns"
	}
	var tlsError tls.RecordHeaderError
	if errors.As(err, &tlsError) || strings.Contains(err.Error(), "certificate") {
		return "tls"
	}
	return "connect"
}
