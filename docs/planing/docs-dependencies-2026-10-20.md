# D5: Docs-Abhängigkeiten und Browser-Abnahme

Stand: 20.10.2026. Lokale Änderungen, kein Publish.

## Ergebnis

`apps/docs` meldete zunächst 21 npm-Auditbefunde: 1 kritisch, 7 hoch,
11 moderat und 2 niedrig. Nach gezielten Updates meldet der installierte
Paketbaum **0 Befunde**. `npm ci` und der vollständige Astro-Build sind
erfolgreich; der Build erzeugt 13 Seiten ohne Warnungen.

| Paket | Vorher deklarierter Bereich | Jetzt |
| --- | --- | --- |
| Astro | `^5.13.2` | `7.4.1` |
| Starlight | `^0.35.2` | `0.42.6` |
| Sharp | `^0.34.3` | `0.35.5` |
| Markdown Remark | transitiv | `7.3.3`, expliziter Starlight-Peer |

Der Lockfile wurde gemeinsam mit den direkten Versionen aktualisiert.
Es wurde weder `npm audit fix --force` noch eine erzwungene Peer-Auflösung
verwendet. Eine temporäre frische Paketauflösung vermied den Konflikt mit
dem bisherigen Astro-5-Peer-Baum.

## Begründung und Quellen

- Die [Astro-AVIF-Advisory](https://github.com/advisories/GHSA-26w7-cxv4-gfx2)
  nennt 7.2.8 als erste gepatchte Version. Angreiferkontrollierte AVIF-Bilder
  konnten bei der Verarbeitung durch den Sharp-Bilddienst Codeausführung
  auslösen. Die statisch veröffentlichten HTML-Dateien betreiben keinen
  solchen Bildserver; relevant bleibt unter anderem die Build-Verarbeitung.
- Die [Sharp/librsvg-Advisory](https://github.com/advisories/GHSA-wq5f-xc86-pv6w)
  verlangt mindestens 0.35.5. Diese Version deckt außerdem die vorherigen
  libheif-/libvips-Fixes ab.
- Die offiziellen Migrationshinweise für
  [Astro 6](https://docs.astro.build/en/guides/upgrade-to/v6/) und
  [Astro 7](https://docs.astro.build/en/guides/upgrade-to/v7/) wurden geprüft.
  Die unterstützte Mindestversion ist jetzt Node `22.22.0`, npm `9.6.5`,
  ausdrücklich im Docs-Paket hinterlegt. Die Docs-CI verwendet Node `22`;
  lokal wurde mit Node `26.12.0` gebaut.
- Als einziger verbleibender transitiver Befund benötigte der
  [PostCSS-Parser](https://github.com/advisories/GHSA-rj75-hqrm-r3gf) mindestens
  7.1.6. Expressive Code hängt noch an `postcss-nested ^6`.
  Der begrenzte Override betrifft ausschließlich
  `@expressive-code/core → postcss-nested 7.0.2`; dessen Parser-Bereich erlaubt
  den gepatchten Parser. Die
  [Nested-7-Releaseänderungen](https://github.com/postcss/postcss-nested/releases/tag/7.0.0)
  betreffen die Mindest-Node-Version und Kommentarplatzierung. Der komplette
  Doku-/Codeblock-Build und die Browser-Abnahme prüfen die tatsächliche Nutzung.
  Diesen Override bei einem entsprechenden Upstream-Update wieder entfernen.

## Kompatibilitätskorrekturen

- Explizite i18n-Collection und deutsche 404-Seite verhindern Warnungen über
  fehlende Einträge. Die 404-Collection ist für den generischen Docs-Catch-all
  als Draft markiert; Starlights eigene `/404`-Route rendert sie ausdrücklich.
  Sie wird nicht in die Suche aufgenommen.
- Suche sitzt jetzt im normalen Header-Layout statt verdeckt hinter einer
  zweiten Kopfzeile. Mobil sind Website- und Dokumentationsmenü getrennt
  angeordnet und beschriftet; die Suche bleibt erreichbar.

## Prüfung und Grenzen

- `npm ci`: erfolgreich, Audit 0.
- Abschließendes `npm audit --json`: alle Schweregrade 0.
- `npm ls`: Astro/Starlight/Markdown/Sharp ohne Peer-Konflikte.
- `npm run build`: 13 Seiten, keine Warnungen oder Fehler.
- Chromium gegen ausschließlich lokale statische Vorschau: 390, 768, 820,
  1024 und 1440 Pixel. Suche mit tatsächlichen Docker-Treffern, beide Menüs,
  Navigation nach Kubernetes, kein horizontaler Überlauf.
- Deutsche 404-Seite gerendert; keine Browser-Ausnahmen. Desktop und Mobil
  zusätzlich visuell geprüft.

Der Auditbefund ist eine Momentaufnahme des aktuellen npm-Advisory-Datensatzes.
Kein Release oder Hosting-Deployment wurde ausgeführt. npm 11 meldete beim
Installieren den noch nicht freigegebenen optionalen Installationshook von
`fsevents 2.3.3`; dieser wurde nicht pauschal freigegeben. Der statische Build
benötigt ihn nicht. Ein separater Dauerbetrieb des Entwicklungs-Watchers wurde
nicht abgenommen.
