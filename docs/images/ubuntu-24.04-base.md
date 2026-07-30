# Ubuntu 24.04 ARM64 guest-agent base

Issue #14 bakes `vzctl-agent` into the Ubuntu base before any clone is sealed.
First boot only supplies VM identity, SSH access, the private agent token and
network configuration. It never downloads or installs the agent.

## Build

Run the offline customization on an ARM64 Linux builder with Go 1.22+,
`libguestfs-tools`, `qemu-utils`, `curl` and `sha256sum`:

```bash
sudo apt-get install libguestfs-tools qemu-utils curl
./scripts/build-guest-agent-base.sh
```

The script:

1. downloads the Ubuntu 24.04 ARM64 cloud image;
2. verifies it against Ubuntu's `SHA256SUMS`;
3. cross-builds a static ARM64 `vzctl-agent`;
4. creates the unprivileged `vzctl-agent` account;
5. installs and enables the hardened systemd unit;
6. writes `/usr/lib/vzctl-agent/image-metadata.json`;
7. runs `cloud-init clean` and clears machine ID, SSH host keys and random seed;
8. emits `artifacts/ubuntu-24.04-vzctl-base.raw`.

For a byte-pinned input, pass the reviewed image digest:

```bash
UBUNTU_IMAGE_SHA256=<64-hex-digest> ./scripts/build-guest-agent-base.sh
```

Build outputs and downloaded images live below `artifacts/` and are ignored by
Git.

## Seal-ready checks

```bash
./scripts/smoke-guest-agent-base.sh \
  artifacts/ubuntu-24.04-vzctl-base.raw
```

Before `vzctl image seal`, verify:

- `/usr/local/sbin/vzctl-agent` is a static ARM64 Linux binary;
- `vzctl-agent.service` is enabled and runs as `vzctl-agent`;
- the service bounding/ambient capability set contains only `CAP_SYS_TIME`;
- `/usr/lib/vzctl-agent/image-metadata.json` records agent version, protocol
  `1` and vsock port `21950`;
- `/etc/machine-id` is empty; dbus machine ID, SSH host keys and cloud-init
  instance state are absent;
- `/run/vzctl/agent.token` is absent from the base.

The implemented `image seal` command preserves the binary, enabled unit and
metadata while cleaning clone identity and making the base read-only. See the
[Seal Contract v1](seal-contract-v1.md) and
[P1 spike](../spikes/p1-image-seal.md).

## Clone seed and boot proof

Render a fresh seed from
[`guest-agent/image/cloud-init`](../../guest-agent/image/cloud-init). Replace
all placeholders and generate the token, for example:

```bash
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n'
```

The seed may contain only:

- hostname/instance identity;
- SSH public keys;
- `/run/vzctl/agent.token` with owner `vzctl-agent:vzctl-agent`, mode `0600`;
- network config.

After booting a fresh clone with a virtio-vsock device:

```bash
systemctl is-active vzctl-agent
systemctl show -p User -p MainPID vzctl-agent
ss --vsock --listening --numeric | grep ':21950'
cat /usr/lib/vzctl-agent/image-metadata.json
```

Expected: `active`, user `vzctl-agent`, a non-zero PID, and a listener on
port `21950`.

The full host↔guest proof boots an APFS clone, creates a per-VM NoCloud seed
and exercises the protocol without SSH:

```bash
./scripts/smoke-helper-agent-e2e.sh \
  artifacts/ubuntu-24.04-vzctl-base.raw
```

See [`p0-helper-agent-e2e.md`](../spikes/p0-helper-agent-e2e.md) for the
required builder hand-off and current verification status.
