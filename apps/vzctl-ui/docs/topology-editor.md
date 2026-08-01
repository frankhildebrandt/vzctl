# Topology-Editor (hypernetwork/v1)

Visueller Netzwerk-/VM-Editor in **vzctl-ui**. Persistenz ist immer
`hypernetwork.config.yaml`. Diagrammlayout liegt in `.vzctl/diagram.json`.

## Architektur

```
Domain (Zod Environment)
  → Commands / Undo-History (Zustand)
  → Diagram Projection (inkrementell)
  → AntV X6 Graph
```

X6 ist nur Darstellung und Interaktion. Fachliche Daten sind vom Graph getrennt.

## Domain-Modell

Entspricht Spec [`docs/specs/hypernetwork-v1.md`](../../../docs/specs/hypernetwork-v1.md):

| UI | YAML |
|---|---|
| Netzwerk-Container | `spec.networks.*` (nur single-homed VMs als Children) |
| VM-Node (im Container) | `spec.vms.*` mit genau einer NIC |
| Multi-homed VM (außerhalb) | `vms.*.networks.length > 1` — Kanten zu allen Netzen |
| Router-VM | `roles: [router]` + `spec.routes` |
| NIC-Kante | Port ↔ Netz-Attach (nur multi-homed) |
| Firewall-Regeln | `spec.policies[].allow` (`to` Netz oder `internet`) |
| IGW-Node | nur visual, wenn `natEgress !== false` |

**Zuweisung:** Single-homed VM in einen Netz-Container ziehen (Drag & Drop)
setzt die Primär-NIC und vergibt eine neue IP im Ziel-CIDR. Sobald eine
zweite NIC hängt, wandert die VM nach außerhalb und verbindet per Port-Kanten
(Primär durchgezogen, weitere gestrichelt).
`natEgress: false` blendet die Internet-Wolke aus und legt bei Bedarf eine
leere deny-all Policy an.

Beim Öffnen läuft automatisch `layoutByNetwork` + Fit View.

## X6-Plugins (3.x, aus `@antv/x6`)

Selection, Snapline, Keyboard, Clipboard, Scroller, MiniMap, Transform, Export.
**History ist deaktiviert** — Undo/Redo läuft über die fachliche Command-History.

## Synchronisation

1. Load YAML + Diagram-Sidecar → Store
2. Commands mutieren Environment + DiagramState atomar
3. `projectToGraph` mit `projecting`-Flag (X6-Events während Projection ignoriert)
4. X6 move/connect/delete → Commands

## Build & Start

```bash
cd apps/vzctl-ui
npm install
cargo build -p vzctl   # CLI für validate/apply
npm run tauri:dev
```

Nur Frontend: `npm run dev` (Persistenz dann Memory-Fallback).

## Tests

```bash
npm test              # Vitest Unit
npx playwright install chromium
npm run test:e2e      # Playwright (Browser, Memory-Store)
```

## Bekannte Einschränkungen

- Kein Switch / LoadBalancer in der Spec → nicht in der Palette
- YAML-Kommentare gehen beim Speichern verloren
- Subnets = Network-CIDR
- Policy = deny-all + allow-to-network
- IGW nicht in YAML

## Beispiel

`examples/edge-dmz` laden (Stack öffnen). Optional Layout:
`.vzctl/diagram.json` (siehe `examples/edge-dmz/.vzctl/diagram.json`).
