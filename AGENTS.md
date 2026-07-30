## Learned User Preferences

- Antworten immer auf einfachem, kurzem Deutsch; nicht ausschweifen.
- Analysen, Vergleiche und Implementationspläne bevorzugt als Canvas aufbereiten und bei Plan-Änderungen mitaktualisieren.
- Plan-Reviews mit explizit genanntem Modell (z. B. Fable, GPT-SOL) als Subagent; ohne Angabe Parent-/Auto-Modell erben, kein stiller Override.
- GitHub-Issues für vzctl sollen implementationsreif, spezifisch und mit Findings/Dependencies verknüpft sein.

## Learned Workspace Facts

- `vzctl` ist ein CLI-first DevStack-/Hypervisor-Projekt auf Apple Virtualization.framework (Environments as Code für macOS-VMs).
- Privates GitHub-Repo unter `frankhildebrandt`; Planungsdokumente liegen unter `docs/planing/` (Plan, Reviews, Decision Log, Canvas).
- Mindest-macOS-Version ist 26; Pre-26 ist unsupported.
- Prozessmodell: Supervisor plus ein Helper-Prozess pro VM; Guest-Control primär per vsock-Agent.
- Internes DNS stellt der Hypervisor bereit (Zone `{vm}.{net}.{project}.vz.test`); macOS löst über `/etc/resolver` auf.
- Stacks sind deklarativ per Verzeichnis/Git-Repo und `hypernetwork.config.yaml` steuerbar (up/down/apply).
- VMs teilen ein sealed Base-/Snapshot-Image; Diff-/Data-Disks sind pro VM, Identity (Machine-ID, NIC/MAC) wird automatisch neu gesetzt.
- Netzwerk-/Entitlement-Spike (G0) ist Go/No-Go vor P0-Scaffolding; Ingress/OIDC/CA gehören zu v0.2, v0.1 ist Alpha.
