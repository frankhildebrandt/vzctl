# hypernetwork/v1

`hypernetwork/v1` ist der deklarative Desired State für einen vzctl-Stack.
Die Rust-Typen und das daraus erzeugte JSON Schema liegen in
`crates/vzctl/src/config.rs`.

## Validieren

```bash
vzctl validate -C ./examples/edge-dmz
vzctl validate -C ./examples/edge-dmz --format json
vzctl validate --schema > hypernetwork-v1.schema.json
```

`-C` akzeptiert ein Verzeichnis mit `hypernetwork.config.yaml` oder direkt
eine Config-Datei. Erfolg liefert Exit `0`, ungültige Config Exit `3` und
Usage-Fehler Exit `2`. Das JSON-Ergebnis folgt dem CLI-v1-Envelope.

Fehler enthalten einen JSON-Pfad und eine Art:

```json
{
  "kind": "semantic",
  "path": "$.spec.routes[0].via",
  "message": "route via references unknown VM \"missing-router\""
}
```

## Pflichtstruktur

- `apiVersion: hypernetwork/v1`, `kind: Environment`
- `metadata.name`
- `spec.project`, `spec.domain` mit Suffix `.vz.test`
- `spec.dns`, `images`, `networks`, `routes`, `policies`, `vms`
- VM: `from`, `dataDisk`, mindestens ein `networks[]` mit `name` und `ip`

`clone` ist optional und standardmäßig `linked`. `cloudInit`, `dependsOn`,
`roles` sowie das v0.2-Vorbereitungsfeld `requires` sind optional.
Unbekannte Felder werden abgewiesen. Das exportierte Schema ist JSON Schema
Draft 7 und hat die ID
`https://vzctl.dev/schemas/hypernetwork-v1.schema.json`.

## Semantische Regeln

- Image-, Network-, Route-, Policy- und VM-Referenzen müssen existieren.
- `route.via` muss eine Router-VM sein, die an Quell- und Zielnetz hängt.
- Netz-CIDRs müssen gültige kanonische IPv4-Netze sein.
- Statische IPs müssen im CIDR liegen, dürfen weder Netzwerk/Broadcast noch
  die reservierten Offsets `.0`/`.1` verwenden und dürfen nicht doppelt sein.
- DHCP und statische VM-IP auf demselben Netz sind unzulässig. DHCP bleibt
  gemäß G0/Decision Log standardmäßig aus.
- `dependsOn` darf nur bekannte VMs referenzieren und muss ein DAG bilden.
- Policies referenzieren bekannte Netze; TCP/UDP brauchen Ports, ICMP nicht.

Die Validierung verändert weder Runtime-State noch Journal/Lease. Reconcile
und Apply folgen separat in [#37](https://github.com/frankhildebrandt/vzctl/issues/37).
