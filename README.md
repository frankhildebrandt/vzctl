# vzctl

macOS Virtualization.framework–based **devstack supervisor**: Git-native multi-VM environments, custom networks, Hypervisor-DNS, linked clones, Docker context.

> Status: planning / pre-scaffold. See [`docs/planing/`](docs/planing/).

## Docs

| Document | Description |
|---|---|
| [Planning index](docs/planing/README.md) | Übersicht aller Planungsartefakte |
| [Implementation plan](docs/planing/01-implementation-plan.md) | Aktueller Plan (Fable + SOL Must-Fixes) |
| [Fable review](docs/planing/02-fable-review.md) | Architektur-Review (Claude Fable 5 High) |
| [GPT SOL review](docs/planing/05-gpt-sol-review.md) | Follow-up-Review (GPT 5.6 SOL Medium) |
| [Decision log](docs/planing/04-decision-log.md) | Festgezogene Architektur-Entscheidungen |
| [UTM / Multipass / HyperKit](docs/planing/03-utm-multipass-hyperkit-vergleich.md) | Vorab-Vergleich |
| [Canvases](docs/planing/canvases/) | Original Cursor Canvas sources |

## Working title

CLI: `vzctl` · Supervisor daemon · VM helper (1 process / VM) · vsock guest agent

## License

Private repository. All rights reserved.
