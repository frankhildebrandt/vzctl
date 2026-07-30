# Planning — vzctl

Planungsartefakte für den Apple-VZ Devstack-Supervisor (Arbeitstitel **vzctl**).

Entstanden in Cursor (Juli 2026): Virtualisierungs-Vergleich → Implementationsplan → Feature-Erweiterungen → Fable-Review → Must-Fixes.

## Dokumente

| # | Datei | Inhalt |
|---|---|---|
| 01 | [01-implementation-plan.md](01-implementation-plan.md) | Aktueller Implementationsplan (nach Must-Fixes) |
| 02 | [02-fable-review.md](02-fable-review.md) | Fable-5-High-Bewertung + Top-5-Entscheidungen |
| 03 | [03-utm-multipass-hyperkit-vergleich.md](03-utm-multipass-hyperkit-vergleich.md) | UTM · Multipass · Self-made (HyperKit/VZ) |
| 04 | [04-decision-log.md](04-decision-log.md) | Festgezogene Architektur-Entscheidungen |
| — | [canvases/](canvases/) | Original `.canvas.tsx` Quellen |

## Lesereihenfolge

1. Vergleich (03) — Warum Self-made / VZ?
2. Fable-Review (02) — kritische Löcher
3. Decision Log (04) — was wir übernommen haben
4. Implementation Plan (01) — aktueller Soll-Zustand

## Cursor Canvases (lokal)

Die lebendigen Canvases lagen unter:

- `~/.cursor/projects/empty-window/canvases/vz-hypervisor-implementationsplan.canvas.tsx`
- `~/.cursor/projects/empty-window/canvases/utm-multipass-hyperkit-vergleich.canvas.tsx`

Kopien liegen in [`canvases/`](canvases/).
