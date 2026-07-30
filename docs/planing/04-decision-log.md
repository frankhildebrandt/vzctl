# Decision Log

Festgezogene Entscheidungen nach der [Fable-Review](02-fable-review.md) (2026-07-30).

## Must-Fixes (übernommen)

| # | Entscheidung | Default |
|---|---|---|
| 1 | Prozessmodell | **Supervisor + 1 Helper-Prozess pro VM** (nicht Monolith) |
| 2 | Guest control plane | **vsock Guest-Agent** first-class; SSH nur Fallback |
| 3 | DNS / Domain | Kanonisch: `{vm}.{net}.{project}.vz` |
| 4 | DNS-Anbieter | **Hypervisor** bietet autoritativen DNS für die interne Zone |
| 5 | Host-Auflösung | **macOS `/etc/resolver/{project}.vz`** → Hypervisor-DNS |
| 6 | `*.localhost` | Nur **Host-Alias** in v0.2 — nie kanonischer OIDC-Issuer |
| 7 | OIDC Issuer | `https://auth.svc.{project}.vz` |
| 8 | Ingress / OIDC / CA-Rollout | **v0.2**, nicht v0.1-Muss |
| 9 | Build vs Embed | **Caddy + Dex embedden**, nicht selbst bauen |
| 10 | MVP-Schnitt | v0.1 ≈ 8–10 Wochen Core; P5 = v0.2 |

## IP / Plattform (Spike offen)

- Woche-1-Spike: DHCP vs. static Precedence pro vmnet-Mode
- Bridged braucht ggf. `com.apple.vm.networking` (Apple Approval)
- Entscheidung offen: **macOS 26 Baseline** vs. getesteter Pre-26-Fallback

## Positionierung

Nicht gegen OrbStack / Multipass antreten.

**Nische:** „compose für VM-Topologien“ / Environments as Code für macOS-VMs — Multi-VM, echte Netze, git-native, agent-steuerbar.
