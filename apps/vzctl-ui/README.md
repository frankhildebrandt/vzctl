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

- Open Environment (folder with `hypernetwork.config.yaml`)
- Diff / Up / **Apply** / Down (Apply mit Bestätigung + `--force`)
- DNS / OIDC / CA status bundle

See Epic [#47](https://github.com/frankhildebrandt/vzctl/issues/47).
