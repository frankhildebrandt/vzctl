---
title: Quickstart
description: In wenigen Schritten den Referenz-Stack edge-dmz validieren und starten.
---

Ziel: Installation prüfen und den Beispiel-Stack `examples/edge-dmz` anfassen.

## 1. Installieren

```bash
make install
export PATH="$HOME/.local/bin:$PATH"
vzctl doctor
```

## 2. Config validieren

Offline, ohne Supervisor-Mutation:

```bash
vzctl validate -C ./examples/edge-dmz
```

Optional Plan/Diff (Supervisor muss laufen, ändert aber nichts):

```bash
vzctl plan -C ./examples/edge-dmz
vzctl diff -C ./examples/edge-dmz
```

## 3. Image und Stack

```bash
vzctl image pull ubuntu-latest
vzctl up -C ./examples/edge-dmz
```

`up` bringt Desired State und Runtime zusammen (Images, Netze, VMs, Apply-Pipeline).

Nützliche Checks:

```bash
vzctl vm list
vzctl docker -- ps
vzctl port list
vzctl dns status
```

## 4. Stoppen

```bash
vzctl down -C ./examples/edge-dmz
```

Ressourcen inkl. Docker-Context und Port-Forwards hart entfernen:

```bash
vzctl down -C ./examples/edge-dmz --purge
```

`--purge` stoppt/löscht VMs hart (SIGKILL) — Datenverlust ist akzeptiert.
Normales `down` bleibt graceful.

## Weiterlesen

- [Hypernetwork](/vzctl/docs/guides/hypernetwork/)
- [Beispiel edge-dmz](/vzctl/docs/reference/example-edge-dmz/)
