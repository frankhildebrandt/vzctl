# vzctl UI (Tauri 2)

Minimal desktop shell around the `vzctl` CLI. **No second reconciler**, no
direct Virtualization.framework access — every mutation is `vzctl … --format json`.

## Dev

```bash
# from repo root
cargo build -p vzctl
cd apps/vzctl-ui
npm install
npm run tauri:dev
```

Set `VZCTL_BIN` if `vzctl` is not on `PATH`.

## Views

- **Stacks / Projekte:** öffnen oder neu anlegen (`hypernetwork.config.yaml`)
- **Topologie-Editor** (AntV X6): Netze, VMs, Router, Attachments, Policies — speichert YAML + `.vzctl/diagram.json`
- **Betrieb:** Diff / Up / **Apply** / Down (Apply mit Bestätigung + `--force`)
- DNS / OIDC / CA status bundle
- **Doctor:** `vzctl doctor` inkl. Local-CA Keychain-Trust + Install-Button,
  DNS-Bind-Helper (`dns.bind_helper`) + Install mit Admin-Dialog
- VMs-Liste / Detail (Runtime)

Siehe [docs/topology-editor.md](docs/topology-editor.md) und Epic [#47](https://github.com/frankhildebrandt/vzctl/issues/47).

## Tests

```bash
npm test
npx playwright install chromium   # einmalig
npm run test:e2e
```
