package scripts

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

const rules = "table inet vzctl { chain forward { type filter hook forward priority 0; policy drop; } }"
const plan = `{"static_routes":[{"destination":"10.23.0.0/24","via":"10.22.0.2"}]}`

const fakeTools = `#!/usr/bin/env python3
import json,os,pathlib,sys
root=pathlib.Path(os.environ['VZ_ROUTER_FIXTURE_DIR']); state=root/'state'
name=pathlib.Path(sys.argv[0]).name; args=sys.argv[1:]
with (root/'calls').open('a') as f: f.write(json.dumps([name,*args])+'\n')
if name=='nft':
 if args==['list','table','inet','vzctl']: sys.exit(0 if (state/'vzctl').exists() else 1)
 if args==['-j','list','ruleset']:
  print((state/'inventory').read_text() if (state/'inventory').exists() else '{"nftables":[]}')
  sys.exit(0)
 if len(args)==2 and args[0]=='-f':
  value=sys.stdin.read() if args[1]=='-' else pathlib.Path(args[1]).read_text()
  if 'INVALID' in value or os.environ.get('FAIL_NFT_LOAD'): sys.exit(1)
  assert 'flush ruleset' not in value
  assert 'delete table inet foreign' not in value
  assert 'table inet vzctl {' in value
  (state/'vzctl').write_text(value)
  if 'delete chain ip filter FORWARD' in value:
   (state/'legacy-removed').touch()
   (state/'inventory').write_text('{"nftables":[]}')
  with (root/'loads').open('a') as f: f.write(json.dumps(value)+'\n')
 else: raise Exception('unexpected nft call')
elif name=='sysctl':
 if args==['-n','net.ipv4.ip_forward']: print((state/'forwarding').read_text())
 elif args in [['-q','-w','net.ipv4.ip_forward=1'],['-q','-w','net.ipv4.ip_forward=0']]:
  (state/'forwarding').write_text(args[-1][-1])
 else: raise Exception('unexpected sysctl call')
elif name=='systemctl':
 if args==['daemon-reload']: pass
 elif args==['is-enabled','vzctl-router.service']:
  print('enabled' if (state/'enabled').exists() else 'disabled')
  sys.exit(0 if (state/'enabled').exists() else 1)
 elif args==['enable','vzctl-router.service']:
  if os.environ.get('FAIL_ENABLE'): sys.exit(1)
  (state/'enabled').touch()
 else: raise Exception('unexpected service mutation: '+repr(args))
elif name=='rc-update':
 assert args==['add','vzctl-router','default']
 if os.environ.get('FAIL_ENABLE'): sys.exit(1)
 link=root/'etc/runlevels/default/vzctl-router'; link.parent.mkdir(parents=True,exist_ok=True)
 if not link.exists(): link.symlink_to(root/'etc/init.d/vzctl-router')
elif name=='ip':
 assert args==['route','replace','10.23.0.0/24','via','10.22.0.2']
 (state/'static-route').touch()
else: raise Exception('unexpected tool '+name)
`

type fixture struct {
	t            *testing.T
	root, script string
	extra        []string
}

func newFixture(t *testing.T, init string) *fixture {
	t.Helper()
	if _, err := exec.LookPath("python3"); err != nil {
		t.Fatal("script fixtures require python3:", err)
	}
	root := t.TempDir()
	for _, path := range []string{"etc/vzctl", "etc/sysctl.d", "etc/systemd/system", "etc/init.d", "state", "tools"} {
		if err := os.MkdirAll(filepath.Join(root, path), 0755); err != nil {
			t.Fatal(err)
		}
	}
	if init == "systemd" {
		if err := os.MkdirAll(filepath.Join(root, "run/systemd/system"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	if init == "openrc" {
		if err := os.MkdirAll(filepath.Join(root, "sbin"), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(root, "sbin/openrc-run"), []byte("#!/bin/sh\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
	content, err := os.ReadFile("router-apply")
	if err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(root, "router-apply")
	text := strings.NewReplacer("/etc/", root+"/etc/", "/run/", root+"/run/", "/var/lib/cloud/", root+"/var/lib/cloud/", "/sbin/openrc-run", root+"/sbin/openrc-run", "/usr/local/lib/vzctl/router-apply", script).Replace(string(content))
	if err := os.WriteFile(script, []byte(text), 0755); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"nft", "sysctl", "systemctl", "rc-update", "ip"} {
		if err := os.WriteFile(filepath.Join(root, "tools", name), []byte(fakeTools), 0755); err != nil {
			t.Fatal(err)
		}
	}
	f := &fixture{t: t, root: root, script: script}
	f.write("state/foreign", "foreign firewall must survive")
	f.write("state/forwarding", "0")
	f.write("etc/nftables.conf", "flush ruleset\n# foreign default configuration\n")
	return f
}
func (f *fixture) write(path, value string) {
	f.t.Helper()
	if err := os.WriteFile(filepath.Join(f.root, path), []byte(value), 0644); err != nil {
		f.t.Fatal(err)
	}
}
func (f *fixture) read(path string) string {
	f.t.Helper()
	v, err := os.ReadFile(filepath.Join(f.root, path))
	if err != nil {
		f.t.Fatal(err)
	}
	return string(v)
}
func (f *fixture) run(arg string, success bool) string {
	f.t.Helper()
	cmd := exec.Command("sh", f.script, arg)
	cmd.Env = append(os.Environ(), "VZ_ROUTER_FIXTURE_DIR="+f.root, "PATH="+filepath.Join(f.root, "tools")+":"+os.Getenv("PATH"))
	cmd.Env = append(cmd.Env, f.extra...)
	if arg != "--restore" {
		cmd.Stdin = strings.NewReader(plan)
	}
	out, err := cmd.CombinedOutput()
	if success && err != nil {
		f.t.Fatalf("apply failed: %v: %s", err, out)
	}
	if !success && err == nil {
		f.t.Fatalf("expected failure, got %s", out)
	}
	return string(out)
}
func (f *fixture) reboot() {
	f.t.Helper()
	for _, p := range []string{"state/vzctl", "state/static-route"} {
		if err := os.Remove(filepath.Join(f.root, p)); err != nil {
			f.t.Fatal(err)
		}
	}
	f.write("state/forwarding", "0")
}
func (f *fixture) assertForeignUnchanged() {
	f.t.Helper()
	if f.read("state/foreign") != "foreign firewall must survive" || f.read("etc/nftables.conf") != "flush ruleset\n# foreign default configuration\n" {
		f.t.Fatal("foreign nftables state changed")
	}
}

func (f *fixture) legacy(provenance bool, rule bool) {
	f.t.Helper()
	items := `[{"chain":{"family":"ip","table":"filter","name":"FORWARD","type":"filter","hook":"forward","prio":0,"policy":"drop"}}`
	if rule {
		items += `,{"rule":{"family":"ip","table":"filter","chain":"FORWARD","expr":[{"drop":null}]}}`
	}
	f.write("state/inventory", `{"nftables":`+items+`]}`)
	if provenance {
		dir := filepath.Join(f.root, "var/lib/cloud/instances/fixture/scripts")
		if err := os.MkdirAll(dir, 0755); err != nil {
			f.t.Fatal(err)
		}
		f.write("var/lib/cloud/instances/fixture/scripts/runcmd", "#!/bin/sh\n'sh' '-c' 'command -v iptables >/dev/null && iptables -P FORWARD DROP || true'\n")
		if err := os.Symlink("instances/fixture", filepath.Join(f.root, "var/lib/cloud/instance")); err != nil {
			f.t.Fatal(err)
		}
	}
}

func TestRouterLegacyBootstrapMigrationIsOneAtomicLoad(t *testing.T) {
	f := newFixture(t, "systemd")
	f.legacy(true, false)
	f.run(rules, true)
	load := f.read("state/vzctl")
	if !strings.Contains(load, "delete chain ip filter FORWARD\n") ||
		!strings.Contains(load, "table inet vzctl {") || strings.Count(f.read("loads"), "\n") != 1 {
		t.Fatal("legacy chain migration and owned rules must be in one nft transaction")
	}
	f.assertForeignUnchanged()
	f.run(rules, true)
	if strings.Count(f.read("loads"), "\n") != 1 {
		t.Fatal("migration is not idempotent")
	}
}

func TestRouterLegacyBootstrapRejectsUnownedOrNonemptyAndRollsBackInvalidLoad(t *testing.T) {
	for _, mode := range []string{"missing-origin", "foreign-rule", "modified-origin", "writable-origin", "writable-parent", "invalid-load"} {
		t.Run(mode, func(t *testing.T) {
			f := newFixture(t, "systemd")
			f.legacy(mode != "missing-origin", mode == "foreign-rule")
			if mode == "modified-origin" {
				f.write("var/lib/cloud/instances/fixture/scripts/runcmd", "#!/bin/sh\niptables -P FORWARD DROP\n")
			}
			if mode == "writable-origin" {
				if err := os.Chmod(filepath.Join(f.root, "var/lib/cloud/instances/fixture/scripts/runcmd"), 0666); err != nil {
					t.Fatal(err)
				}
			}
			if mode == "writable-parent" {
				if err := os.Chmod(filepath.Join(f.root, "var/lib/cloud/instances/fixture/scripts"), 0777); err != nil {
					t.Fatal(err)
				}
			}
			if mode == "invalid-load" {
				f.extra = []string{"FAIL_NFT_LOAD=1"}
				f.write("state/vzctl", "previous owned firewall")
				f.write("etc/vzctl/routes.json", "previous routes")
				f.write("etc/vzctl/vzctl.nft", "previous saved rules")
			}
			failure := f.run(rules, false)
			for _, path := range []string{"state/legacy-removed", "state/enabled"} {
				if _, err := os.Stat(filepath.Join(f.root, path)); !os.IsNotExist(err) {
					t.Fatal("failed migration changed state:", path)
				}
			}
			if mode == "invalid-load" {
				if !strings.Contains(failure, "boot_guards_installed=true") ||
					f.read("etc/sysctl.d/90-vzctl-router.conf") != "net.ipv4.ip_forward=0\n" {
					t.Fatal("failed nft transaction must report retained conservative boot protection")
				}
				if f.read("state/vzctl") != "previous owned firewall" {
					t.Fatal("failed nft transaction changed the existing firewall")
				}
				if f.read("etc/vzctl/routes.json") != "previous routes" || f.read("etc/vzctl/vzctl.nft") != "previous saved rules" {
					t.Fatal("failed nft transaction changed persisted rules")
				}
			} else if _, err := os.Stat(filepath.Join(f.root, "state/vzctl")); !os.IsNotExist(err) {
				t.Fatal("failed ownership gate changed the firewall")
			}
			if mode != "invalid-load" {
				if _, err := os.Stat(filepath.Join(f.root, "etc/vzctl/routes.json")); !os.IsNotExist(err) {
					t.Fatal("failed ownership gate wrote routes")
				}
			}
			f.assertForeignUnchanged()
		})
	}
}

func TestRouterBootstrapDenyOwnsOnlyVzctlTableAndPreservesExistingRules(t *testing.T) {
	f := newFixture(t, "systemd")
	f.run("--bootstrap", true)
	if !strings.Contains(f.read("state/vzctl"), "policy drop") ||
		strings.Contains(f.read("calls"), "iptables") {
		t.Fatal("bootstrap must use own deny table")
	}
	before := f.read("state/vzctl")
	calls := f.read("calls")
	if f.read("state/forwarding") != "1" ||
		strings.Index(calls, `"-f"`) > strings.Index(calls, `"net.ipv4.ip_forward=1"`) {
		t.Fatal("owned deny must precede forwarding, including Docker package autostart")
	}
	f.run("--bootstrap", true)
	if f.read("state/vzctl") != before || strings.Count(f.read("loads"), "\n") != 1 {
		t.Fatal("bootstrap changed existing owned table")
	}
	f.assertForeignUnchanged()
}

func TestRouterBootstrapInvalidDenyNeverEnablesForwarding(t *testing.T) {
	f := newFixture(t, "systemd")
	f.write("state/forwarding", "1")
	f.extra = []string{"FAIL_NFT_LOAD=1"}
	f.run("--bootstrap", false)
	if f.read("state/forwarding") != "0" {
		t.Fatal("failed bootstrap enabled forwarding")
	}
}

func TestExistingRouterBootGetsFailClosedDefaultAndPrivilegedServiceGuards(t *testing.T) {
	f := newFixture(t, "systemd")
	f.write("etc/sysctl.d/90-vzctl-router.conf", "net.ipv4.ip_forward=1\n")
	f.run(rules, true)
	if f.read("etc/sysctl.d/90-vzctl-router.conf") != "net.ipv4.ip_forward=0\n" {
		t.Fatal("old VM must get a fail-closed persistent boot default")
	}
	for _, service := range []string{"docker", "vzctl-agent"} {
		path := "etc/systemd/system/" + service + ".service.d/20-vzctl-router-bootstrap.conf"
		if !strings.Contains(f.read(path), "ExecStartPre=+"+f.script+" --bootstrap") ||
			!strings.Contains(f.read(path), "After=systemd-sysctl.service nftables.service") {
			t.Fatal("missing privileged runtime guard:", service)
		}
	}
	f.reboot()
	// Replay early sysctl=0, then Docker's privileged Prestart, then late restore.
	if f.read("state/forwarding") != "0" {
		t.Fatal("boot must not begin forwarding")
	}
	f.run("--bootstrap", true)
	if f.read("state/forwarding") != "1" || !strings.Contains(f.read("state/vzctl"), "policy drop") {
		t.Fatal("Docker prestart did not install deny before enabling forwarding")
	}
	f.run("--restore", true)
	if f.read("etc/sysctl.d/90-vzctl-router.conf") != "net.ipv4.ip_forward=0\n" ||
		f.read("state/forwarding") != "1" {
		t.Fatal("restore must preserve closed boot default while enabling current runtime")
	}
}

func TestRouterRefusesForeignServiceGuardBeforeFirewallMigration(t *testing.T) {
	for _, init := range []string{"systemd", "openrc"} {
		t.Run(init, func(t *testing.T) {
			f := newFixture(t, init)
			f.legacy(true, false)
			path := "etc/conf.d/docker"
			if init == "systemd" {
				path = "etc/systemd/system/docker.service.d/20-vzctl-router-bootstrap.conf"
			}
			if err := os.MkdirAll(filepath.Dir(filepath.Join(f.root, path)), 0755); err != nil {
				t.Fatal(err)
			}
			f.write(path, "foreign service configuration")
			f.run(rules, false)
			if f.read(path) != "foreign service configuration" {
				t.Fatal("foreign service configuration changed")
			}
			if _, err := os.Stat(filepath.Join(f.root, "state/vzctl")); !os.IsNotExist(err) {
				t.Fatal("foreign guard failure must precede firewall mutation")
			}
			if _, err := os.Stat(filepath.Join(f.root, "etc/sysctl.d/90-vzctl-router.conf")); !os.IsNotExist(err) {
				t.Fatal("foreign guard failure must precede persistent boot writes")
			}
		})
	}
}

func TestRouterBootRestoreSystemdKeepsForeignFirewallAndIsIdempotent(t *testing.T) {
	f := newFixture(t, "systemd")
	f.run(rules, true)
	unit := f.read("etc/systemd/system/vzctl-router.service")
	for _, fragment := range []string{"Type=oneshot", "After=network-online.target nftables.service", f.script + " --restore", "WantedBy=multi-user.target"} {
		if !strings.Contains(unit, fragment) {
			t.Fatalf("missing unit contract %q", fragment)
		}
	}
	if _, err := os.Stat(filepath.Join(f.root, "state/enabled")); err != nil {
		t.Fatal("boot service not enabled:", err)
	}
	if !strings.Contains(f.run(rules, true), "changed=false") {
		t.Fatal("repeated apply changed state")
	}
	before := f.read("calls")
	f.reboot()
	f.run("--restore", true)
	if !strings.Contains(f.read("state/vzctl"), "table inet vzctl {") || f.read("state/forwarding") != "1" {
		t.Fatal("router state not restored")
	}
	if _, err := os.Stat(filepath.Join(f.root, "state/static-route")); err != nil {
		t.Fatal("static route not restored:", err)
	}
	after := strings.TrimPrefix(f.read("calls"), before)
	if strings.Contains(after, "systemctl") {
		t.Fatal("boot restore must not recurse into service management")
	}
	f.run("--restore", true)
	if strings.Count(f.read("state/vzctl"), "delete table inet vzctl") != 1 ||
		strings.Count(f.read("state/vzctl"), "table inet vzctl {") != 1 {
		t.Fatal("repeated restore must replace only one owned table atomically")
	}
	f.assertForeignUnchanged()
}

func TestRouterBootRestoreOpenRC(t *testing.T) {
	f := newFixture(t, "openrc")
	f.run(rules, true)
	unit := f.read("etc/init.d/vzctl-router")
	for _, fragment := range []string{"openrc-run", "after nftables", f.script + " --restore"} {
		if !strings.Contains(unit, fragment) {
			t.Fatalf("missing OpenRC contract %q", fragment)
		}
	}
	if !strings.Contains(unit, "need net vzctl-router-bootstrap") ||
		!strings.Contains(f.read("etc/init.d/vzctl-router-bootstrap"), "after sysctl nftables") {
		t.Fatal("OpenRC restore must require the safe bootstrap")
	}
	for _, service := range []string{"docker", "vzctl-agent"} {
		config := f.read("etc/conf.d/" + service)
		if !strings.Contains(config, `rc_need="${rc_need:-} vzctl-router-bootstrap"`) {
			t.Fatal("OpenRC service must require a successful bootstrap:", service)
		}
	}
	if f.read("etc/sysctl.d/90-vzctl-router.conf") != "net.ipv4.ip_forward=0\n" {
		t.Fatal("OpenRC must persist the closed boot default")
	}
	if _, err := os.Lstat(filepath.Join(f.root, "etc/runlevels/default/vzctl-router")); err != nil {
		t.Fatal("OpenRC service not enabled:", err)
	}
	f.reboot()
	f.run("--bootstrap", true)
	f.run("--restore", true)
	f.assertForeignUnchanged()
}

func TestOpenRCRunlevelOverrideCannotBypassBootstrap(t *testing.T) {
	f := newFixture(t, "openrc")
	if err := os.MkdirAll(filepath.Join(f.root, "etc/conf.d"), 0755); err != nil {
		t.Fatal(err)
	}
	f.write("etc/conf.d/docker.default", `rc_need="foreign"`)
	f.run(rules, false)
	if _, err := os.Stat(filepath.Join(f.root, "state/vzctl")); !os.IsNotExist(err) {
		t.Fatal("ambiguous dependency override mutated firewall")
	}
}

func TestRouterPersistenceFailuresAreVisibleAndForeignUnitIsNotOverwritten(t *testing.T) {
	for _, mode := range []string{"foreign-unit", "symlink-unit", "enable-failure", "invalid-rules", "missing-state", "unsupported-init"} {
		t.Run(mode, func(t *testing.T) {
			init := "systemd"
			if mode == "unsupported-init" {
				init = "none"
			}
			f := newFixture(t, init)
			switch mode {
			case "foreign-unit":
				f.write("etc/systemd/system/vzctl-router.service", "foreign unit\n")
			case "symlink-unit":
				if err := os.Symlink(filepath.Join(f.root, "state/foreign"), filepath.Join(f.root, "etc/systemd/system/vzctl-router.service")); err != nil {
					t.Fatal(err)
				}
			case "enable-failure":
				f.extra = []string{"FAIL_ENABLE=1"}
			case "invalid-rules":
				f.run("INVALID", false)
				f.assertForeignUnchanged()
				return
			case "missing-state":
				f.run("--restore", false)
				f.assertForeignUnchanged()
				return
			}
			f.run(rules, false)
			f.assertForeignUnchanged()
			if mode != "enable-failure" {
				if _, err := os.Stat(filepath.Join(f.root, "state/vzctl")); !os.IsNotExist(err) {
					t.Fatal("init/unit preflight failure must happen before changing rules")
				}
			}
			if mode == "foreign-unit" && f.read("etc/systemd/system/vzctl-router.service") != "foreign unit\n" {
				t.Fatal("foreign unit overwritten")
			}
		})
	}
}

func TestRestoreRejectsSymlinkedSavedRules(t *testing.T) {
	f := newFixture(t, "systemd")
	f.run(rules, true)
	old := f.read("state/vzctl")
	saved := filepath.Join(f.root, "etc/vzctl/vzctl.nft")
	if err := os.Remove(saved); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(f.root, "state/foreign"), saved); err != nil {
		t.Fatal(err)
	}
	f.run("--restore", false)
	if f.read("state/vzctl") != old {
		t.Fatal("unsafe saved path changed active rules")
	}
	f.assertForeignUnchanged()
}

func TestRestoreRejectsInvalidSavedRulesWithoutDeletingExistingTables(t *testing.T) {
	f := newFixture(t, "systemd")
	f.run(rules, true)
	old := f.read("state/vzctl")
	f.write("etc/vzctl/vzctl.nft", "INVALID")
	f.run("--restore", false)
	if f.read("state/vzctl") != old {
		t.Fatal("failed nft transaction changed active rules")
	}
	f.assertForeignUnchanged()
	loads := strings.Split(strings.TrimSpace(f.read("loads")), "\n")
	var first string
	if err := json.Unmarshal([]byte(loads[0]), &first); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(first, "flush ruleset") {
		t.Fatal("global flush is forbidden")
	}
}
