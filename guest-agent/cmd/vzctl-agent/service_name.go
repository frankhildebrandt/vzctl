package main

import (
	"fmt"
	"strings"
	"unicode"
)

func validateServiceName(value string) error {
	value = strings.ToLower(strings.TrimSpace(value))
	if value == "" {
		return fmt.Errorf("service name is required")
	}
	if value == "svc" {
		return fmt.Errorf("service name %q is reserved", value)
	}
	if len(value) > 63 {
		return fmt.Errorf("service name is too long")
	}
	if value[0] == '-' || value[len(value)-1] == '-' {
		return fmt.Errorf("invalid service name")
	}
	for _, r := range value {
		if unicode.IsDigit(r) || (r >= 'a' && r <= 'z') || r == '-' {
			continue
		}
		return fmt.Errorf("invalid service name")
	}
	return nil
}
