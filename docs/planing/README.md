# Planning — vzctl

Planungsartefakte für den Apple-VZ Devstack-Supervisor (Arbeitstitel **vzctl**).

Entstanden in Cursor (Juli 2026): Vergleich → Plan → Features → Fable-Review → Must-Fixes → GPT-SOL-Review → Plan-Update.

## Dokumente

| # | Datei | Inhalt |
|---|---|---|
| 01 | [01-implementation-plan.md](01-implementation-plan.md) | Aktueller Plan (Fable + SOL Must-Fixes) |
| 02 | [02-fable-review.md](02-fable-review.md) | Fable-5-High-Bewertung |
| 03 | [03-utm-multipass-hyperkit-vergleich.md](03-utm-multipass-hyperkit-vergleich.md) | UTM · Multipass · Self-made |
| 04 | [04-decision-log.md](04-decision-log.md) | Festgezogene Entscheidungen |
| 05 | [05-gpt-sol-review.md](05-gpt-sol-review.md) | GPT 5.6 SOL Medium-Bewertung |
| 06 | [06-github-tracking.md](06-github-tracking.md) | Labels, Milestones, Epics, Issue-Map |
| — | [canvases/](canvases/) | Original `.canvas.tsx` Quellen |

## Lesereihenfolge

1. Vergleich (03) — Warum Self-made / VZ?
2. Fable-Review (02) — erste kritischen Löcher
3. GPT-SOL-Review (05) — Feinschliff nach Must-Fixes
4. Decision Log (04) — was übernommen wurde
5. Implementation Plan (01) — aktueller Soll-Zustand
6. GitHub Tracking (06) — Epics/Stories

## Nächster Schritt

**P1 / #17** — #18 CLI-v1-Contract für JSON, Exitcodes und Events
vervollständigen. `doctor` liefert bereits den ersten JSON-Envelope; danach
#21 Base Seal / APFS Linked Clones. P0 #20 ist abgeschlossen.
