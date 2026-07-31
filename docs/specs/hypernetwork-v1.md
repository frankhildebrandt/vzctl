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

`clone` is optional und standardmäßig `linked`. `cpus` (positive Ganzzahl) und
`memory` (MiB als bare Integer oder Size wie `2Gi`/`2048MiB`) sind optional und
steuern Helper-Resources beim Create (Defaults: 2 vCPUs / 1024 MiB). `cloudInit`,
`dependsOn`, `roles` sowie das v0.2-Vorbereitungsfeld `requires` sind optional.

`roles` akzeptiert nur `router` und `docker`. `cloudInit` ist ein relativer Pfad
zur Stack-Config und wird beim Create mit dem System-NoCloud-Seed gemerged
(System-Felder gewinnen bei Skalar-Konflikten; Listen werden angehängt).

Host-Port-Forwards:

- Stack: `spec.ports` — `"8080:web:80"` oder `"127.0.0.1:8080:web:80"`
- VM: `spec.vms.*.ports` — `"8080:80"` oder `"127.0.0.1:8080:80"`

Alpha bindet nur `127.0.0.1`; `0.0.0.0` und doppelte Host-`(bind,port)` sind
Validate-Fehler. Unbekannte Felder werden abgewiesen. Das exportierte Schema ist
JSON Schema Draft 7 und hat die ID
`https://vzctl.dev/schemas/hypernetwork-v1.schema.json`.

virtiofs-Mounts (siehe [virtiofs-v1.md](virtiofs-v1.md)):

- Stack: `spec.volumes` — Map `name → Hostpfad` (relativ zur Config-Datei oder absolut)
- VM: `spec.vms.*.mounts` — `{ source: <volume>, target: /abs/path, readOnly?: false }`
- Volume-Namen: 1–36 Zeichen `[A-Za-z0-9][A-Za-z0-9_-]*`, Tag `vzctl` ist reserviert
- Targets absolut und unique pro VM; `source` muss ein bekanntes Volume referenzieren

v0.2 Ingress / CA / OIDC (siehe [certs-v1.md](certs-v1.md), [ingress-v1.md](ingress-v1.md),
[oidc-v1.md](oidc-v1.md)):

```yaml
certs:
  enabled: true
  onRotate: reinject   # reinject | reboot
ingress:
  enabled: true
  bind: "127.0.0.1"    # nur Loopback in v0.2
  hostAliases: true    # web.localhost → gleicher Upstream (Host only)
  redirectHttp: true
  routes:
    - { host: web.svc.edge-dmz.vz.test, to: "web:80", requires: [oidc] }
    - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
oidc:
  enabled: true
  mode: embedded
  issuer: https://auth.svc.edge-dmz.vz.test   # nie *.localhost
  listen: "127.0.0.1:5556"
  clients: auto
  passwordFile: .vzctl/oidc/passwords.bcrypt
```

- `ingress.routes[].to`: `vm:port` oder `oidc:<port>`
- `oidc.issuer` Host muss `auth.svc.{domain}` sein und (wenn Ingress enabled)
  zu einer Route mit `to: oidc:…` passen
- `requires: [oidc]` (VM oder Route) braucht `oidc.enabled`
- `*.localhost` ist kein kanonischer Route-Host; nur Host-Alias über `hostAliases`

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
