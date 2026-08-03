---
title: Architektur
description: Prozessmodell von vz-net, vz-edge, vz-supervisor und vz-helper.
---

## Komponenten

| Prozess | Rolle |
| --- | --- |
| `vz-net` | HyperNetwork-Supervisor: vmnet-Refs, Host-Bridges, `net.sock` |
| `vz-edge` | Host-Dataplane: DNS, Ports, Ingress, Caddy/Dex/oidc-simple, `edge.sock` |
| `vz-supervisor` | Control Plane: Desired State, SQLite, Apply, REST (`api.sock`) |
| `vz-helper` | eine Instanz pro VM, owns `VZVirtualMachine` |

Startreihenfolge: **vz-net → vz-edge → vz-supervisor**.
Install-Stop umgekehrt: supervisor → edge → vz-net.

Helper bekommen nur Attachment-Handles via Serialize. Console-/exec-TTY-Attach
über Unix-Sockets unter `helpers/`.

## Entitlements

Nur `com.apple.security.virtualization`. Das Entitlement `com.apple.vm.networking`
bei ad-hoc codesign führt zu SIGKILL.

## UI

Die Tauri-UI spricht REST über `invoke` / `api_request` (nicht Browser-fetch).
Terminal bleibt UDS. Topology-Edits schreiben immer `hypernetwork.config.yaml`.

## Specs im Repo

- `docs/specs/supervisor-rest-v1.md`
- `docs/specs/vz-net-v1.md`
- `docs/specs/vz-edge-v1.md`
- `docs/adr/0002-process-ownership.md`
- `docs/adr/0003-apply-state.md`
