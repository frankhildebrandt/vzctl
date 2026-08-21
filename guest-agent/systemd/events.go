package systemd

import (
	"strconv"
	"strings"
	"sync"
	"time"
)

const defaultEventCapacity = 512

type eventBuffer struct {
	mu     sync.RWMutex
	events []Event
	next   int
	full   bool
}

func newEventBuffer(capacity int) *eventBuffer {
	if capacity <= 0 {
		capacity = defaultEventCapacity
	}
	return &eventBuffer{events: make([]Event, 0, capacity)}
}

func (b *eventBuffer) append(event Event) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.events) < cap(b.events) {
		b.events = append(b.events, event)
		return
	}
	if cap(b.events) == 0 {
		b.events = append(b.events, event)
		return
	}
	b.events[b.next] = event
	b.next = (b.next + 1) % cap(b.events)
	b.full = true
}

func (b *eventBuffer) since(sinceID string, limit int) ([]Event, string) {
	if limit <= 0 {
		limit = 100
	}
	if limit > cap(b.events) && cap(b.events) > 0 {
		limit = cap(b.events)
	}
	b.mu.RLock()
	defer b.mu.RUnlock()
	ordered := b.orderedLocked()
	if sinceID == "" {
		return tail(ordered, limit), lastID(ordered)
	}
	start := 0
	for i, event := range ordered {
		if event.ID == sinceID {
			start = i + 1
			break
		}
	}
	return tail(ordered[start:], limit), lastID(ordered)
}

func (b *eventBuffer) orderedLocked() []Event {
	if !b.full {
		out := make([]Event, len(b.events))
		copy(out, b.events)
		return out
	}
	out := make([]Event, 0, cap(b.events))
	out = append(out, b.events[b.next:]...)
	out = append(out, b.events[:b.next]...)
	return out
}

func tail(events []Event, limit int) []Event {
	if len(events) <= limit {
		out := make([]Event, len(events))
		copy(out, events)
		return out
	}
	out := make([]Event, limit)
	copy(out, events[len(events)-limit:])
	return out
}

// appendWatchedUnitEvent records a service/timer/socket state change.
func appendWatchedUnitEvent(manager *Manager, name, load, active, sub string, at time.Time) {
	if manager == nil || name == "" {
		return
	}
	unitType := unitSuffixType(name)
	if unitType != "service" && unitType != "timer" && unitType != "socket" {
		return
	}
	manager.events.append(newEvent(name, unitType, load, active, sub, "properties_changed", at))
}

func lastID(events []Event) string {
	if len(events) == 0 {
		return ""
	}
	return events[len(events)-1].ID
}

func eventIDLess(left, right string) bool {
	if left == right {
		return false
	}
	leftAt, leftUnit := splitEventID(left)
	rightAt, rightUnit := splitEventID(right)
	if leftAt != rightAt {
		return leftAt < rightAt
	}
	return leftUnit < rightUnit
}

func splitEventID(id string) (int64, string) {
	parts := strings.SplitN(id, ":", 2)
	if len(parts) != 2 {
		return 0, id
	}
	at, err := strconv.ParseInt(parts[0], 10, 64)
	if err != nil {
		return 0, id
	}
	return at, parts[1]
}
