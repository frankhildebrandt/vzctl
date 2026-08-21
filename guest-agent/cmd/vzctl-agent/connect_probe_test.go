package main

import (
	"context"
	"errors"
	"net"
	"testing"
	"time"
)

func TestConnectProbeDNSFailTCPOkBoth(t *testing.T) {
	io := connectProbeIO{
		lookup: func(context.Context, string) ([]net.IP, error) {
			return nil, &net.DNSError{Err: "no such host", Name: "main-node.core.neti.vz.test", IsNotFound: true}
		},
		dial: func(_ context.Context, _, address string) (net.Conn, error) {
			if address != "10.90.0.2:4222" {
				t.Fatalf("dialed %s", address)
			}
			return discardConn{}, nil
		},
	}
	result := runConnectProbe(context.Background(), networkProbeParams{
		Target:    "main-node.core.neti.vz.test:4222",
		Via:       "both",
		ConnectIP: "10.90.0.2",
	}, io)
	if result.DNS == nil || result.DNS.OK || result.DNS.ErrorStage != "dns" {
		t.Fatalf("dns leg = %#v", result.DNS)
	}
	if result.IP == nil || !result.IP.OK || result.IP.ChosenIP != "10.90.0.2" {
		t.Fatalf("ip leg = %#v", result.IP)
	}
	if result.ErrorStage != "dns" || result.ChosenIP != "10.90.0.2" {
		t.Fatalf("summary = %#v", result)
	}
}

func TestConnectProbeTCPTimeout(t *testing.T) {
	io := connectProbeIO{
		lookup: func(context.Context, string) ([]net.IP, error) {
			return []net.IP{net.ParseIP("10.90.0.2")}, nil
		},
		dial: func(ctx context.Context, _, _ string) (net.Conn, error) {
			<-ctx.Done()
			return nil, ctx.Err()
		},
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	result := runConnectProbe(ctx, networkProbeParams{
		Target: "10.90.0.2:4222",
		Via:    "ip",
	}, io)
	if result.IP == nil || result.IP.OK || result.IP.ErrorStage != "timeout" {
		t.Fatalf("timeout leg = %#v", result.IP)
	}
}

func TestConnectProbeTCPOkViaDNS(t *testing.T) {
	io := connectProbeIO{
		lookup: func(context.Context, string) ([]net.IP, error) {
			return []net.IP{net.ParseIP("10.90.0.2")}, nil
		},
		dial: func(_ context.Context, _, address string) (net.Conn, error) {
			if address != "10.90.0.2:4222" {
				t.Fatalf("dialed %s", address)
			}
			return discardConn{}, nil
		},
	}
	result := runConnectProbe(context.Background(), networkProbeParams{
		Target: "main-node.core.neti.vz.test:4222",
		Via:    "dns",
	}, io)
	if result.DNS == nil || !result.DNS.OK || result.ChosenIP != "10.90.0.2" {
		t.Fatalf("dns ok result = %#v", result)
	}
}

func TestConnectProbeRejectsURLAndTargetTogether(t *testing.T) {
	_, _, _, _, err := validateConnectProbeParams(networkProbeParams{
		URL:    "https://example.test/",
		Target: "10.90.0.2:4222",
	})
	if err == nil {
		t.Fatal("expected validation error")
	}
}

func TestConnectProbeIPRequiresConnectIPForHostname(t *testing.T) {
	result := runConnectProbe(context.Background(), networkProbeParams{
		Target: "main-node.core.neti.vz.test:4222",
		Via:    "ip",
	}, connectProbeIO{
		dial: func(context.Context, string, string) (net.Conn, error) {
			t.Fatal("must not dial")
			return nil, errors.New("unused")
		},
	})
	if result.IP == nil || result.IP.OK || result.IP.ErrorStage != "dns" {
		t.Fatalf("ip without connect_ip = %#v", result.IP)
	}
}

type discardConn struct{ net.Conn }

func (discardConn) Close() error { return nil }

func (discardConn) Read([]byte) (int, error)  { return 0, errors.New("unused") }
func (discardConn) Write([]byte) (int, error) { return 0, errors.New("unused") }
func (discardConn) LocalAddr() net.Addr       { return nil }
func (discardConn) RemoteAddr() net.Addr      { return nil }
func (discardConn) SetDeadline(time.Time) error      { return nil }
func (discardConn) SetReadDeadline(time.Time) error  { return nil }
func (discardConn) SetWriteDeadline(time.Time) error { return nil }
