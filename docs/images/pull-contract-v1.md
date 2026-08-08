# Image Pull Contract v1

`vzctl image pull <alias>` lädt ein ARM64-Cloud-/Disk-Image, prüft den
Upstream-Digest und normalisiert es als VZ-taugliches Raw-Image. Installer-ISOs
und amd64-Images sind nicht Teil dieses Vertrags.

```bash
vzctl image list
vzctl image pull ubuntu-latest
vzctl image pull coreos-latest --format json
vzctl image seal ubuntu-latest
vzctl vm create web --from ubuntu-latest --disk 20
```

## Katalog und Pins

| Alias | Release/Kanal | ARM64-Quelle | Eingabe |
|---|---|---|---|
| `ubuntu-latest` | Ubuntu 26.04 LTS | `cloud-images.ubuntu.com/releases/26.04/release/ubuntu-26.04-server-cloudimg-arm64.img` | qcow2 |
| `debian-latest` | Debian 13 Stable (Trixie) | `cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-arm64.qcow2` | qcow2 |
| `alpine-latest` | Alpine 3.24.1 Generic UEFI cloud-init | `dl-cdn.alpinelinux.org/alpine/v3.24/releases/cloud/generic_alpine-3.24.1-aarch64-uefi-cloudinit-r0.qcow2` | qcow2 |
| `arch-latest` | Arch Linux ARM, UTM-Snapshot `archlinux-arm64` | `utmapp/vm-downloads` Asset `archlinux-arm64-utm4.zip` | zip/qcow2 |
| `fedora-latest` | Fedora 44 Cloud Base 44-1.7 | Fedora `releases/44/Cloud/aarch64/images` | qcow2 |
| `rocky-latest` | Rocky Linux 10.2 GenericCloud | Rocky `10/images/aarch64` | qcow2 |
| `alma-latest` | AlmaLinux 10 Stable GenericCloud | AlmaLinux `10/cloud/aarch64/images` | qcow2 |
| `opensuse-latest` | openSUSE Leap 16.0 Minimal Cloud | `distribution/openSUSE-current/appliances` | qcow2 |
| `fedora-coreos-latest`, `coreos-latest` | Fedora CoreOS Stable | FCOS Stable-Stream-Metadaten, QEMU aarch64 | qcow2.xz |
| `flatcar-latest` | Flatcar Stable | `stable.release.flatcar-linux.net/arm64-usr`, QEMU UEFI | raw.bz2 |
| `photon-latest` | VMware Photon OS 5.0 GA | offizielles ARM64-UEFI-OVA | ova/vmdk |
| `opensuse-microos-latest` | openSUSE MicroOS Tumbleweed Current | `ports/aarch64/tumbleweed/appliances` | qcow2 |
| `talos-latest` | Talos aktuelles Stable-Release | GitHub-Asset `metal-arm64.raw.zst` | raw.zst |

`fedora-latest` verwendet ausschließlich Fedora Cloud Base, niemals
Workstation oder Live Media. `coreos-latest` und `fedora-coreos-latest`
registrieren dasselbe Objekt und denselben kanonischen Alias.

Arch Linux veröffentlicht derzeit kein offizielles ARM64-Cloud-Disk-Image.
`arch-latest` ist deshalb die dokumentierte Ausnahme: ein ARM64-VM-Snapshot
von UTM auf Basis von Arch Linux ARM. Das unveränderliche ZIP ist mit
SHA256 `e9891d07b5f1174cc5fc2a37dbb3844de5f9a2d3a5d3ee606891d9470196cfa8`
gepinnt. Ein offizielles Arch-ARM64-Cloud-Image soll diesen Eintrag ersetzen,
sobald Upstream eines veröffentlicht.

## Checksum und Store

Statische Releases werden gegen das veröffentlichte SHA256-/SHA512-Manifest
oder einen dokumentierten Inline-Digest geprüft. FCOS und Talos liefern
SHA256 im Release-Metadatum; Flatcar und Debian werden gegen SHA512 geprüft.
Zusätzlich berechnet `vzctl` immer SHA256 über das normalisierte Raw-Image.
Ein Mismatch veröffentlicht weder Objekt noch Alias.

Der lokale Store ist:

```text
~/Library/Application Support/vzctl/images/
  objects/<normalized-sha256>.raw
  aliases/<alias>.json
  baked/<canonical-alias>@<tag>.raw
  sealed/<canonical-alias>@<tag>.raw
  .tmp/
```

`VZCTL_IMAGES_DIR` überschreibt den Pfad. Alias-Manifeste verwenden
`vzctl.dev/image-alias/v1` und enthalten Release, Architektur, Quell-URL,
Upstream-Digest, Raw-SHA256, relativen Objektpfad sowie `tags.<tag>` für
Bake-/Seal-Artefakte. Temporäre Downloads und
Konvertierungen werden erst nach erfolgreicher Prüfung atomar veröffentlicht.

Ein erneuter Pull fragt das aktuelle Release-/Checksum-Metadatum ab. Verweist
der Alias bereits auf denselben geprüften Digest und stimmt der lokale
Raw-SHA256, ist das Ergebnis `unchanged`; das große Image wird nicht erneut
geladen.

## Normalisierung und Lifecycle

- qcow2, VMDK und Archive mit qcow2/VMDK werden über `qemu-img convert -O raw`
  normalisiert.
- xz, zstd und bzip2 werden vor der Konvertierung entpackt.
- Benötigte lokale Werkzeuge sind je nach Quelle `qemu-img`, `xz`, `zstd`,
  `bzip2`, `tar` oder `unzip`; ein fehlendes Werkzeug liefert Exit `12`.
- Das content-addressed Pull-Objekt bleibt unverändert und **nicht sealed**.
- `image seal` bleibt der separate Agent-/Clone-safe-Schritt. Container-OS
  benötigen später eigene Ignition-/Machine-Config- beziehungsweise
  `talosctl`-Flows.
- `image seal <alias> --tag <tag>` materialisiert zuerst
  `sealed/<canonical-alias>@<tag>.raw`, verändert nur diese Arbeitskopie und
  schreibt `tags.<tag>` im Alias-Manifest. Das Pull-Objekt und sein SHA256
  bleiben erhalten.
- Apply/`ensure_images` skippt Bake+Seal, wenn `tags.<tag>` bereits sealed
  inkl. Marker ist. `vm create --from` erhält den aufgelösten sealed Pfad.

## List

`vzctl image list` liest `aliases/*.json` und meldet den lokalen Cache plus den
festen Pull-Katalog (ohne Netzwerk). Kanonischer JSON-Command: `image.list`.
Payloads: `images[]` (Alias, Kanonik, Distribution/Release, Pfad, SHA256,
`baked`/`sealed`, optional `agent_version`) und `catalog[]` (alle
`CATALOG`-Einträge). `summary.count` / `summary.images_dir` beschreiben den
Store. Leerer Cache ist Exit `0`.

## CLI-/JSON-Vertrag

Der kanonische JSON-Command ist `image.pull`. Erfolg enthält
`summary.change=pulled|unchanged`, `image` mit Alias, Release, Raw-SHA256,
Pfad und Seal-State sowie `source` mit URL, Format und Upstream-Digest. Ein
frischer Pull liefert `sealed=false`; ein unveränderter Re-Pull eines danach
versiegelten Alias darf `sealed=true` melden. Fortschritt (Phasenmeldungen,
Download-Bar, `qemu-img`-Convert) geht nur auf stderr — default wenn stderr ein
TTY ist, oder wenn `VZCTL_PROGRESS` truthy gesetzt ist (Supervisor-Jobs);
stdout bleibt das Ergebnisdokument.

| Exit | Bedeutung |
|---|---|
| `0` | erfolgreich oder unverändert |
| `2` | Usage/Flag ungültig |
| `3` | Alias unbekannt |
| `12` | lokales Normalisierungswerkzeug fehlt |
| `15` | lokaler Store, Manifest oder Normalisierung inkonsistent |
| `21` | Netzwerk- oder Upstream-Metadatenfehler |
| `22` | Checksum-Mismatch, ungültiger Upstream-Digest oder lokaler Digestfehler |
| `23` | Host-Architektur ist nicht ARM64 |

Out of scope sind OCI-Registries, amd64, Agent-Bake beim Pull, Ignition,
Talos-Machine-Config und fertige Kubernetes-Cluster-Bundles.
