# P1 Spike: Identity-Reset pro Clone

- **Issue:** #24, letztes Child von Epic #21
- **Voraussetzungen:** #22 sealed Base, #23 writable Root/Data-Bundle
- **Stand:** Code sowie Rust-/Swift-Tests abgeschlossen; Live-Boot-Smoke bleibt
  host-/imageabhängig

## Ergebnis

`vzctl vm create <id> --from <sealed> --data-disk <GiB>` erzeugt zusätzlich:

```text
vm.bundle/
├── cidata.iso    # read-only am Gast, Mode 0600 auf dem Host
├── agent.token   # 256 Bit, unpadded base64url, Mode 0600
└── vm.json       # instance-id, hostname/fqdn und MAC pro NIC
```

Jeder Create-Aufruf verwendet neue Zufallswerte:

| Feld | Umsetzung |
|---|---|
| MAC | `02:…`, im Manifest persistiert und vom Helper am frischen VZ-NIC gesetzt |
| instance-id | UUID v4 in NoCloud `meta-data` |
| Hostname/FQDN | aus der VM-ID normalisiert und per cloud-init gesetzt |
| SSH Host Keys | `ssh_deletekeys: true`, Neugenerierung von Ed25519 und RSA |
| machine-id | Seal leert machine-id und dbus-ID; systemd regeneriert beim ersten Clone-Boot |
| Netzwerk | NoCloud matcht die persistierte MAC; aktueller NAT-CLI-Pfad bezieht IPv4 per DHCP |

Der Clone-Pfad öffnet ausschließlich die neue Root-Disk writable. Base und
Seal-Marker werden weder für Identity noch für Seed verändert.

## Automatisierte Nachweise

- Zwei Bundles erhalten verschiedene UUIDs, MACs und Agent-Tokens.
- MACs beginnen mit `02:`; NoCloud matcht exakt diese MAC.
- user-data fordert neue SSH Host Keys an.
- `cidata.iso` und `agent.token` haben Host-Modus `0600`.
- Der native macOS-Test baut das NoCloud-ISO mit `hdiutil`.
- Swift liest die MAC aus `vm.json`; ein explizites `--mac-address` bleibt für
  bestehende/manuelle Bundles möglich.

## Live-Boot-Acceptance

Mit zwei Clones derselben sealed Base prüfen:

```bash
for vm in clone-a clone-b; do
  vzctl vm create "$vm" --from ubuntu-base --data-disk 1
  daemon/.build/debug/vz-helper run \
    --vm-id "$vm" \
    --bundle "$HOME/Library/Application Support/vzctl/vms/$vm"
done
```

Nach dem Boot per Guest-Agent oder Konsole vergleichen:

```bash
cat /etc/machine-id
cat /sys/class/net/*/address
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
cloud-init query instance_id
hostname --fqdn
```

Erwartet: machine-id, MAC, SSH-Fingerprint und instance-id unterscheiden sich;
der Hostname entspricht der jeweiligen VM-ID. SSH-Verbindungen zu den beiden
VM-Adressen erzeugen keine Warnung wegen identischer Host Keys.

## Grenzen

Statische IPs aus `hypernetwork.config.yaml` werden mit dem späteren
Reconciler in dieselbe per-NIC network-config gerendert. Registry Pull und
vollständige Purge-CLI bleiben außerhalb dieses Slices.
