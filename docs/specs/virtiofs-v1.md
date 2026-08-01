# virtiofs-v1

Host→Guest directory sharing for vzctl via Apple Virtualization
`VZVirtioFileSystemDevice` + guest bind-mounts.

## Model

- One VirtioFS device per VM, tag **`vzctl`** (reserved).
- Host directories are exposed as a `VZMultipleDirectoryShare` under that tag.
- Guest mounts the device at `/mnt/vzctl`, then bind-mounts
  `/mnt/vzctl/<name>` → `<target>`.
- Live updates use `VZVirtioFileSystemDevice.share` swap (no Helper restart)
  plus Guest-Agent `fs.mount` / `fs.unmount`.

## hypernetwork YAML

```yaml
spec:
  volumes:
    web-src: ../app          # relative to config dir, or absolute
  vms:
    web:
      mounts:
        - { source: web-src, target: /srv/app }
        - { source: web-src, target: /srv/ro, readOnly: true }
```

Rules:

- Volume names: 1–36 chars `[A-Za-z0-9][A-Za-z0-9_-]*`, not `vzctl`
- `source` must reference `spec.volumes`
- `target` absolute, unique per VM
- Host path must be an existing directory at validate time (when config path is known)

## CLI

```bash
vzctl vm create web --from ubuntu --data-disk 8 \
  --mount tag=web-src,source=/Users/me/app,target=/srv/app

# Live (running VM) or manifest-only (stopped):
vzctl vm mount web --source ~/app --target /srv/app [--tag web-src] [--ro]
vzctl vm unmount web --target /srv/app
vzctl vm mounts web
```

Running VM: Supervisor → Helper `mount.add` / `mount.remove` → share swap →
agent `fs.mount`. Stopped VM: only `vm.json` is updated; applied on next start.

## Guest helper

System NoCloud installs:

- `/usr/local/lib/vzctl/virtiofs-bind` (root)
- `/etc/sudoers.d/vzctl-virtiofs` for `vzctl-agent`

The Guest-Agent capability is `fs_mount`. The agent process itself stays without
`CAP_SYS_ADMIN`. `virtiofs-bind` re-enters PID 1's mount namespace (`nsenter`)
so binds remain visible to Docker and other host services when the agent runs
with `PrivateTmp=yes`.

## Coherence / edge cases

| Case | Behavior |
|------|----------|
| Host sleep / wake | VirtioFS usually recovers; remount via `fs.mount` if needed after time sync |
| Rename across mount boundary | Host rename into/out of a shared tree may surprise Linux guests; prefer copy+delete for critical moves |
| xattrs / ACL | Limited; do not rely on macOS↔Linux xattr parity |
| Large files | Prefer virtiofs over `vm transfer` (256 KiB agent cap) |
| Hotplug of *new* VZ devices | Not used — one device, live share dict updates |
| Empty mounts | Placeholder share `_empty` under `bundle/virtiofs-empty/` |

## Perf notes (vs Multipass)

Rough local methodology (document your numbers when measuring):

1. Sequential write/read: `dd if=/dev/zero of=/srv/app/bench.bin bs=1M count=512`
2. Small-file create: `for i in $(seq 1 2000); do echo x > /srv/app/f$i; done`
3. Compare the same workload on `multipass mount` into an Ubuntu instance

Expect virtiofs to beat SSHFS-style Multipass mounts on sequential IO and to be
closer (but still host-FS bound) on many small files. Numbers vary with host
disk, APFS pressure, and guest caching — treat this as a smoke comparison, not
a lab benchmark.

## Related

- [hypernetwork-v1.md](hypernetwork-v1.md)
- [guest-agent-v1.md](guest-agent-v1.md)
- Issue [#42](https://github.com/frankhildebrandt/vzctl/issues/42)
