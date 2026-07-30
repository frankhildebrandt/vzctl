# vzctl daemon/helper (P0)

`vz-supervisor` owns durable state and accepts newline-framed JSON-RPC over its
user-only UDS. Er broadcastet den versionierten
[Event Stream v1](../docs/specs/events-v1.md) an `events.subscribe`-Clients.
`vz-helper` owns exactly one `VZVirtualMachine` per process, per
[ADR 0002](../docs/adr/0002-process-ownership.md).

## Build and sign

```sh
swift build --package-path daemon
daemon/scripts/codesign-helper.sh daemon/.build/debug/vz-helper
```

The development signature contains `com.apple.security.virtualization` and
intentionally does **not** contain `com.apple.vm.networking`.

## VM bundle and run

The boot disk must be a writable raw Ubuntu arm64 disk image (not QCOW2):

```text
vm.bundle/
├── disk.raw       # required unless --disk is supplied
├── cidata.iso     # optional cloud-init seed
└── nvram.bin      # generated on first run
```

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
then cleanly stops B with `SIGTERM`. NAT is local to each VM in this slice;
supervisor-owned vmnet attachments remain follow-up work.
