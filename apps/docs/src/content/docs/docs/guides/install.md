---
title: Installation
description: vzctl und Supervisor lokal auf macOS 26+ installieren.
---

## Voraussetzungen

- macOS **26** oder neuer (Pre-26 ist unsupported)
- Apple Silicon
- Xcode Command Line Tools / Swift-Toolchain für Daemon-Builds aus dem Repo
- optional: Homebrew (`/opt/homebrew/bin` muss im LaunchAgent-PATH liegen)

## Release-Install

Aktuelle Builds: [Latest Release](https://github.com/frankhildebrandt/vzctl/releases/latest)
(immer der neueste Tag, z. B. `.pkg` / `.dmg` / `.tar.gz`).

Vom Repository-Root (aus dem Checkout bauen):

```bash
make install
export PATH="$HOME/.local/bin:$PATH"
vzctl doctor
```

`make install` legt Binaries unter `~/.local/bin` ab und aktiviert den LaunchAgent
`com.vzctl.supervisor.plist`. Laufende Agents werden vor dem Binary-Replace
gestoppt; `vz-net` nur graceful (kein SIGKILL), damit keine CIDR-Orphans bleiben.

Test ohne launchd-Aktivierung:

```bash
make install ACTIVATE=0
```

State-Verzeichnis (Default):

```text
~/Library/Application Support/vzctl/
```

Override: `VZCTL_STATE_DIR`.

## Vendor (Ingress / OIDC)

Für Caddy und Dex:

```bash
make vendor
make install-vendor
```

## UI (optional)

```bash
make ui-install
make ui-dev
```

## Doctor

`vzctl doctor` prüft macOS, Entitlements, APFS, Disk-Space, DNS und Supervisor-Health.
Warnungen bleiben Exit `0`; harte Fehler und Abhilfen stehen in der Repo-Datei
`docs/doctor.md`.
