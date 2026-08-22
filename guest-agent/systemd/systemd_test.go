package systemd

import (
	"strings"
	"testing"
	"time"
)

func TestValidateUnitName(t *testing.T) {
	if err := ValidateUnitName("nginx.service"); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"", "foo;bar", "../escape", "a\x00b"} {
		if err := ValidateUnitName(bad); err == nil {
			t.Fatalf("expected error for %q", bad)
		}
	}
}

func TestValidateUnitType(t *testing.T) {
	for _, tc := range []struct {
		in   string
		want string
	}{
		{"", "service"},
		{"service", "service"},
		{"timer", "timer"},
		{"socket", "socket"},
	} {
		got, err := ValidateUnitType(tc.in)
		if err != nil || got != tc.want {
			t.Fatalf("ValidateUnitType(%q) = (%q, %v)", tc.in, got, err)
		}
	}
	if _, err := ValidateUnitType("mount"); err == nil {
		t.Fatal("expected error")
	}
}

func TestParseListUnits(t *testing.T) {
	raw := `[
	  {"unit":"nginx.service","load":"loaded","active":"active","sub":"running","description":"nginx"},
	  {"unit":"foo.timer","load":"loaded","active":"inactive","sub":"dead","description":"foo"}
	]`
	units, err := parseListUnits(raw, "service")
	if err != nil {
		t.Fatal(err)
	}
	if len(units) != 2 || units[0].Name != "nginx.service" || units[1].Sub != "dead" {
		t.Fatalf("units=%#v", units)
	}
}

func TestParseShow(t *testing.T) {
	raw := `[{"Id":"nginx.service","LoadState":"loaded","ActiveState":"active","SubState":"running","Description":"nginx","UnitFileState":"enabled","FragmentPath":"/lib/systemd/system/nginx.service"}]`
	props, err := parseShow(raw, "nginx.service")
	if err != nil {
		t.Fatal(err)
	}
	if props["name"] != "nginx.service" || props["type"] != "service" || props["active"] != "active" {
		t.Fatalf("props=%#v", props)
	}
}

func TestParseShowKeyValueOutput(t *testing.T) {
	raw := `Id=cron.service
Description=Regular background program processing daemon
LoadState=loaded
ActiveState=active
SubState=running
FragmentPath=/usr/lib/systemd/system/cron.service
UnitFileState=enabled`
	props, err := parseShow(raw, "cron.service")
	if err != nil {
		t.Fatal(err)
	}
	if props["name"] != "cron.service" || props["load"] != "loaded" || props["unit_file"] != "enabled" {
		t.Fatalf("props=%#v", props)
	}
	if props["fragment"] != "/usr/lib/systemd/system/cron.service" {
		t.Fatalf("fragment=%q", props["fragment"])
	}
}

func TestEventBufferSince(t *testing.T) {
	buf := newEventBuffer(4)
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	for i, unit := range []string{"a.service", "b.service", "c.service"} {
		buf.append(newEvent(unit, "service", "loaded", "active", "running", "properties_changed", now.Add(time.Duration(i)*time.Second)))
	}
	events, cursor := buf.since("", 2)
	if len(events) != 2 || events[0].Unit != "b.service" || cursor == "" {
		t.Fatalf("events=%#v cursor=%q", events, cursor)
	}
	events, _ = buf.since(events[0].ID, 10)
	if len(events) != 1 || events[0].Unit != "c.service" {
		t.Fatalf("tail=%#v", events)
	}
}

func TestSystemctlEnvPreservesPath(t *testing.T) {
	env := systemctlEnv()
	foundLC := false
	foundPath := false
	for _, entry := range env {
		if entry == "LC_ALL=C" {
			foundLC = true
		}
		if strings.HasPrefix(entry, "PATH=") && strings.Contains(entry, "/usr/bin") {
			foundPath = true
		}
	}
	if !foundLC {
		t.Fatal("expected LC_ALL=C in systemctl env")
	}
	if !foundPath {
		t.Fatalf("expected PATH with /usr/bin in systemctl env: %#v", env)
	}
}

func TestDefaultExecTimeoutIsSeconds(t *testing.T) {
	if defaultExecTimeout < time.Second {
		t.Fatalf("defaultExecTimeout = %v, want >= 1s", defaultExecTimeout)
	}
}

func TestParseSystemctlVersion(t *testing.T) {
	got := parseSystemctlVersion("systemd 255 (255.4-1ubuntu8)\n+PAM ...")
	if got != "255" {
		t.Fatalf("version=%q", got)
	}
}

func TestAppendWatchedUnitEvent(t *testing.T) {
	manager := NewManager()
	at := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	appendWatchedUnitEvent(manager, "nginx.service", "loaded", "active", "running", at)
	appendWatchedUnitEvent(manager, "", "loaded", "active", "running", at)
	events, _ := manager.events.since("", 10)
	if len(events) != 1 || events[0].Unit != "nginx.service" || events[0].Active != "active" {
		t.Fatalf("events=%#v", events)
	}
}
