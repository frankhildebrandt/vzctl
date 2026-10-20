# Native UI-Apply-Abnahme · 20.10.2026

Installierte App: /Users/frank/Applications/vzctl.app, Binary-SHA256
c0728d18154013d9c8cfc36238703162609aa8a3fe174830afe91d88dbfcde0a.

## Ablauf und Ergebnis

- Nach erfolgreichem Supervisor-Neustart und graceful Lab-Down zeigte die
  Betriebsseite Down, drei gestoppte Lab-VMs und genau drei Start-Aktionen.
  Die zuvor alte Ansicht wurde durch Dashboard → Lab neu eingelesen.
- Genau einmal Apply samt vorhandenem Bestätigungsdialog ausgelöst.
  Konfiguration unverändert; geprüfter Plan ohne Breaking-Aktionen.
- Sichtbares apply.started um20:39:54, invocation
  ebbea683-ed47-4c7a-9cec-3c21050f1917.
- Während aktiv: Debug-Ansicht geöffnet und zu Betrieb zurückgekehrt.
  Beide Wechsel unmittelbar bedienbar, Apply lief weiter; Fortschritt50→95%,
  Log mit laufenden Phasen. Mutierende Aktionsbuttons gesperrt.
- apply.finished und „apply fertig“ um20:40:31, Dauer37Sekunden.
  Danach drei Lab-VMs running, Drift0, Aktionsbuttons wieder frei;
  LOG ließ sich öffnen und zeigte die vollständige Phasenkette.
- Ein Status-Snapshot unmittelbar nach Boot meldete worker agent degraded.
  Frischer CLI-Status bestätigt den begrenzten Performancehinweis:
  exec p99=8511ms, Queue0, Agent ready, Token-/Servicechecks erfolgreich.
  Das ist kein gestoppter VM-Fehler und kein belegter Agent-Defekt;
  kein Upgrade/Restart aufgrund dieses Hinweises. Die Metrik bleibt sichtbar.

Die Bedienbarkeit ist direkt nativ belegt, nicht aus einem CLI-Lauf abgeleitet.
Die interne REST-jobId wird in dieser App nicht angezeigt und vom Supervisor
nicht gelistet. Die sichtbare invocation ist dagegen die konkrete neue
Journal-ID Generation14; der vorhandene Runtime-Observer bindet genau sie.
Ein zweites Apply nur für eine interne Job-ID wurde nicht gestartet.

## Belege

Screenshots und vollständige AX-Texte liegen lokal unter
/private/tmp/vzctl-ui-apply-20261020/. Sie enthalten das Lab-Phasenlog,
keine Credentials. Prüfsummen:

| Datei | SHA256 |
|---|---|
| active.ax.txt | d178e25b203832f955a5a3c128e583f46704be06b881a3b47b77236f32b76a3d |
| active.png | f3214183c04cf434807831c843004546451ad555c25920ca59034d60faed9360 |
| final.ax.txt | 4dc173235f2f293c5250b06dcd99d3c38b5629405471d49f56d30d1c38e512a1 |
| final.png | 4969531a4452dceef2c3e699d2ce1dab2d96848f6781ff5b9c0bf7475d5d3200 |

Runtime-/Daten-/Consumer-Abschluss wird separat in09-live-update-acceptance.md
und der Runtime-Übergabe nachgewiesen. Dieser UI-Beleg allein schließt keine
externen Installer- oder Repository-Auslieferungskriterien.
