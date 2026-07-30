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
| P2 — Net + DNS | vmnet, Dual-DNS, Policies | in progress (#31 complete) |
| P3 — Stacks | hypernetwork reconcile | open |
| P4 — Docker + Ports | SSH Context, Ports basic | open |
| v0.1.x — Polish | virtiofs, Docker polish | open |
| v0.2 — Ingress/OIDC/CA | Caddy, Dex, CA, Tauri | open |

## Epics

| # | Epic | Milestone | Status |
|---|---|---|---|
| [#1](https://github.com/frankhildebrandt/vzctl/issues/1) | G0 Netzwerk-/DNS-/Crash-Spike | G0 | **closed** (Go) |
| [#7](https://github.com/frankhildebrandt/vzctl/issues/7) | Process-Modell & Ownership | P0 | **closed** |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) | vsock Guest-Agent | P0 | **closed** (Live Sleep/Base-Raw residual) |
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | CLI Contracts | P1 | contract complete; CLI surface rest open |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | P1 | **complete** |
| [#25](https://github.com/frankhildebrandt/vzctl/issues/25) | Dual-DNS + macOS Resolver | P2 | open (G0 Bind ✅) |
| [#30](https://github.com/frankhildebrandt/vzctl/issues/30) | vmnet + Routes + Policies | P2 | in progress (#31 ✅) |
| [#34](https://github.com/frankhildebrandt/vzctl/issues/34) | Stack Reconciler | P3 | open (ADR 0003 ✅) |
| [#39](https://github.com/frankhildebrandt/vzctl/issues/39) | Docker Context + Ports | P4 | open |
| [#43](https://github.com/frankhildebrandt/vzctl/issues/43) | v0.2 Ingress + CA + OIDC | v0.2 | open |
| [#48](https://github.com/frankhildebrandt/vzctl/issues/48) | DX Logs / Docs | P1 | open |

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
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | Epic CLI Contracts | Contract-DoD complete; CLI surface rest |
| [#18](https://github.com/frankhildebrandt/vzctl/issues/18) | JSON + Exitcodes Spec | **complete** (CLI Contract v1 + Golden Tests) |
| [#19](https://github.com/frankhildebrandt/vzctl/issues/19) | Event-Schema + Subscribe | **closed** (UDS + NDJSON + Filter) |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | **complete** (#22/#23/#24 ✅) |
| [#22](https://github.com/frankhildebrandt/vzctl/issues/22) | `vzctl image seal` | **closed** (offline ✅; Builder Residual) |
| [#23](https://github.com/frankhildebrandt/vzctl/issues/23) | APFS linked clone + `dataDisk` | **closed** (APFS COW + fallback + Helper attach) |
| [#24](https://github.com/frankhildebrandt/vzctl/issues/24) | Identity-Reset | **complete** (NoCloud + Helper-MAC) |
| [#31](https://github.com/frankhildebrandt/vzctl/issues/31) | vmnet Network CRUD + Attachments | **closed** (SQLite rebuild + refs + metadata) |
| [#32](https://github.com/frankhildebrandt/vzctl/issues/32) | Router-Template + Routes | ← **next network slice** |

## Label-Schema

- `type:` epic | story | spike | adr | chore
- `priority:` p0 | p1 | p2
- `area:` supervisor | helper | agent | dns | network | …
- `phase:` g0 | p0 | … | v02
- `finding:` fable | sol | plan
