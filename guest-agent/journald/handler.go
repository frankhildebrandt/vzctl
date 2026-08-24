package journald

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/frankhildebrandt/vzctl/guest-agent/systemd"
)

// ServeHTTP handles builtin journald paths when no guest_publish service exists.
func ServeHTTP(ctx context.Context, unit string, method string, path string) (int, map[string]string, []byte, error) {
	if !systemd.Available() {
		return http.StatusServiceUnavailable, nil, []byte("journald unavailable"), nil
	}
	query, route, err := ParseQuery(path)
	if err != nil {
		return http.StatusBadRequest, nil, []byte(err.Error()), nil
	}
	switch route {
	case "/api/status":
		if method != http.MethodGet {
			return http.StatusMethodNotAllowed, nil, nil, nil
		}
		lines, err := snapshotLines(ctx, unit, query, nil)
		if err != nil {
			return http.StatusInternalServerError, nil, []byte("journalctl failed: " + err.Error()), nil
		}
		body, _ := json.Marshal(buildStatus(unit, lines, query))
		return http.StatusOK, map[string]string{"Content-Type": "application/json"}, body, nil
	case "/api/logs":
		if method != http.MethodGet {
			return http.StatusMethodNotAllowed, nil, nil, nil
		}
		lines, err := snapshotLines(ctx, unit, query, nil)
		if err != nil {
			return http.StatusInternalServerError, nil, []byte("journalctl failed: " + err.Error()), nil
		}
		body, _ := json.Marshal(map[string]any{"lines": lines})
		return http.StatusOK, map[string]string{"Content-Type": "application/json"}, body, nil
	case "/api/logs/sse":
		return http.StatusMethodNotAllowed, nil, []byte("use services.stream for SSE"), nil
	default:
		if strings.HasPrefix(route, "/api/logs/") && method == http.MethodGet {
			lines, err := snapshotLines(ctx, unit, query, nil)
			if err != nil {
				return http.StatusInternalServerError, nil, []byte("journalctl failed: " + err.Error()), nil
			}
			for _, line := range lines {
				if line.Index == query.Index {
					body, _ := json.Marshal(map[string]any{"line": line})
					return http.StatusOK, map[string]string{"Content-Type": "application/json"}, body, nil
				}
			}
			return http.StatusNotFound, nil, []byte("line not found"), nil
		}
		if strings.HasPrefix(route, "/api/") {
			return http.StatusNotFound, nil, nil, nil
		}
		return http.StatusNotFound, nil, []byte("not found"), nil
	}
}

func writeSSEEvent(w io.Writer, event string, payload any) error {
	data, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	if _, err := fmt.Fprintf(w, "event: %s\n", event); err != nil {
		return err
	}
	if _, err := fmt.Fprintf(w, "data: %s\n\n", data); err != nil {
		return err
	}
	return nil
}

// OpenSSE returns a stream of SSE events for journalctl follow mode.
func OpenSSE(ctx context.Context, unit string, path string) (io.ReadCloser, error) {
	if !systemd.Available() {
		return nil, fmt.Errorf("journald unavailable")
	}
	query, route, err := ParseQuery(path)
	if err != nil {
		return nil, err
	}
	if route != "/api/logs/sse" {
		return nil, fmt.Errorf("unsupported journald stream path")
	}
	pr, pw := io.Pipe()
	go func() {
		err := writeSSE(ctx, pw, unit, query)
		pw.CloseWithError(err)
	}()
	return pr, nil
}

func writeSSE(ctx context.Context, w io.Writer, unit string, query Query) error {
	lines, err := snapshotLines(ctx, unit, query, nil)
	if err != nil {
		return err
	}
	status := buildStatus(unit, lines, query)
	if err := writeSSEEvent(w, "status", status); err != nil {
		return err
	}
	if fields := status.ObservedFields; len(fields) > 0 {
		if err := writeSSEEvent(w, "fields", fields); err != nil {
			return err
		}
	}
	for _, line := range lines {
		if err := writeSSEEvent(w, "line", line); err != nil {
			return err
		}
	}

	args := []string{"-f"}
	if priority := priorityArg(query.MinLevel); priority != "" {
		args = append(args, "-p", priority)
	}
	cmd := journalctlCommand(ctx, unit, args)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return err
	}
	defer func() {
		if cmd.Process != nil {
			_ = cmd.Process.Kill()
		}
		_ = cmd.Wait()
	}()

		buf := make([]byte, 0, 4096)
		chunk := make([]byte, 4096)
		index := len(lines)
		for {
			if ctx.Err() != nil {
				return ctx.Err()
			}
			n, readErr := stdout.Read(chunk)
			if n > 0 {
				buf = append(buf, chunk[:n]...)
				for {
					idx := bytes.IndexByte(buf, '\n')
					if idx < 0 {
						break
					}
					lineBytes := bytes.TrimSpace(buf[:idx])
					buf = buf[idx+1:]
					if len(lineBytes) == 0 {
						continue
					}
					var entry map[string]any
					if err := json.Unmarshal(lineBytes, &entry); err != nil {
						continue
					}
					mapped := mapEntry(unit, index, entry)
					if !matchesQuery(mapped, query) {
						continue
					}
					index++
					mapped.Index = index
					if err := writeSSEEvent(w, "line", mapped); err != nil {
						return err
					}
				}
			}
			if readErr != nil {
				if readErr == io.EOF {
					return nil
				}
				return readErr
			}
		}
}
