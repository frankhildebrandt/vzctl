package systemd

import (
	"fmt"
	"strings"
)

const maxUnitNameLen = 256

// ValidateUnitName rejects unsafe or malformed systemd unit names.
func ValidateUnitName(name string) error {
	if name == "" {
		return fmt.Errorf("unit name is required")
	}
	if len(name) > maxUnitNameLen {
		return fmt.Errorf("unit name exceeds %d bytes", maxUnitNameLen)
	}
	if strings.Contains(name, ";") || strings.Contains(name, "..") {
		return fmt.Errorf("unit name contains invalid characters")
	}
	if strings.ContainsAny(name, "\x00\n\r") {
		return fmt.Errorf("unit name contains invalid characters")
	}
	return nil
}

// ValidateUnitType ensures list filters stay within the v1 contract.
func ValidateUnitType(raw string) (string, error) {
	switch raw {
	case "", "service":
		return "service", nil
	case "timer", "socket":
		return raw, nil
	default:
		return "", fmt.Errorf("unsupported unit type %q", raw)
	}
}

// ValidateControlAction ensures lifecycle verbs stay within the v1 contract.
func ValidateControlAction(action string) error {
	switch action {
	case "start", "stop", "restart":
		return nil
	default:
		return fmt.Errorf("unsupported control action %q", action)
	}
}
