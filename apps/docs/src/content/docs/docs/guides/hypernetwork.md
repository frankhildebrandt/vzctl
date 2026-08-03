---
title: Hypernetwork
description: Deklarativer Desired State mit hypernetwork.config.yaml.
---

`hypernetwork/v1` ist der Desired State eines vzctl-Stacks. Die Datei heißt
üblicherweise `hypernetwork.config.yaml` und liegt im Stack-Verzeichnis (oder Git-Repo).

## Pflichtstruktur

- `apiVersion: hypernetwork/v1`, `kind: Environment`
- `metadata.name`
- `spec.project`, `spec.domain` mit Suffix `.vz.test`
- `spec.dns`, `images`, `networks`, `routes`, `policies`, `vms`

Minimalbeispiel:

```yaml
apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: demo
spec:
  project: demo
  domain: demo.vz.test
  dns: {}
  images:
    ubuntu:
      from: ubuntu-latest
      role: base
      tag: v1
  networks:
    lan:
      cidr: 10.90.0.0/24
      mode: shared
  routes: []
  policies: {}
  vms:
    web:
      from: ubuntu
      dataDisk: 4Gi
      networks:
        - name: lan
          ip: 10.90.0.10
```

## Lifecycle

| Befehl | Wirkung |
| --- | --- |
| `validate` | Schema + Semantik offline |
| `plan` / `diff` | Desired vs. Actual (ohne Mutation) |
| `up` / `apply` | Create/Reconcile |
| `down` | Stop / Teardown (graceful) |
| `down --purge` | harte Entfernung |
| `adopt` | report-only (stale Locks melden) |

Unvollständige Apply-Journals: `--resume` oder `--abort`.

## Wichtige Felder

- **Images:** `from` (Pull-Alias), `role: base`, `tag` pinnt sealed Artefakte
- **VMs:** `from`, `dataDisk`, mindestens ein Netz mit `name` + `ip`
- **Optional:** `cpus`, `memory`, `cloudInit`, `dependsOn`, `roles` (`router`, `docker`)
- **Policies:** bei mehreren passenden Routern `policies.*.via` setzen

VMs ohne explizites Netz landen im konfigurierbaren Default-Netz (shared, NAT-Egress).

Topology- und Projekt-Edits in der UI schreiben immer diese Config — sie ist Source of Truth.

Vollständige Spec: `docs/specs/hypernetwork-v1.md` im Repository.
