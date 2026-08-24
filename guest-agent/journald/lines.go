package journald

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"time"
)

const maxSnapshotBytes = 256 * 1024

// Line matches the iwatch log line envelope consumed by vzctl UI/CLI.
type Line struct {
	Index   int               `json:"index,omitempty"`
	Session int               `json:"session,omitempty"`
	Source  string            `json:"source,omitempty"`
	Text    string            `json:"text"`
	Fields  map[string]string `json:"fields,omitempty"`
	Ts      string            `json:"ts,omitempty"`
	Level   string            `json:"level,omitempty"`
}

// Status mirrors iwatch `/api/status` for toolbar filters.
type Status struct {
	Process        string   `json:"process,omitempty"`
	BufferLen      int      `json:"bufferLen,omitempty"`
	BufferCap      int      `json:"bufferCap,omitempty"`
	CommandTitle   string   `json:"commandTitle,omitempty"`
	LastURL        string   `json:"lastUrl,omitempty"`
	ObservedFields []string `json:"observedFields,omitempty"`
	GroupField     string   `json:"groupField,omitempty"`
	GroupValues    []string `json:"groupValues,omitempty"`
}

// Query decodes iwatch-compatible `/api/logs` query parameters.
type Query struct {
	Tail       int
	Q          string
	MinLevel   string
	GroupField string
	GroupValue string
	Filters    map[string]string
	Index      int
	Before     int
	Limit      int
}

func ParseQuery(rawPath string) (Query, string, error) {
	parsed, err := url.Parse(rawPath)
	if err != nil {
		return Query{}, "", fmt.Errorf("invalid path")
	}
	path := parsed.Path
	query := Query{Filters: map[string]string{}}
	for key, values := range parsed.Query() {
		if len(values) == 0 {
			continue
		}
		value := values[0]
		switch key {
		case "tail":
			if n, err := strconv.Atoi(value); err == nil && n > 0 {
				query.Tail = n
			}
		case "q":
			query.Q = value
		case "minLevel":
			query.MinLevel = value
		case "groupField":
			query.GroupField = value
		case "groupValue":
			query.GroupValue = value
		case "before":
			if n, err := strconv.Atoi(value); err == nil {
				query.Before = n
			}
		case "limit":
			if n, err := strconv.Atoi(value); err == nil && n > 0 {
				query.Limit = n
			}
		default:
			if strings.HasPrefix(key, "filter.") {
				filterKey := strings.TrimPrefix(key, "filter.")
				if filterKey != "" {
					query.Filters[filterKey] = value
				}
			}
		}
	}
	if path == "/api/status" {
		return query, path, nil
	}
	if path == "/api/logs" || path == "/api/logs/sse" {
		return query, path, nil
	}
	if strings.HasPrefix(path, "/api/logs/") {
		segment := strings.TrimPrefix(path, "/api/logs/")
		if segment == "" {
			return query, path, fmt.Errorf("invalid line index")
		}
		if strings.HasPrefix(segment, "sse") {
			return query, "/api/logs/sse", nil
		}
		index, err := strconv.Atoi(segment)
		if err != nil || index < 0 {
			return query, path, fmt.Errorf("invalid line index")
		}
		query.Index = index
		return query, "/api/logs/" + segment, nil
	}
	if strings.HasPrefix(path, "/api/share/") {
		return query, path, nil
	}
	return query, path, fmt.Errorf("unsupported journald path")
}

func priorityArg(minLevel string) string {
	switch strings.ToLower(strings.TrimSpace(minLevel)) {
	case "fatal":
		return "crit"
	case "error":
		return "err"
	case "warn":
		return "warning"
	case "info":
		return "info"
	case "trace", "debug":
		return "debug"
	default:
		return ""
	}
}

func levelFromPriority(raw string) string {
	n, err := strconv.Atoi(strings.TrimSpace(raw))
	if err != nil {
		return "info"
	}
	switch {
	case n <= 2:
		return "fatal"
	case n == 3:
		return "error"
	case n == 4:
		return "warn"
	case n <= 6:
		return "info"
	default:
		return "debug"
	}
}

func parseJournalJSON(unit string, payload []byte) ([]Line, error) {
	trimmed := strings.TrimSpace(string(payload))
	if trimmed == "" {
		return nil, nil
	}
	var lines []Line
	if strings.HasPrefix(trimmed, "[") {
		var entries []map[string]any
		if err := json.Unmarshal(payload, &entries); err != nil {
			return nil, err
		}
		for i, entry := range entries {
			lines = append(lines, mapEntry(unit, i, entry))
		}
		return lines, nil
	}
	for _, line := range strings.Split(trimmed, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var entry map[string]any
		if err := json.Unmarshal([]byte(line), &entry); err != nil {
			continue
		}
		lines = append(lines, mapEntry(unit, len(lines), entry))
	}
	return lines, nil
}

func mapEntry(unit string, index int, entry map[string]any) Line {
	fields := map[string]string{}
	for key, value := range entry {
		if key == "MESSAGE" {
			continue
		}
		fields[key] = stringifyField(value)
	}
	text := stringifyField(entry["MESSAGE"])
	if text == "" {
		text = stringifyField(entry["message"])
	}
	level := levelFromPriority(stringifyField(entry["PRIORITY"]))
	ts := formatTimestamp(stringifyField(entry["__REALTIME_TIMESTAMP"]))
	return Line{
		Index:  index,
		Source: unit,
		Text:   text,
		Fields: fields,
		Ts:     ts,
		Level:  level,
	}
}

func stringifyField(value any) string {
	switch typed := value.(type) {
	case nil:
		return ""
	case string:
		return typed
	case float64:
		return strconv.FormatInt(int64(typed), 10)
	case json.Number:
		return typed.String()
	default:
		return fmt.Sprint(typed)
	}
}

func formatTimestamp(raw string) string {
	if raw == "" {
		return ""
	}
	usec, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		return raw
	}
	sec := usec / 1_000_000
	nsec := (usec % 1_000_000) * 1_000
	return time.Unix(sec, nsec).UTC().Format(time.RFC3339Nano)
}

func applyQueryFilters(lines []Line, query Query) []Line {
	out := make([]Line, 0, len(lines))
	for _, line := range lines {
		if !matchesQuery(line, query) {
			continue
		}
		out = append(out, line)
	}
	for i := range out {
		out[i].Index = i
	}
	return out
}

func matchesQuery(line Line, query Query) bool {
	if query.GroupField != "" && query.GroupValue != "" {
		if line.Fields[query.GroupField] != query.GroupValue {
			return false
		}
	}
	for key, want := range query.Filters {
		if line.Fields[key] != want {
			return false
		}
	}
	if query.Q != "" && !textMatchesQ(line.Text, query.Q) {
		return false
	}
	if query.MinLevel != "" && query.MinLevel != "all" {
		if levelRank(line.Level) < levelRank(query.MinLevel) {
			return false
		}
	}
	return true
}

// textMatchesQ applies iwatch-style `q` filtering in-process (`|` = OR, case-insensitive).
func textMatchesQ(text, q string) bool {
	q = strings.TrimSpace(q)
	if q == "" {
		return true
	}
	lower := strings.ToLower(text)
	for _, part := range strings.Split(q, "|") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		if strings.Contains(lower, strings.ToLower(part)) {
			return true
		}
	}
	return false
}

func levelRank(level string) int {
	switch strings.ToLower(level) {
	case "fatal":
		return 6
	case "error":
		return 5
	case "warn":
		return 4
	case "info":
		return 3
	case "debug":
		return 2
	case "trace":
		return 1
	default:
		return 0
	}
}

func observedFields(lines []Line) []string {
	seen := map[string]struct{}{}
	for _, line := range lines {
		for key := range line.Fields {
			seen[key] = struct{}{}
		}
	}
	out := make([]string, 0, len(seen))
	for key := range seen {
		out = append(out, key)
	}
	sortStrings(out)
	return out
}

func groupValues(lines []Line, field string) []string {
	if field == "" {
		return nil
	}
	seen := map[string]struct{}{}
	for _, line := range lines {
		if value := line.Fields[field]; value != "" {
			seen[value] = struct{}{}
		}
	}
	out := make([]string, 0, len(seen))
	for value := range seen {
		out = append(out, value)
	}
	sortStrings(out)
	return out
}

func sortStrings(values []string) {
	for i := 0; i < len(values); i++ {
		for j := i + 1; j < len(values); j++ {
			if values[j] < values[i] {
				values[i], values[j] = values[j], values[i]
			}
		}
	}
}

func buildStatus(unit string, lines []Line, query Query) Status {
	groupField := query.GroupField
	if groupField == "" {
		groupField = "component"
	}
	return Status{
		Process:        unit,
		BufferLen:      len(lines),
		BufferCap:      len(lines),
		CommandTitle:   "journalctl -u " + unit,
		ObservedFields: observedFields(lines),
		GroupField:     groupField,
		GroupValues:    groupValues(lines, groupField),
	}
}
