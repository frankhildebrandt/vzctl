## Learned User Preferences

- Antworten immer auf einfachem, kurzem Deutsch; nicht ausschweifen.
- Analysen, Vergleiche und Implementationspläne bevorzugt als Canvas aufbereiten und bei Plan-Änderungen mitaktualisieren.
- Plan-Reviews mit explizit genanntem Modell (z. B. Fable, GPT-SOL) als Subagent; ohne Angabe Parent-/Auto-Modell erben, kein stiller Override.
- GitHub-Issues für vzctl sollen implementationsreif, spezifisch und mit Findings/Dependencies verknüpft sein.
- Nach abgeschlossenen Implementierungs-Slices: Fertigstellung prüfen, passende GitHub-Issues schließen und Folgeprompt für den nächsten Slice erzeugen.

## Learned Workspace Facts

- `vzctl` ist ein CLI-first DevStack-/Hypervisor-Projekt auf Apple Virtualization.framework (Environments as Code für macOS-VMs).
- Privates GitHub-Repo unter `frankhildebrandt`; Planungsdokumente liegen unter `docs/planing/` (Plan, Reviews, Decision Log, Canvas).
- Mindest-macOS-Version ist 26; Pre-26 ist unsupported.
- Prozessmodell: Supervisor plus ein Helper-Prozess pro VM (Helper owns `VZVirtualMachine`); Guest-Control primär per vsock-Agent.
- Repo-Layout: Rust-CLI unter `crates/vzctl`, Swift-Daemon unter `daemon/` (`vz-supervisor`, `vz-helper`); Runtime-State unter `~/Library/Application Support/vzctl/` (UDS + SQLite).
- Internes DNS stellt der Hypervisor bereit (Zone `{vm}.{net}.{project}.vz.test`); macOS löst über `/etc/resolver` auf; auf Custom-vmnet ist Host-Gateway/DNS `.0` (nicht `.1`), Router typisch `.2`, Guests `.10+`.
- Stacks sind deklarativ per Verzeichnis/Git-Repo und `hypernetwork.config.yaml` steuerbar (up/down/apply).
- VMs teilen ein sealed Base-/Snapshot-Image; Diff-/Data-Disks sind pro VM, Identity (Machine-ID, NIC/MAC) wird automatisch neu gesetzt.
- Guest-Agent-Wire-Contract: `docs/specs/guest-agent-v1.md` (virtio-vsock, length-prefixed JSON, Token-Auth).
- G0-Netzwerk-/Entitlement-Spike ist Go (vor P0 abgeschlossen); nur Entitlement `com.apple.security.virtualization` — `com.apple.vm.networking` bei ad-hoc codesign → SIGKILL; Ingress/OIDC/CA gehören zu v0.2, v0.1 ist Alpha.
