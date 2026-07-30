# Image Bake Contract v1

`vzctl image bake <alias>` baut den Guest-Agent in ein zuvor gepulltes
Raw-Objekt ein, ohne das content-addressed Pull-Objekt zu verändern.

```bash
vzctl image pull ubuntu-latest
vzctl image bake ubuntu-latest
vzctl image seal ubuntu-latest
```

## Zustandsmaschine

| Zustand | Pfad | Manifest |
|---|---|---|
| pulled | `objects/<sha>.raw` | `sealed=false`, kein `baked` |
| baked | `baked/<canonical>.raw` | `baked=true`, `baked_image.{path,sha256,agent_version}` |
| sealed | `sealed/<canonical>.raw` | `sealed=true`, `sealed_image…` |

- Pull-Objekte bleiben immutable.
- Bake kopiert das Pull-Objekt nach `baked/`, customized nur die Kopie.
- `image seal` materialisiert aus `baked/` wenn vorhanden, sonst aus `objects/`.
- Bake auf bereits sealed Alias schlägt fehl (Exit 3).
- Idempotent: gleicher `agent_version` → `summary.change=unchanged`.

## Backend

Gleiches Backend wie Seal: lokales `virt-customize` oder Builder-VM.
Agent-Binary wird per Go cross-build (`GOOS=linux GOARCH=arm64`) aus
`guest-agent/` erzeugt; Version aus `guest-agent/VERSION` bzw.
`VZCTL_AGENT_VERSION`.

## CLI-/JSON-Vertrag

Command `image.bake`. Payloads: `summary.change=baked|unchanged`,
`image.{alias,canonical_alias,path,baked,agent_version}`.

| Exit | Bedeutung |
|---|---|
| `0` | gebacken oder unverändert |
| `2` | Usage |
| `3` | Alias fehlt / bereits sealed |
| `12` | Backend/Go/Helper fehlt |
| `13` | Bake/`virt-customize` fehlgeschlagen |
| `15` | Manifest/State inkonsistent |
