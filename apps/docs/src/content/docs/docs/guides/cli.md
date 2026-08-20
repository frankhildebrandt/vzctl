---
title: CLI
description: Überblick über vzctl-Befehle und den JSON-Contract.
---

Contract: `docs/specs/cli-contract-v1.md` — JSON-Envelope, stdout = Daten,
stderr = Diagnostics, stabile Exitcodes (`vzctl help exit-codes`).

Hilfe: `vzctl help`, `vzctl <command> help` (z. B. `vzctl net help`).

## Stacks

```bash
vzctl validate -C ./examples/edge-dmz
vzctl plan -C ./examples/edge-dmz
vzctl diff -C ./examples/edge-dmz
vzctl up -C ./examples/edge-dmz
vzctl apply -C ./examples/edge-dmz
vzctl down -C ./examples/edge-dmz
vzctl adopt -C ./examples/edge-dmz
```

## VMs

```bash
vzctl vm list
vzctl vm start <id>
vzctl vm stop <id>
vzctl vm delete <id>
vzctl vm ps
vzctl vm inspect <id>
vzctl vm exec <id> -- <cmd>
vzctl vm attach <id>
vzctl vm logs <id>
vzctl vm mount|unmount|mounts …
```

Runtime-IDs aus Stacks: `{project}/{vm}` (in REST-Pfaden encoded lassen).

Interaktives `vm exec -it` braucht Capability `exec_tty`. Detach am TTY:
**Ctrl-P Ctrl-Q**.

## Images, DNS, Docker, Ports

```bash
vzctl image list|pull|bake|seal
vzctl dns status|query|install-resolver|uninstall-resolver|install-bind-helper
vzctl docker -- ps|inspect|start|stop|restart|run
vzctl port list
vzctl oidc status
vzctl doctor
vzctl events subscribe --filter 'vm.*,apply.*'
```

Host-Port-Forwards lauschen auf `127.0.0.1`.

## Agent-Skill

```bash
vzctl skill                     # Skill + Anhänge auf stdout
vzctl skill --install-local     # ./.agents/skills/vzctl
vzctl skill --install-global    # ~/.agents/skills/vzctl
```

Der Skill beschreibt CLI und `hypernetwork.config.yaml`, damit ein LLM einen
Hypercontainer erzeugen kann. Ohne Flags ist stdout der komplette Skill
(inkl. YAML-Referenz, CLI-Cheat-Sheet und Minimalbeispiel).
