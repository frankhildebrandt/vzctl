# Planning — vzctl

Planungsartefakte für den Apple-VZ Devstack-Supervisor (Arbeitstitel **vzctl**).

Entstanden in Cursor (Juli 2026): Vergleich → Plan → Features → Fable-Review → Must-Fixes → GPT-SOL-Review → Plan-Update.

## Dokumente

| # | Datei | Inhalt |
|---|---|---|
| 01 | [01-implementation-plan.md](01-implementation-plan.md) | Ursprünglicher Plan (Fable + SOL Must-Fixes) |
| 02 | [02-fable-review.md](02-fable-review.md) | Fable-5-High-Bewertung |
| 03 | [03-utm-multipass-hyperkit-vergleich.md](03-utm-multipass-hyperkit-vergleich.md) | UTM · Multipass · Self-made |
| 04 | [04-decision-log.md](04-decision-log.md) | Festgezogene Entscheidungen |
| 05 | [05-gpt-sol-review.md](05-gpt-sol-review.md) | GPT 5.6 SOL Medium-Bewertung |
| 06 | [06-github-tracking.md](06-github-tracking.md) | Labels, Milestones, Epics, Issue-Map |
| 07 | [07-controller-lab-acceptance.md](07-controller-lab-acceptance.md) | Isolierte Live-Abnahme für K3s und Controller |
| 08 | [08-cli-release-contract-matrix.md](08-cli-release-contract-matrix.md) | CLI-Vertragsprüfung und geprüfte Korrekturen |
| 09 | [09-live-update-acceptance.md](09-live-update-acceptance.md) | Freigegebenes Betriebsupdate und Abnahme |
| 10 | [10-next-slice.md](10-next-slice.md) | FullClaims-/Consumer-Abnahme, Wiederholung, Ausfälle, Retention und Auslieferung |
| 11 | [11-certificate-upgrade-2026-10-20.md](11-certificate-upgrade-2026-10-20.md) | F45: strikte TLS-Prüfung, Bestandsmigration und Trust-Rollout |
| 12 | [12-release-completion-goal.md](12-release-completion-goal.md) | Maßgebliches Arbeitsziel, drei Abschlussblöcke und begrenzte Restabnahme |
| — | [ui-native-rendering-2026-10-20.md](ui-native-rendering-2026-10-20.md) | F3/F28/V1: native Abnahme, Hintergrund-Rendering und Regressionen |
| — | [canvases/](canvases/) | Original `.canvas.tsx` Quellen |

## Lesereihenfolge

1. Vergleich (03) — Warum Self-made / VZ?
2. Fable-Review (02) — erste kritischen Löcher
3. GPT-SOL-Review (05) — Feinschliff nach Must-Fixes
4. Decision Log (04) — was übernommen wurde
5. Implementation Plan (01) — ursprünglicher Soll-Zustand
6. GitHub Tracking (06) — Epics/Stories

## Aktueller Arbeitsstand — 20.10.2026

Für die weitere Arbeit gilt der [Abschlussplan](12-release-completion-goal.md).
Er bündelt den unveränderten vereinbarten Umfang in drei Abschlussblöcken.
Ältere Zwischenstände in den Nachweisberichten sind historisch.

Das [Fertigstellungsdashboard](canvases/stack-review-2026-10-20.canvas.tsx)
und [Release-Arbeitspaket #73](https://github.com/frankhildebrandt/vzctl/issues/73)
führen die aktuelle Abarbeitung. Erledigt bedeutet dort Code und Tests;
Native-Live-Abnahme und Veröffentlichung sind eigene Schritte.

- Bestehende Befunde: REST/Health, Caddy/DNS-Recovery, Lease/Resume, UI-Verträge
  und fachliche Aufteilung der großen Dateien.
- [#74](https://github.com/frankhildebrandt/vzctl/issues/74): K3s und lokale
  Kubernetes-YAMLs pro Stack in `hypernetwork.config.yaml`.
- [#75](https://github.com/frankhildebrandt/vzctl/issues/75): Ingress/OIDC/DB/S3-
  Controller für VM, Docker und K3s, einschließlich Agent-Zugriff.
- [#76](https://github.com/frankhildebrandt/vzctl/issues/76): vollständige Pakete
  und Stop-Reihenfolge vor Binary-Replacement.

Die [native UI-Abnahme F3/F28/V1](ui-native-rendering-2026-10-20.md) und die
neutrale Bewertung gestoppter Stacks sind abgeschlossen. CLI/Swift/UI-Verträge
und die Modulaufteilung sind abgesichert. Im Controller-Lab bestehen VM,
Docker und K3s gemeinsam PostgreSQL/S3/OIDC/HTTPS, normales Pod-DNS und
striktes TLS. MCP-Status/Diagnose, privater Kubeconfig-Export und wiederholtes
Apply ohne unnötige Neustarts sind live belegt.

Vier nachträgliche Claims, MCP mit neun Claims und vier Besitz-/Ausfallfälle
sind bestanden, ebenso Retention, Supervisor-Neustart und Lab-Down. Der native UI-Apply war in 37 Sekunden erfolgreich; auch die finalen VM-/Docker-/K3s-Consumerproben sind bestanden. Daten/Bindings erhalten, Generation14 abgeschlossen. Dashboard:61/63, nur F16/F17 wegen Repository-/Release-/externen Installer-Schritten offen.
Aktuelle F48-Dev-Pakete und der [Paketprüfbericht](package-verification-2026-10-20.md)
sind fertig. Ein separater macOS-26-Mac für saubere PKG-Erstinstallation/Update
fehlt weiterhin. Der [Folgeauftrag](10-next-slice.md) nennt ausschließlich
die aktuellen nächsten Schritte; gemeinsame Restkriterien werden einmal geprüft.

Virtiofs ist bereits implementiert; es ist kein zukünftiger Grundlagenslice.
Diagnosebundle und RAMwarnungen aus #48 sind auch live abgenommen; siehe
[Diagnosenachweis](diagnose-live-2026-10-20.md). Commit/Push und Draft-PR sind nun freigegeben; Übergabe auf
codex/release-readiness-2026-10-20 in Arbeit. Release noch nicht veröffentlicht.
