# Local CA v1

Status: v0.2<br>
Issues: [#45](https://github.com/frankhildebrandt/vzctl/issues/45),
[#43](https://github.com/frankhildebrandt/vzctl/issues/43)

## Scope

Host-local Certificate Authority for Ingress TLS and OIDC trust. Guests receive
the CA into their system trust store so `curl` / apps work without `-k`.

## Storage (Host)

```text
~/Library/Application Support/vzctl/ca/
  root/
    ca.crt
    ca.key          # mode 0600
    fingerprint     # sha256 hex of ca.crt DER
  issued/
    {san}/
      cert.pem
      key.pem       # mode 0600
      meta.json
  trust/
    vzctl-local.crt # copy of ca.crt for rollout
```

One global CA per host in v0.2 (not per project). Never commit these files.

## CLI

```bash
vzctl certs ca init
vzctl certs ca install          # optional macOS Keychain trust
vzctl certs mint <san> [--san alias...]
vzctl certs fingerprint
vzctl certs rollout [--vm NAME]
vzctl certs verify --vm NAME --url https://auth.svc.…
```

## Guest rollout

1. **Boot seed (NoCloud):** write
   `/usr/local/share/ca-certificates/vzctl-local.crt` and run
   `update-ca-certificates`.
2. **Live:** Guest-Agent method `ca_inject` (capability `ca_inject`).
3. Lockfile / SQLite stores the CA fingerprint; drift → reinject
   (`certs.onRotate: reinject`) or reboot (`reboot`).

Java trust stores are out of Alpha / Nice-to-have.

## Rotate

1. `vzctl certs ca init --force` (or rotate helper) writes a new root.
2. Re-mint leafs for all ingress hosts.
3. Reload Caddy with new PEMs.
4. Reinject CA into guests (or reboot per `onRotate`).
