# Image Bake Contract v1

`vzctl image bake <alias> --tag <tag>` baut den Guest-Agent in ein zuvor
gepulltes Raw-Objekt ein, ohne das content-addressed Pull-Objekt zu
verändern. Das Ergebnis ist ein **getaggtes** Artefakt.

```bash
vzctl image pull ubuntu-latest
vzctl image bake ubuntu-latest --tag v1
vzctl image seal ubuntu-latest --tag v1
```

`--tag` ist Pflicht (1–64 `[A-Za-z0-9][A-Za-z0-9._-]*`). Tags sind unabhängige
Artifact-Pins; mehrere Tags dürfen parallel existieren.

## Zustandsmaschine

| Zustand | Pfad | Manifest |
|---|---|---|
| pulled | `objects/<sha>.raw` | `tags` leer oder ohne Eintrag |
| baked | `baked/<canonical>@<tag>.raw` | `tags.<tag>.baked=true`, `baked_image…` |
| sealed | `sealed/<canonical>@<tag>.raw` | `tags.<tag>.sealed=true`, `sealed_image…` |

- Pull-Objekte bleiben immutable.
- Bake kopiert das Pull-Objekt nach `baked/<canonical>@<tag>.raw`, customized nur die Kopie.
- `image seal --tag` materialisiert aus dem Bake-Tag wenn vorhanden, sonst aus `objects/`.
- Bake auf **dasselbe** schon sealed Tag → `unchanged` (kein Fehler).
- Bake auf ein anderes Tag bleibt erlaubt.
- Idempotent: gleiches Tag + gleicher `agent_version` → `summary.change=unchanged`.

## Backend

Gleiches Backend wie Seal: lokales `virt-customize` oder Builder-VM.
Agent-Binary wird per Go cross-build (`GOOS=linux GOARCH=arm64`) aus
`guest-agent/` erzeugt; Version aus `guest-agent/VERSION` bzw.
`VZCTL_AGENT_VERSION`. `iwatch` kommt von GitHub Releases
(`linux_arm64`, Pin `guest-agent/IWATCH_VERSION`, Override
`VZCTL_IWATCH_VERSION` / lokal `VZCTL_IWATCH_BIN`) nach
`/usr/local/bin/iwatch`. Fehlender Download/Checksum → Exit `12`.

## CLI-/JSON-Vertrag

Command `image.bake`. Payloads: `summary.change=baked|unchanged`,
`image.{alias,canonical_alias,tag,path,baked,agent_version}`.

| Exit | Bedeutung |
|---|---|
| `0` | gebacken oder unverändert |
| `2` | Usage (inkl. fehlendes/`ungültiges` `--tag`) |
| `3` | Alias fehlt |
| `12` | Backend/Go/Helper fehlt |
| `13` | Bake/`virt-customize` fehlgeschlagen |
| `15` | Manifest/State inkonsistent |
