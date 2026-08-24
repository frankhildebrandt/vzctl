package journald

import "testing"

func TestValidUnitRequiresTypeSuffix(t *testing.T) {
	if !ValidUnit("nginx.service") {
		t.Fatal("expected nginx.service valid")
	}
	if ValidUnit("app") {
		t.Fatal("short iwatch-style names must not match journald units")
	}
}

func TestParseQueryMapsFilters(t *testing.T) {
	query, path, err := ParseQuery("/api/logs?tail=100&q=err&minLevel=warn&filter.component=api")
	if err != nil || path != "/api/logs" {
		t.Fatalf("parse: path=%s err=%v", path, err)
	}
	if query.Tail != 100 || query.Q != "err" || query.MinLevel != "warn" {
		t.Fatalf("query = %#v", query)
	}
	if query.Filters["component"] != "api" {
		t.Fatalf("filters = %#v", query.Filters)
	}
}

func TestMatchesQueryMinLevel(t *testing.T) {
	line := Line{Text: "hello", Level: "info"}
	filtered := applyQueryFilters([]Line{line}, Query{MinLevel: "warn"})
	if len(filtered) != 0 {
		t.Fatalf("info line should be below warn minLevel: %#v", filtered)
	}
}

func TestTextMatchesQPipeAlternation(t *testing.T) {
	if !textMatchesQ("heartbeat ok", "wait|error|heartbeat") {
		t.Fatal("expected heartbeat match")
	}
	if textMatchesQ("all good", "wait|error|heartbeat") {
		t.Fatal("expected no match")
	}
	if !textMatchesQ("ERROR: disk", "wait|error|heartbeat") {
		t.Fatal("expected error match")
	}
}
