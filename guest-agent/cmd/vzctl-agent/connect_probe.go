package main

import (
	"context"
	"errors"
	"fmt"
	"net"
	"strconv"
	"strings"
	"time"
)

const maxNetworkProbeTargetBytes = 253

type connectProbeIO struct {
	lookup func(ctx context.Context, host string) ([]net.IP, error)
	dial   func(ctx context.Context, network, address string) (net.Conn, error)
}

var defaultConnectProbeIO = connectProbeIO{
	lookup: lookupIPs,
	dial:   (&net.Dialer{}).DialContext,
}

type probeLeg struct {
	OK          bool     `json:"ok"`
	ResolvedIPs []string `json:"resolved_ips,omitempty"`
	ChosenIP    string   `json:"chosen_ip,omitempty"`
	ConnectMS   int64    `json:"connect_ms"`
	ErrorStage  string   `json:"error_stage,omitempty"`
	Error       string   `json:"error,omitempty"`
}

type connectProbeResult struct {
	Target      string    `json:"target"`
	Via         string    `json:"via"`
	ResolvedIPs []string  `json:"resolved_ips,omitempty"`
	ChosenIP    string    `json:"chosen_ip,omitempty"`
	ConnectMS   int64     `json:"connect_ms"`
	ErrorStage  string    `json:"error_stage,omitempty"`
	Error       string    `json:"error,omitempty"`
	DNS         *probeLeg `json:"dns,omitempty"`
	IP          *probeLeg `json:"ip,omitempty"`
}

func validateConnectProbeParams(params networkProbeParams) (host string, port int, via string, timeout time.Duration, err error) {
	if params.URL != "" {
		return "", 0, "", 0, errors.New("network_probe cannot set both url and target")
	}
	if params.Target == "" || len(params.Target) > maxNetworkProbeTargetBytes+6 {
		return "", 0, "", 0, errors.New("network_probe target must be host:port")
	}
	host, port, err = splitHostPort(params.Target)
	if err != nil {
		return "", 0, "", 0, err
	}
	via = params.Via
	if via == "" {
		via = "dns"
	}
	switch via {
	case "dns", "ip", "both":
	default:
		return "", 0, "", 0, errors.New("network_probe via must be dns, ip, or both")
	}
	if params.ConnectIP != "" && net.ParseIP(params.ConnectIP) == nil {
		return "", 0, "", 0, errors.New("network_probe connect_ip must be an IP address")
	}
	timeout = defaultNetworkProbeTimeout
	if params.TimeoutMS != nil {
		if *params.TimeoutMS < 100 || *params.TimeoutMS > maxNetworkProbeTimeout.Milliseconds() {
			return "", 0, "", 0, errors.New("network_probe timeout_ms must be 100...30000")
		}
		timeout = time.Duration(*params.TimeoutMS) * time.Millisecond
	}
	return host, port, via, timeout, nil
}

func splitHostPort(target string) (string, int, error) {
	host, portText, err := net.SplitHostPort(target)
	if err != nil {
		return "", 0, errors.New("network_probe target must be host:port")
	}
	if host == "" || strings.ContainsAny(host, " /?#@") {
		return "", 0, errors.New("network_probe target host is invalid")
	}
	port, err := strconv.Atoi(portText)
	if err != nil || port < 1 || port > 65535 {
		return "", 0, errors.New("network_probe target port must be 1...65535")
	}
	return host, port, nil
}

func runConnectProbe(parent context.Context, params networkProbeParams, io connectProbeIO) connectProbeResult {
	host, port, via, timeout, err := validateConnectProbeParams(params)
	if err != nil {
		return connectProbeResult{Target: params.Target, Via: params.Via, ErrorStage: "proto", Error: err.Error()}
	}
	if io.lookup == nil {
		io.lookup = defaultConnectProbeIO.lookup
	}
	if io.dial == nil {
		io.dial = defaultConnectProbeIO.dial
	}
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()

	result := connectProbeResult{Target: fmt.Sprintf("%s:%d", host, port), Via: via}
	if via == "dns" || via == "both" {
		leg := probeViaDNS(ctx, io, host, port)
		result.DNS = &leg
	}
	if via == "ip" || via == "both" {
		leg := probeViaIP(ctx, io, host, port, params.ConnectIP)
		result.IP = &leg
	}
	promoteConnectProbe(&result)
	return result
}

func probeViaDNS(ctx context.Context, io connectProbeIO, host string, port int) probeLeg {
	started := time.Now()
	if net.ParseIP(host) != nil {
		return dialLeg(ctx, io, started, []string{host}, host, port, "tcp")
	}
	ips, err := io.lookup(ctx, host)
	if err != nil {
		return probeLeg{
			ConnectMS:  time.Since(started).Milliseconds(),
			ErrorStage: classifyConnectError(err, "dns"),
			Error:      sanitizeProbeError(err),
		}
	}
	resolved := make([]string, 0, len(ips))
	for _, ip := range ips {
		resolved = append(resolved, ip.String())
	}
	if len(resolved) == 0 {
		return probeLeg{
			ConnectMS:  time.Since(started).Milliseconds(),
			ErrorStage: "dns",
			Error:      "no addresses",
		}
	}
	return dialLeg(ctx, io, started, resolved, resolved[0], port, "tcp")
}

func probeViaIP(ctx context.Context, io connectProbeIO, host string, port int, connectIP string) probeLeg {
	started := time.Now()
	chosen := connectIP
	if chosen == "" {
		if net.ParseIP(host) == nil {
			return probeLeg{
				ConnectMS:  time.Since(started).Milliseconds(),
				ErrorStage: "dns",
				Error:      "connect_ip required when target is not an IP",
			}
		}
		chosen = host
	}
	return dialLeg(ctx, io, started, nil, chosen, port, "tcp")
}

func dialLeg(ctx context.Context, io connectProbeIO, started time.Time, resolved []string, chosen string, port int, stage string) probeLeg {
	address := net.JoinHostPort(chosen, strconv.Itoa(port))
	conn, err := io.dial(ctx, "tcp", address)
	leg := probeLeg{
		ResolvedIPs: resolved,
		ChosenIP:    chosen,
		ConnectMS:   time.Since(started).Milliseconds(),
	}
	if err != nil {
		leg.ErrorStage = classifyConnectError(err, stage)
		leg.Error = sanitizeProbeError(err)
		return leg
	}
	_ = conn.Close()
	leg.OK = true
	return leg
}

func promoteConnectProbe(result *connectProbeResult) {
	if result.DNS != nil {
		result.ResolvedIPs = result.DNS.ResolvedIPs
	}
	switch {
	case result.Via == "dns" && result.DNS != nil:
		copyLegSummary(result, result.DNS)
	case result.Via == "ip" && result.IP != nil:
		copyLegSummary(result, result.IP)
	case result.Via == "both":
		if result.IP != nil && result.IP.OK {
			result.ChosenIP = result.IP.ChosenIP
			result.ConnectMS = result.IP.ConnectMS
		} else if result.DNS != nil && result.DNS.OK {
			result.ChosenIP = result.DNS.ChosenIP
			result.ConnectMS = result.DNS.ConnectMS
		} else if result.IP != nil {
			result.ChosenIP = result.IP.ChosenIP
			result.ConnectMS = result.IP.ConnectMS
		}
		if result.DNS != nil && !result.DNS.OK {
			result.ErrorStage = result.DNS.ErrorStage
			result.Error = result.DNS.Error
		} else if result.IP != nil && !result.IP.OK {
			result.ErrorStage = result.IP.ErrorStage
			result.Error = result.IP.Error
		}
	}
}

func copyLegSummary(result *connectProbeResult, leg *probeLeg) {
	result.ChosenIP = leg.ChosenIP
	result.ConnectMS = leg.ConnectMS
	result.ErrorStage = leg.ErrorStage
	result.Error = leg.Error
	if len(result.ResolvedIPs) == 0 {
		result.ResolvedIPs = leg.ResolvedIPs
	}
}

func classifyConnectError(err error, fallback string) string {
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return "timeout"
	}
	var dnsError *net.DNSError
	if errors.As(err, &dnsError) {
		return "dns"
	}
	var opError *net.OpError
	if errors.As(err, &opError) && opError.Op == "dial" {
		return "tcp"
	}
	return fallback
}

func sanitizeProbeError(err error) string {
	if err == nil {
		return ""
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return "timeout"
	}
	var dnsError *net.DNSError
	if errors.As(err, &dnsError) {
		return "no such host"
	}
	message := err.Error()
	switch {
	case strings.Contains(message, "connection refused"):
		return "connection refused"
	case strings.Contains(message, "timeout"):
		return "timeout"
	case strings.Contains(message, "no such host"):
		return "no such host"
	default:
		return "connect failed"
	}
}

func lookupIPs(ctx context.Context, host string) ([]net.IP, error) {
	addrs, err := net.DefaultResolver.LookupIPAddr(ctx, host)
	if err != nil {
		return nil, err
	}
	ips := make([]net.IP, 0, len(addrs))
	for _, addr := range addrs {
		ips = append(ips, addr.IP)
	}
	return ips, nil
}
