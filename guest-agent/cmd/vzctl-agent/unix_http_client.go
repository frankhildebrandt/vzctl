package main

import (
	"context"
	"net"
	"net/http"
	"time"
)

func unixHTTPClient(socket string) *http.Client {
	dialer := net.Dialer{Timeout: 2 * time.Second}
	return &http.Client{
		Timeout: 2 * time.Second,
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				return dialer.DialContext(ctx, "unix", socket)
			},
		},
	}
}
