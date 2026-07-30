# GitHub Tracking Map

Issues, Labels und Milestones für **frankhildebrandt/vzctl**.

- Issues: https://github.com/frankhildebrandt/vzctl/issues
- Milestones: https://github.com/frankhildebrandt/vzctl/milestones
- Labels: https://github.com/frankhildebrandt/vzctl/labels
- Machine map: [`06-github-issue-map.json`](06-github-issue-map.json)
- Bootstrap script: [`../../scripts/bootstrap-github-issues.py`](../../scripts/bootstrap-github-issues.py)

## Milestones

| Milestone | Fokus |
|---|---|
| G0 — Spike Gate | Netz+DNS+Crash Go/No-Go |
| P0 — Foundation | Ownership, Helper, Agent, Journal-ADR, doctor |
| P1 — CLI + Clones | JSON/Events, Seal/Clone/Identity |
| P2 — Net + DNS | vmnet, Dual-DNS, Policies |
| P3 — Stacks | hypernetwork reconcile |
| P4 — Docker + Ports | SSH Context, Ports basic |
| v0.1.x — Polish | virtiofs, Docker polish |
| v0.2 — Ingress/OIDC/CA | Caddy, Dex, CA, Tauri |

## Epics

| # | Epic | Milestone |
|---|---|---|
| [#1](https://github.com/frankhildebrandt/vzctl/issues/1) | G0 Netzwerk-/DNS-/Crash-Spike | G0 |
| [#7](https://github.com/frankhildebrandt/vzctl/issues/7) | Process-Modell & Ownership | P0 |
| [#12](https://github.com/frankhildebrandt/vzctl/issues/12) | vsock Guest-Agent | P0 |
| [#17](https://github.com/frankhildebrandt/vzctl/issues/17) | CLI Contracts | P1 |
| [#21](https://github.com/frankhildebrandt/vzctl/issues/21) | Base Seal / Linked Clones / Identity | P1 |
| [#25](https://github.com/frankhildebrandt/vzctl/issues/25) | Dual-DNS + macOS Resolver | P2 |
| [#30](https://github.com/frankhildebrandt/vzctl/issues/30) | vmnet + Routes + Policies | P2 |
| [#34](https://github.com/frankhildebrandt/vzctl/issues/34) | Stack Reconciler | P3 |
| [#39](https://github.com/frankhildebrandt/vzctl/issues/39) | Docker Context + Ports | P4 |
| [#43](https://github.com/frankhildebrandt/vzctl/issues/43) | v0.2 Ingress + CA + OIDC | v0.2 |
| [#48](https://github.com/frankhildebrandt/vzctl/issues/48) | DX Logs / Docs | P1 |

Stories sind als **Sub-Issues** unter den Epics verknüpft; **blocked-by** Dependencies folgen dem Plan (G0 → Ownership → …).

Issue-Bodies sind **implementationsreif** formatiert (Summary, Findings, Scope, Acceptance, Quellen) und enthalten Canvas-/Plan-Details, soweit sie Fable/SOL nicht widersprechen.

## Label-Schema

- `type:` epic | story | spike | adr | chore
- `priority:` p0 | p1 | p2
- `area:` supervisor | helper | agent | dns | network | …
- `phase:` g0 | p0 | … | v02
- `finding:` fable | sol | plan
