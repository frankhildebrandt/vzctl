# GitHub Tracking Map

Issues, Labels und Milestones für **frankhildebrandt/vzctl**.

- Issues: https://github.com/frankhildebrandt/vzctl/issues
- Milestones: https://github.com/frankhildebrandt/vzctl/milestones
- Labels: https://github.com/frankhildebrandt/vzctl/labels
- Machine map: [`06-github-issue-map.json`](06-github-issue-map.json)
- Bootstrap script: [`../../scripts/bootstrap-github-issues.py`](../../scripts/bootstrap-github-issues.py)

## Milestones

| Milestone | Fokus | Status |
|---|---|---|
| G0 — Spike Gate | Netz+DNS+Crash Go/No-Go | **closed** (Go) |
| P0 — Foundation | Ownership, Helper, Agent, Journal-ADR, doctor | **complete** (#20 closed) |
| P1 — CLI + Clones | JSON/Events, Seal/Clone/Identity | **core complete** |
| P2 — Net + DNS | vmnet, Dual-DNS, Policies | **complete** (#25 closed; #30 DoD ✅) |
| P3 — Stacks | hypernetwork reconcile | **complete** (#34 closed; #36/#37/#38/#52 ✅) |
| P4 — Docker + Ports | SSH Context, Ports basic | **complete** (#39 closed; #40/#41 ✅; #42 virtiofs → v0.1.x) |
| v0.1.x — Polish | virtiofs, Docker polish | **#42 virtiofs landed**; Docker polish residual |
| v0.2 — Ingress/OIDC/CA | Caddy, Dex, CA, Tauri | **complete** (#43 closed; #44–#47 ✅; polish #54) |

## Epics

| # | Epic | Milestone | Status |
|---|---|---|---|
| [#1](https://github.com/frankhildebrandt/vzctl/issues/1) | G0 Netzwerk-/DNS-/Crash-Spike | G0 | **closed** (Go) |
| [#7](https://github.com/frankhildebrandt/vzctl/issues/7) | Process-Modell & Ownership | P0 | **closed** |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) | vsock Guest-Agent | P0 | **closed** (Live Sleep/Base-Raw residual) |
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | CLI Contracts | P1 | contract complete; DX-Sofortgaps (#49 logs, adopt report, dns status) landed; Epic rest open |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | P1 | **complete** |
| [#25](https://github.com/frankhildebrandt/vzctl/issues/25) | Dual-DNS + macOS Resolver | P2 | **closed** (#26–#29 ✅; Live-Boot residual) |
| [#30](https://github.com/frankhildebrandt/vzctl/issues/30) | vmnet + Routes + Policies | P2 | **implementation complete** (#31/#32/#51/#33 ✅) |
| [#34](https://github.com/frankhildebrandt/vzctl/issues/34) | Stack Reconciler | P3 | **closed** (#36/#37/#38/#52 ✅; adopt report-only; reclaim deferred) |
| [#39](https://github.com/frankhildebrandt/vzctl/issues/39) | Docker Context + Ports | P4 | **closed** (#40/#41 ✅; #42 deferred v0.1.x) |
| [#43](https://github.com/frankhildebrandt/vzctl/issues/43) | v0.2 Ingress + CA + OIDC | v0.2 | **closed** (#44–#47 ✅; UI polish → #54) |
| [#48](https://github.com/frankhildebrandt/vzctl/issues/48) | DX Logs / Docs | P1 | Alpha `vm logs` (#49) ✅; v0.1.x Diagnose residual |

## Closed Gates / ADRs

| # | Titel | Notes |
|---|---|---|
| [#2](https://github.com/frankhildebrandt/vzctl/issues/2) | ADR macOS 26 | ADR 0001 |
| [#3](https://github.com/frankhildebrandt/vzctl/issues/3)–[#6](https://github.com/frankhildebrandt/vzctl/issues/6) | G0 Spikes | [g0-network.md](../spikes/g0-network.md) |
| [#8](https://github.com/frankhildebrandt/vzctl/issues/8) | ADR Ownership | ADR 0002 |
| [#35](https://github.com/frankhildebrandt/vzctl/issues/35) | ADR Apply/Journal | ADR 0003 |
| [#50](https://github.com/frankhildebrandt/vzctl/issues/50) | Docs Issues↔Plan | done |
| [#13](https://github.com/frankhildebrandt/vzctl/issues/13) | Guest-Agent Spec v1 | [guest-agent-v1.md](../specs/guest-agent-v1.md) |
| [#15](https://github.com/frankhildebrandt/vzctl/issues/15) | Helper↔Agent E2E | [p0-helper-agent-e2e.md](../spikes/p0-helper-agent-e2e.md) |
| [#16](https://github.com/frankhildebrandt/vzctl/issues/16) | Agent Time-Sync | [p0-agent-time-sync.md](../spikes/p0-agent-time-sync.md) |
| [#20](https://github.com/frankhildebrandt/vzctl/issues/20) | `vzctl doctor` | [doctor.md](../doctor.md) |
| [#18](https://github.com/frankhildebrandt/vzctl/issues/18) | CLI Contract v1 | [cli-contract-v1.md](../specs/cli-contract-v1.md) |
| [#19](https://github.com/frankhildebrandt/vzctl/issues/19) | Event Stream v1 | [events-v1.md](../specs/events-v1.md) |

## P0 Foundation (complete)

| # | Story | Scaffold |
|---|---|---|
| [#9](https://github.com/frankhildebrandt/vzctl/issues/9) | Supervisor UDS + SQLite | **closed** (`11f0eaa`) |
| [#10](https://github.com/frankhildebrandt/vzctl/issues/10) | Helper launchd + VZ | **closed** (`25584d7`) |
| [#11](https://github.com/frankhildebrandt/vzctl/issues/11) | Helper Reconnect | **closed** |
| [#14](https://github.com/frankhildebrandt/vzctl/issues/14) | Guest-Agent in Ubuntu Base | **closed** (`5713d46`) |
| [#15](https://github.com/frankhildebrandt/vzctl/issues/15) | Helper↔Agent E2E | **closed** (Live-Boot Residual) |
| [#16](https://github.com/frankhildebrandt/vzctl/issues/16) | Time-Sync nach Sleep | **closed** (Code/Unit ✅; Live Sleep residual) |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) | Epic Guest-Agent | **closed** (Live-Boot-Ops-Residual) |
| [#20](https://github.com/frankhildebrandt/vzctl/issues/20) | `vzctl doctor` | **closed** (Entitlement/APFS/Disk/DNS/JSON) |

Stories sind als **Sub-Issues** unter den Epics verknüpft.

Issue-Bodies sind **implementationsreif** formatiert und mit G0-Messungen (2026-07-30) aktualisiert.

## P1 Abschluss / P2 Next

| # | Story | Status |
|---|---|---|
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | Epic CLI Contracts | Contract-DoD complete; DX-Sofortgaps landed; surface rest |
| [#18](https://github.com/frankhildebrandt/vzctl/issues/18) | JSON + Exitcodes Spec | **complete** (CLI Contract v1 + Golden Tests) |
| [#19](https://github.com/frankhildebrandt/vzctl/issues/19) | Event-Schema + Subscribe | **closed** (UDS + NDJSON + Filter) |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | **complete** (#22/#23/#24 ✅) |
| [#22](https://github.com/frankhildebrandt/vzctl/issues/22) | `vzctl image seal` | **closed** (offline ✅; Builder-VM Backend in vzctl) |
| [#23](https://github.com/frankhildebrandt/vzctl/issues/23) | APFS linked clone + Root-Disk | **closed** (APFS COW, geteilte Base-Blöcke, sparse Root-Kapazität) |
| [#24](https://github.com/frankhildebrandt/vzctl/issues/24) | Identity-Reset | **complete** (NoCloud + Helper-MAC) |
| [#31](https://github.com/frankhildebrandt/vzctl/issues/31) | vmnet Network CRUD + Attachments | **closed** (SQLite rebuild + refs + metadata) |
| [#32](https://github.com/frankhildebrandt/vzctl/issues/32) | Router-Template + Routes | **complete** (Helper→Guest-Agent, idempotent, default DROP) |
| [#51](https://github.com/frankhildebrandt/vzctl/issues/51) | Default-Netzwerk (voller Zugriff) | **complete** (SQLite + Auto-IP + Create/Override) |
| [#33](https://github.com/frankhildebrandt/vzctl/issues/33) | Firewall-Policies | **complete** (nftables, plan/status JSON, edge-dmz) |
| [#26](https://github.com/frankhildebrandt/vzctl/issues/26) | Autoritative DNS-Zone + Forwarder | **complete** (Dual-UDP, TTL, reload, health/events) |
| [#27](https://github.com/frankhildebrandt/vzctl/issues/27) | macOS `/etc/resolver` install/cleanup | **complete** (atomic, scoped, idempotent, collision-safe) |
| [#28](https://github.com/frankhildebrandt/vzctl/issues/28) | `vzctl dns query` | **complete** (direktes UDP, A/AAAA, CLI-v1, Exit 20) |
| [#29](https://github.com/frankhildebrandt/vzctl/issues/29) | Guest-Nameserver `.0` | **closed** (NoCloud `.0` + Search-Domain; Live-Boot residual) |
| [#36](https://github.com/frankhildebrandt/vzctl/issues/36) | hypernetwork/v1 Schema + serde | **complete** (Schema export, CLI Validate, Fixtures, JSON-Pfade) |
| [#52](https://github.com/frankhildebrandt/vzctl/issues/52) | Image-Pull `*-latest` Bases | **complete** (14 Aliase, Digest/Raw-Store, Seal/`--from`) |
| [#37](https://github.com/frankhildebrandt/vzctl/issues/37) | Reconcile-/Apply-Engine | **complete** (plan/diff/up/down/apply, Lease, Journal Resume/Abort, Events) |
| [#38](https://github.com/frankhildebrandt/vzctl/issues/38) | examples/edge-dmz + CI validate/diff | **complete** (README + cloud-init + Supervisor CI validate/plan/diff) |
| [#49](https://github.com/frankhildebrandt/vzctl/issues/49) | `vzctl vm logs` | **complete** (serial + follow + redaction; Agent deferred) |
| [#40](https://github.com/frankhildebrandt/vzctl/issues/40) | Docker VM role + SSH context + `vzctl docker` | **complete** (cloudInit merge, DNS svc, purge/doctor) |
| [#41](https://github.com/frankhildebrandt/vzctl/issues/41) | Port-Forwards + Collision-Check | **complete** (Supervisor userspace proxy, `port list`) |
| [#39](https://github.com/frankhildebrandt/vzctl/issues/39) | Epic Docker Context + Ports | **closed** (Alpha DoD; virtiofs #42 out) |

## v0.2 Ingress / CA / OIDC / Tauri

| # | Story | Status |
|---|---|---|
| [#43](https://github.com/frankhildebrandt/vzctl/issues/43) | Epic v0.2 | **closed** (PR #53) |
| [#44](https://github.com/frankhildebrandt/vzctl/issues/44) | Caddy Ingress | **closed** |
| [#45](https://github.com/frankhildebrandt/vzctl/issues/45) | Local CA + Guest rollout | **closed** |
| [#46](https://github.com/frankhildebrandt/vzctl/issues/46) | Dex OIDC + clients:auto | **closed** |
| [#47](https://github.com/frankhildebrandt/vzctl/issues/47) | Tauri UI Alpha | **closed** (CLI wrapper + events) |
| [#54](https://github.com/frankhildebrandt/vzctl/issues/54) | Tauri UI polish (topology/purge/deeplink/land WT) | **open** |

## Label-Schema

- `type:` epic | story | spike | adr | chore
- `priority:` p0 | p1 | p2
- `area:` supervisor | helper | agent | dns | network | …
- `phase:` g0 | p0 | … | v02
- `finding:` fable | sol | plan
