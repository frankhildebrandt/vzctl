# Vergleich: UTM · Multipass · Self-made (HyperKit/VZ)

> Vorarbeit zum vzctl-Plan (Juli 2026)  
> Canvas-Quelle: [`canvases/utm-multipass-hyperkit-vergleich.canvas.tsx`](canvases/utm-multipass-hyperkit-vergleich.canvas.tsx)

## Kontext

Bewertung von Scriptbarkeit und Flexibilität für:

- Ansible Playbook Dev/Test
- Netzwerke aufbauen
- Kubernetes
- Docker
- Agentisches Orchestrieren

Dritter Kandidat: Self-made nach HyperKit-Muster — auf modernen Macs = **Virtualization.framework** (HyperKit selbst ist auf Apple Silicon faktisch tot).

## Scores (Auszug)

### Capability Baseline

| Kriterium | UTM | Multipass | Self-made |
|---|---|---|---|
| Ansible | 2 | **5** | 4 |
| Netzwerk | 4 | 3 | **5** |
| Kubernetes | 2 | **5** | 3 |
| Docker | 3 | **5** | 4 |
| Agentisch | 2 | **5** | 4 |
| Scriptbarkeit | 2 | **5** | 4 |
| Flexibilität | **5** | 3 | **5** |

**Baseline-Gewinner:** Multipass (Time-to-Value, CLI, JSON).

### Capability mit Agent/∞ Tokens (Self-made gebaut)

Self-made steigt auf ~5/5 Capability → Peak-Gewinner. UTM/Multipass unverändert.

### Produktive Dev-Arbeit

| | Multipass | Self-made+Agent |
|---|---|---|
| Tages-Zuverlässigkeit | **5** | 3 |
| Feedback-Loop | **5** | 4 |
| Unblock in Minuten | **5** | 3 |
| Team/Onboarding | **5** | 3 |

**Prod-Gewinner:** Multipass — Ownership-Risiko schlägt Peak-Capability im Sprint.

## Fazit → vzctl

Self-made lohnt sich, wenn:

- Multi-VM-**Topologien** + Routing das Lieferobjekt sind
- deklarative Git-Environments + Agent-API First-Class sein sollen
- ein Platform-/Supervisor-Ansatz den Break/Fix trägt

Daraus entstand der **vzctl**-Plan: nicht „besseres Multipass“, sondern **compose für VM-Topologien** auf VZ — mit den Must-Fixes aus der [Fable-Review](02-fable-review.md) (Helper-pro-VM, vsock-Agent, Hypervisor-DNS + macOS-Resolver, MVP-Schnitt).

## Drei Bewertungsebenen (Merksatz)

1. **Capability Baseline** → Multipass  
2. **Capability Agent/∞** → Self-made  
3. **Produktive Dev-Arbeit** → Multipass (außer Topologie/API = Produkt)
