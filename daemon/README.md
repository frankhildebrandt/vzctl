# vzctl daemon/helper (P0)

`vz-supervisor` owns durable state and accepts newline-framed JSON-RPC over its
user-only UDS. Er broadcastet den versionierten
[Event Stream v1](../docs/specs/events-v1.md) an `events.subscribe`-Clients.
`vz-helper` owns exactly one `VZVirtualMachine` per process, per
[ADR 0002](../docs/adr/0002-process-ownership.md).

Der Supervisor betreibt außerdem den autoritativen
[Dual-DNS](../docs/dns.md): Host `127.0.0.1:15353`, Guest-Bridge `.0:53`,
Actual-State-A-Records und UDP-Forwarding. Für lokale unprivilegierte Läufe:

```sh
VZCTL_DNS_GUEST_PORT=15353 \
VZCTL_DNS_UPSTREAM=system \
daemon/.build/debug/vz-supervisor serve
```

## Build and sign

```sh
swift build --package-path daemon
daemon/scripts/codesign-helper.sh daemon/.build/debug/vz-helper
```

The development signature contains `com.apple.security.virtualization` and
intentionally does **not** contain `com.apple.vm.networking`.

Für eine Release-Installation inklusive aktivem Supervisor:

```sh
make install
launchctl print "gui/$(id -u)/com.vzctl.supervisor"
```

CLI, Supervisor und Helper landen standardmäßig in `~/.local/bin`. Das Ziel
kann mit `PREFIX` oder `BINDIR` überschrieben werden. `ACTIVATE=0` installiert
und validiert den LaunchAgent, ohne ihn zu laden. Wiederholtes `make install`
ersetzt die Binaries atomar und startet `com.vzctl.supervisor` neu. Laufende
VM-Helper bleiben unangetastet, damit keine VM für ein Tool-Update stoppt.

## VM bundle and run

The boot disk must be a writable raw Ubuntu arm64 disk image (not QCOW2):

```text
vm.bundle/
├── disk.raw       # required unless --disk is supplied
├── dataDisk.raw   # optional writable per-VM data disk
├── cidata.iso     # optional cloud-init seed
├── agent.token    # optional guest-agent token, mode 0600
├── vm.json        # optional vzctl manifest incl. persisted NIC MAC
└── nvram.bin      # generated on first run
```

For `vzctl vm create` bundles, the helper reads `identity.nics[0].mac` from
`vm.json` and applies it to a fresh `VZVirtioNetworkDeviceConfiguration`.
`--mac-address` overrides the manifest for manual bundles.

Start the supervisor and helper:

```sh
daemon/.build/debug/vz-supervisor serve
daemon/.build/debug/vz-helper run --vm-id demo/web --bundle /path/to/vm.bundle
```

Serial output is appended to `~/Library/Logs/vzctl/<safe-vm-id>.serial.log`.
`SIGTERM` requests a clean VM stop before the helper exits. A UDS disconnect
only logs once per error and the five-second heartbeat keeps retrying.

The per-VM lock is an advisory `flock` under
`~/Library/Application Support/vzctl/helpers/` (or `VZCTL_STATE_DIR/helpers`).
The file records the current PID. A second live helper fails immediately.
After a crash, the kernel releases the lock; the next helper adopts the stale
file and replaces its PID. The helper never kills an unknown PID. A future
supervisor reconciler may choose an explicit kill policy.

Large VM disks stay outside Git; `*.raw`, `*.img`, `*.qcow2`, `*.iso` and
`nvram.bin` are ignored for bundle directories.

## launchd

`launchd/com.vzctl.helper.plist.template` documents the job shape.
Generate an escaped per-VM plist with:

```sh
daemon/.build/debug/vz-helper launchd-plist \
  --vm-id demo-web --bundle /path/to/vm.bundle \
  --executable "$PWD/daemon/.build/debug/vz-helper" > /tmp/demo-web.plist
```

`KeepAlive.SuccessfulExit=false` restarts crashes but not clean shutdowns;
`ThrottleInterval=10` bounds restart loops. Helper stdout/stderr go to
`~/Library/Logs/vzctl/`.

Development smokes:

```sh
daemon/scripts/smoke-helper-isolation.sh
daemon/scripts/smoke-helper-rpc.sh
daemon/scripts/smoke-helper-launchd.sh
daemon/scripts/smoke-helper-two-vms.sh /path/to/source.raw /path/to/cidata.iso
```

The quick isolation smoke uses two mock helpers only to test process/lock
isolation. The two-VM smoke clones the supplied G0 raw Ubuntu disk into two
temporary bundle directories, verifies serial output, kills A with `SIGKILL`,
then cleanly stops B with `SIGTERM`.

Supervisor-owned vmnet CRUD and desired attachments are implemented in #31;
see [`docs/network.md`](../docs/network.md). Router and policy plans from
#32/#33 are rendered as nftables and sent Supervisor → per-VM helper →
Guest-Agent over vsock; see [`docs/routes.md`](../docs/routes.md). The existing standalone helper command
still uses NAT when started directly. Applying a desired attachment remains
part of the supervisor-driven helper start path; the Helper receives only a
serialized vmnet attachment handle and never owns the registry ref.
