package systemd

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"
)

// SystemctlRunner executes systemctl with argv-only semantics.
type SystemctlRunner func(ctx context.Context, args ...string) (stdout string, exitCode int, err error)

// Manager implements guest systemd list/show/control and event buffering.
type Manager struct {
	mu      sync.Mutex
	events  *eventBuffer
	watcher *watcherHandle
	run     SystemctlRunner
}

type watcherHandle struct {
	cancel context.CancelFunc
}

// NewManager constructs a systemd manager with the default systemctl runner.
func NewManager() *Manager {
	return &Manager{
		events: newEventBuffer(defaultEventCapacity),
		run:    defaultSystemctlRunner,
	}
}

func defaultSystemctlRunner(ctx context.Context, args ...string) (string, int, error) {
	cmd := exec.CommandContext(ctx, systemctlBinary(), args...)
	cmd.Env = systemctlEnv()
	output, err := cmd.CombinedOutput()
	stdout := string(output)
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			return stdout, exitErr.ExitCode(), nil
		}
		return stdout, -1, err
	}
	return stdout, 0, nil
}

func systemctlBinary() string {
	if path, err := exec.LookPath("systemctl"); err == nil {
		return path
	}
	return "/usr/bin/systemctl"
}

func systemctlEnv() []string {
	return append(os.Environ(), "LC_ALL=C")
}

// Available reports whether systemd is the guest init.
func Available() bool {
	if _, err := os.Stat("/run/systemd/system"); err != nil {
		return false
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultExecTimeout)
	defer cancel()
	_, exit, err := defaultSystemctlRunner(ctx, "--version")
	return err == nil && exit == 0
}

// Status returns capability information for the guest.
func (m *Manager) Status(ctx context.Context) Status {
	if !Available() {
		return Status{Available: false}
	}
	stdout, exit, err := m.run(ctx, "--version")
	if err != nil || exit != 0 {
		return Status{Available: false}
	}
	version := parseSystemctlVersion(stdout)
	return Status{Available: true, Version: version}
}

func parseSystemctlVersion(stdout string) string {
	for _, line := range strings.Split(stdout, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "systemd ") {
			fields := strings.Fields(line)
			if len(fields) >= 2 {
				return fields[1]
			}
		}
	}
	return ""
}

// List returns normalized units for the requested type.
func (m *Manager) List(ctx context.Context, unitType string, all bool) ([]Unit, error) {
	if !Available() {
		return nil, ErrUnavailable
	}
	unitType, err := ValidateUnitType(unitType)
	if err != nil {
		return nil, err
	}
	args := []string{
		"list-units",
		"--type=" + unitType,
		"--output=json",
		"--no-pager",
		"--no-legend",
	}
	if all {
		args = append(args, "--all")
	}
	stdout, exit, err := m.run(ctx, args...)
	if err != nil {
		return nil, err
	}
	if exit != 0 {
		return nil, fmt.Errorf("systemctl list-units failed: %s", strings.TrimSpace(stdout))
	}
	return parseListUnits(stdout, unitType)
}

func parseListUnits(stdout, unitType string) ([]Unit, error) {
	stdout = strings.TrimSpace(stdout)
	if stdout == "" || stdout == "[]" {
		return []Unit{}, nil
	}
	var rows []map[string]any
	if err := json.Unmarshal([]byte(stdout), &rows); err != nil {
		return nil, fmt.Errorf("invalid systemctl json: %w", err)
	}
	units := make([]Unit, 0, len(rows))
	for _, row := range rows {
		unit := Unit{
			Name:        stringField(row, "unit"),
			Type:        unitType,
			Load:        stringField(row, "load"),
			Active:      stringField(row, "active"),
			Sub:         stringField(row, "sub"),
			Description: stringField(row, "description"),
		}
		if unit.Name == "" {
			continue
		}
		units = append(units, unit)
	}
	return units, nil
}

// Show returns selected unit properties.
func (m *Manager) Show(ctx context.Context, unit string) (map[string]string, error) {
	if !Available() {
		return nil, ErrUnavailable
	}
	if err := ValidateUnitName(unit); err != nil {
		return nil, err
	}
	stdout, exit, err := m.run(ctx,
		"show", unit,
		"--property=LoadState,ActiveState,SubState,Description,UnitFileState,FragmentPath,Id",
		"--output=json",
		"--no-pager",
	)
	if err != nil {
		return nil, err
	}
	if exit != 0 {
		return nil, fmt.Errorf("systemctl show failed: %s", strings.TrimSpace(stdout))
	}
	return parseShow(stdout, unit)
}

func parseShow(stdout, unit string) (map[string]string, error) {
	stdout = strings.TrimSpace(stdout)
	if stdout == "" {
		return nil, fmt.Errorf("unit %q not found", unit)
	}
	var rows []map[string]any
	if err := json.Unmarshal([]byte(stdout), &rows); err != nil {
		return nil, fmt.Errorf("invalid systemctl show json: %w", err)
	}
	if len(rows) == 0 {
		return nil, fmt.Errorf("unit %q not found", unit)
	}
	row := rows[0]
	props := map[string]string{
		"name":        stringField(row, "Id", "name", "unit"),
		"load":        stringField(row, "LoadState", "load"),
		"active":      stringField(row, "ActiveState", "active"),
		"sub":         stringField(row, "SubState", "sub"),
		"description": stringField(row, "Description", "description"),
		"unit_file":   stringField(row, "UnitFileState", "unit_file"),
		"fragment":    stringField(row, "FragmentPath", "fragment"),
	}
	if props["name"] == "" {
		props["name"] = unit
	}
	props["type"] = unitSuffixType(props["name"])
	return props, nil
}

func unitSuffixType(name string) string {
	switch {
	case strings.HasSuffix(name, ".timer"):
		return "timer"
	case strings.HasSuffix(name, ".socket"):
		return "socket"
	default:
		return "service"
	}
}

// Control runs start/stop/restart on a unit.
func (m *Manager) Control(ctx context.Context, unit, action string) error {
	if !Available() {
		return ErrUnavailable
	}
	if err := ValidateUnitName(unit); err != nil {
		return err
	}
	if err := ValidateControlAction(action); err != nil {
		return err
	}
	stdout, exit, err := m.run(ctx, action, unit)
	if err != nil {
		return err
	}
	if exit != 0 {
		return fmt.Errorf("systemctl %s %s failed: %s", action, unit, strings.TrimSpace(stdout))
	}
	return nil
}

// Events returns buffered unit changes since the provided cursor.
func (m *Manager) Events(since string, limit int) ([]Event, string) {
	events, cursor := m.events.since(since, limit)
	out := make([]Event, len(events))
	copy(out, events)
	return out, cursor
}

// EnsureWatcher starts the D-Bus-backed watcher once per manager.
func (m *Manager) EnsureWatcher(ctx context.Context) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.watcher != nil {
		return
	}
	m.watcher = startWatcher(ctx, m)
}

func stringField(row map[string]any, keys ...string) string {
	for _, key := range keys {
		if value, ok := row[key]; ok {
			switch typed := value.(type) {
			case string:
				return typed
			case fmt.Stringer:
				return typed.String()
			}
		}
	}
	return ""
}

const defaultExecTimeout = 30 * time.Second

// ErrUnavailable is returned when the guest does not run systemd.
var ErrUnavailable = errors.New("systemd is not available on this guest")
