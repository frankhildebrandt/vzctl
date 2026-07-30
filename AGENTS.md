## Learned User Preferences

- Antworten immer auf einfachem, kurzem Deutsch; nicht ausschweifen.
- Analysen, Vergleiche und Implementationspläne bevorzugt als Canvas aufbereiten und bei Plan-Änderungen mitaktualisieren.
- Plan-Reviews mit explizit genanntem Modell (z. B. Fable, GPT-SOL) als Subagent; ohne Angabe Parent-/Auto-Modell erben, kein stiller Override.
- GitHub-Issues für vzctl sollen implementationsreif, spezifisch und mit Findings/Dependencies verknüpft sein.
- Nach Slice-Abschluss: Fertigstellung prüfen, Issues schließen und nachtragen, Canvas aktualisieren, Folgeprompt erzeugen; Commit/Push wenn gewünscht.

## Learned Workspace Facts

- `vzctl` ist ein CLI-first DevStack-/Hypervisor-Projekt auf Apple Virtualization.framework (Environments as Code für macOS-VMs).
- Privates GitHub-Repo unter `frankhildebrandt`; Planungsdokumente liegen unter `docs/planing/` (Plan, Reviews, Decision Log, Canvas).
- Mindest-macOS-Version ist 26; Pre-26 ist unsupported.
- Prozessmodell: Supervisor plus ein Helper-Prozess pro VM (Helper owns `VZVirtualMachine`); Supervisor owns vmnet-Refs und DNS, Helper nur Attachment-Handles; Guest-Control primär per vsock-Agent.
- Repo-Layout: Rust-CLI unter `crates/vzctl`, Swift-Daemon unter `daemon/` (`vz-supervisor`, `vz-helper`); Runtime-State unter `~/Library/Application Support/vzctl/` (UDS + SQLite).
- Internes DNS stellt der Hypervisor bereit (Zone `{vm}.{net}.{project}.vz.test`); Host-Listener `127.0.0.1:15353`, macOS löst über `/etc/resolver` auf; Guest-Nameserver ist Bridge `.0`; auf Custom-vmnet ist Host-Gateway/DNS `.0` (nicht `.1`), Router typisch `.2`, Guests `.10+`; Cross-Net nur über Router-VM.
- Stacks sind deklarativ per Verzeichnis/Git-Repo und `hypernetwork.config.yaml` (`apiVersion: hypernetwork/v1`, Spec `docs/specs/hypernetwork-v1.md`, `vzctl validate`) steuerbar (up/down/apply); VMs ohne explizites Netz landen im konfigurierbaren Default-Netz (shared, voller Egress/NAT).
- VMs teilen ein sealed Base-Image (APFS linked clone / clonefile + pro-VM dataDisk); Identity (machine-id, NIC/MAC, SSH-Host-Keys, cloud-init instance-id) wird pro Clone neu gesetzt; Base nie writable öffnen.
- CLI-Contract: `docs/specs/cli-contract-v1.md` (JSON-Envelope, stdout=Daten / stderr=Diagnostics, stabile Exitcodes).
- Guest-Agent-Wire-Contract: `docs/specs/guest-agent-v1.md` (virtio-vsock, length-prefixed JSON, Token-Auth).
- Base-/Image-Pull zielt auf ARM64 Cloud/Server-Images (nicht Installer-ISOs); Aliases u. a. `ubuntu|debian|alpine|arch|fedora|rocky|alma|opensuse|coreos|flatcar|photon|opensuse-microos|talos-latest` (Contract `docs/images/pull-contract-v1.md`); Workflow `pull → bake → seal`; Bake/Seal nutzen lokales `virt-customize` oder die gepinnte Builder-VM-Appliance (`scripts/build-builder-appliance.sh`, Cache unter `images/builder/`).
- G0-Netzwerk-/Entitlement-Spike ist Go (vor P0 abgeschlossen); nur Entitlement `com.apple.security.virtualization` — `com.apple.vm.networking` bei ad-hoc codesign → SIGKILL; Ingress/OIDC/CA gehören zu v0.2, v0.1 ist Alpha.
