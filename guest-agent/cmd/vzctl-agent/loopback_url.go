package main

import (
	"fmt"
	"net/url"
	"strings"
)

func validateLoopbackURL(raw string) error {
	parsed, err := url.Parse(strings.TrimSpace(raw))
	if err != nil {
		return fmt.Errorf("invalid url")
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return fmt.Errorf("url must be http or https")
	}
	host := strings.ToLower(parsed.Hostname())
	if host != "127.0.0.1" && host != "localhost" && host != "::1" {
		return fmt.Errorf("url host must be loopback")
	}
	if parsed.Port() == "" {
		return fmt.Errorf("url must include a port")
	}
	return nil
}
