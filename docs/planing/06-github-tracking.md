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
| P0 — Foundation | Ownership, Helper, Agent, Journal-ADR, doctor | **active** |
| P1 — CLI + Clones | JSON/Events, Seal/Clone/Identity | open |
| P2 — Net + DNS | vmnet, Dual-DNS, Policies | open |
| P3 — Stacks | hypernetwork reconcile | open |
| P4 — Docker + Ports | SSH Context, Ports basic | open |
| v0.1.x — Polish | virtiofs, Docker polish | open |
| v0.2 — Ingress/OIDC/CA | Caddy, Dex, CA, Tauri | open |

## Epics

| # | Epic | Milestone | Status |
|---|---|---|---|
| [#1](https://github.com/frankhildebrandt/vzctl/issues/1) | G0 Netzwerk-/DNS-/Crash-Spike | G0 | **closed** (Go) |
| [#7](https://github.com/frankhildebrandt/vzctl/issues/7) | Process-Modell & Ownership | P0 | **closed** |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) | vsock Guest-Agent | P0 | open |
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | CLI Contracts | P1 | open |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | P1 | open |
| [#25](https://github.com/frankhildebrandt/vzctl/issues/25) | Dual-DNS + macOS Resolver | P2 | open (G0 Bind ✅) |
| [#30](https://github.com/frankhildebrandt/vzctl/issues/30) | vmnet + Routes + Policies | P2 | open (G0 Reach ✅) |
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

## P0 Next (active)

| # | Story | Scaffold |
|---|---|---|
| [#9](https://github.com/frankhildebrandt/vzctl/issues/9) | Supervisor UDS + SQLite | **closed** (`11f0eaa`) |
| [#10](https://github.com/frankhildebrandt/vzctl/issues/10) | Helper launchd + VZ | **closed** (Commit ausstehend) |
| [#11](https://github.com/frankhildebrandt/vzctl/issues/11) | Helper Reconnect | **closed** |
| [#20](https://github.com/frankhildebrandt/vzctl/issues/20) | `vzctl doctor` | macOS + supervisor health |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) / [#13](https://github.com/frankhildebrandt/vzctl/issues/13) | Guest-Agent vsock | ← **next** |
| [#16](https://github.com/frankhildebrandt/vzctl/issues/16) | Time-Sync nach Sleep | G0 Prozedur deferred |

Stories sind als **Sub-Issues** unter den Epics verknüpft.

Issue-Bodies sind **implementationsreif** formatiert und mit G0-Messungen (2026-07-30) aktualisiert.

## Label-Schema

- `type:` epic | story | spike | adr | chore
- `priority:` p0 | p1 | p2
- `area:` supervisor | helper | agent | dns | network | …
- `phase:` g0 | p0 | … | v02
- `finding:` fable | sol | plan
