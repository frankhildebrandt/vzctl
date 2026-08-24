package journald

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

type runner func(ctx context.Context, unit string, args []string) ([]byte, error)

func defaultRunner(ctx context.Context, unit string, args []string) ([]byte, error) {
	return runJournalctl(ctx, unit, args)
}

func journalctlArgv(unit string, args []string) []string {
	binary := journalctlBinary()
	return append([]string{binary, "-u", unit, "--no-pager", "-o", "json"}, args...)
}

func journalctlCommand(ctx context.Context, unit string, args []string) *exec.Cmd {
	argv := journalctlArgv(unit, args)
	cmd := exec.CommandContext(ctx, "sudo", append([]string{"-n"}, argv...)...)
	cmd.Env = append(os.Environ(), "LC_ALL=C")
	return cmd
}

func runJournalctl(ctx context.Context, unit string, args []string) ([]byte, error) {
	cmd := journalctlCommand(ctx, unit, args)
	output, err := cmd.CombinedOutput()
	if err != nil {
		trimmed := strings.TrimSpace(string(output))
		if trimmed == "" {
			return nil, nil
		}
		if isJournalctlNoEntries(trimmed) {
			return nil, nil
		}
		return output, fmt.Errorf("%s: %s", err.Error(), trimmed)
	}
	return output, nil
}

func isJournalctlNoEntries(message string) bool {
	lower := strings.ToLower(message)
	return strings.Contains(lower, "no entries") ||
		strings.Contains(lower, "failed to match") ||
		strings.Contains(lower, "specifier not found")
}

func journalctlBinary() string {
	if path, err := exec.LookPath("journalctl"); err == nil {
		return path
	}
	return "/usr/bin/journalctl"
}

func snapshotLines(ctx context.Context, unit string, query Query, run runner) ([]Line, error) {
	if run == nil {
		run = defaultRunner
	}
	tail := query.Tail
	if tail <= 0 {
		tail = 200
	}
	args := []string{"-n", fmt.Sprintf("%d", tail)}
	if priority := priorityArg(query.MinLevel); priority != "" {
		args = append(args, "-p", priority)
	}
	raw, err := run(ctx, unit, args)
	if err != nil {
		return nil, err
	}
	if len(raw) > maxSnapshotBytes {
		raw = raw[len(raw)-maxSnapshotBytes:]
		if idx := bytes.IndexByte(raw, '\n'); idx >= 0 {
			raw = raw[idx+1:]
		}
	}
	lines, err := parseJournalJSON(unit, raw)
	if err != nil {
		return nil, err
	}
	return applyQueryFilters(lines, query), nil
}
