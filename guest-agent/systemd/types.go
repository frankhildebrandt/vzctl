package systemd

import (
	"fmt"
	"time"
)

// Status reports whether PID 1 is systemd on this guest.
type Status struct {
	Available bool   `json:"available"`
	Version   string `json:"version,omitempty"`
}

// Unit is a normalized systemd unit row for list/show responses.
type Unit struct {
	Name        string `json:"name"`
	Type        string `json:"type"`
	Load        string `json:"load"`
	Active      string `json:"active"`
	Sub         string `json:"sub"`
	Description string `json:"description,omitempty"`
}

// Event is a normalized unit state change emitted to subscribers.
type Event struct {
	ID       string `json:"id"`
	Unit     string `json:"unit"`
	UnitType string `json:"unit_type"`
	Load     string `json:"load"`
	Active   string `json:"active"`
	Sub      string `json:"sub"`
	Reason   string `json:"reason"`
	At       string `json:"at"`
}

func newEvent(unit, unitType, load, active, sub, reason string, at time.Time) Event {
	return Event{
		ID:       formatEventID(at, unit),
		Unit:     unit,
		UnitType: unitType,
		Load:     load,
		Active:   active,
		Sub:      sub,
		Reason:   reason,
		At:       at.UTC().Format(time.RFC3339Nano),
	}
}

func formatEventID(at time.Time, unit string) string {
	return fmtEventID(at.UnixNano(), unit)
}

func fmtEventID(nanos int64, unit string) string {
	return fmt.Sprintf("%d:%s", nanos, unit)
}
