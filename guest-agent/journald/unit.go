package journald

import (
	"strings"
	"unicode"
)

// ValidUnit reports whether name looks like a systemd unit identifier.
func ValidUnit(unit string) bool {
	if unit == "" || len(unit) > 256 {
		return false
	}
	runes := []rune(unit)
	if len(runes) == 0 || !isUnitStart(runes[0]) {
		return false
	}
	for _, ch := range runes[1:] {
		if isUnitChar(ch) {
			continue
		}
		return false
	}
	return strings.Contains(unit, ".")
}

func isUnitStart(ch rune) bool {
	return unicode.IsLetter(ch) || unicode.IsDigit(ch)
}

func isUnitChar(ch rune) bool {
	return unicode.IsLetter(ch) || unicode.IsDigit(ch) || strings.ContainsRune("@._-\\", ch)
}
