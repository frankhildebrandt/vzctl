# ADR 0003: Apply-Zustand — Journal, Idempotenz, Resume/Abort, Purge

- **Status:** Accepted
- **Date:** 2026-07-30
- **Issue:** [#35](https://github.com/frankhildebrandt/vzctl/issues/35)
- **Related:** [#34](https://github.com/frankhildebrandt/vzctl/issues/34) Stack Reconciler, ADR 0002 Ownership

## Context

Lease allein reicht nicht (SOL): parallele `apply`, Crash mitten im Step und Drift zwischen YAML / SQLite / Lockfile brauchen eine explizite Zustandsmaschine. Alpha muss resume/abort idempotent und destruktive Ops eng begrenzt halten.

## Decision

### Wahrheitsquellen (Priorität bei Konflikt)

| Rang | Quelle | Rolle |
|---|---|---|
| 1 | **Journal** (incomplete ops) | „was gerade passiert“ — blockiert neue apply bis resume/abort |
| 2 | **Desired** `hypernetwork.config.yaml` | Sollzustand (git) |
| 3 | **Actual** SQLite | bekannter Ist-Zustand (managed Ressourcen) |
| 4 | **Lockfile** / Instanz-Map | lokale Runtime (PIDs, helper sockets, net handles) |

`vzctl diff` zeigt Desired↔Actual. Lockfile-Orphans → `adopt` oder kill nach Policy.

### Journal-Schema

Jede apply-Op = eine Journal-Row (SQLite):

| Feld | Bedeutung |
|---|---|
| `id` | UUID |
| `stack_id` | Stack / Projekt |
| `generation` | monoton pro Stack (Desired-Hash oder Counter) |
| `step` | enum Step (siehe unten) |
| `status` | `pending` \| `running` \| `done` \| `failed` \| `aborted` |
| `payload` | JSON (IDs, CIDRs, paths) |
| `error` | optional Text |
| `created_at` / `updated_at` | UTC |

**Regel:** Es darf höchstens **eine** incomplete Op (`pending`/`running`/`failed` ohne abort) pro `stack_id` geben.

### Steps (Alpha, geordnet)

```
validate → acquire_lease → ensure_nets → ensure_dns → ensure_ca →
ensure_images → attach_nets → ensure_vms → start_helpers →
await_agents → await_cloud_init → ensure_guest_utils →
ensure_docker_project_mount → ensure_oidc → ensure_ingress →
ensure_ca_rollout → ensure_oidc_inject → ensure_docker_context →
ensure_containers → ensure_ports → apply_routes_policies →
release_lease → done
```

`down`: stop_helpers → detach → destroy_vms? → release_nets? → dns_cleanup?  
`down --purge`: zusätzlich Löschung nur `managed-by=vzctl` (siehe Purge).

### Idempotenz

- Jeder Step ist **retry-safe**: „ensure“ statt „create-or-die“.
- Re-entry mit gleichem `generation`+`step` darf keine doppelten Ressourcen anlegen (Actual-Lookup by name/id).
- Fehlschlag setzt `status=failed`, belässt Teil-Ist in Actual; kein automatisches Rollback außer dokumentierten Compensating Steps.

### Resume / Abort

| CLI | Semantik |
|---|---|
| `vzctl apply` | Wenn incomplete Journal → **Exit 5**, Hinweis auf `--resume`/`--abort` |
| `vzctl apply --resume` | Setzt `running` am failed/pending Step fort; gleiche `generation` |
| `vzctl apply --abort` | Markiert Op `aborted`; Compensating: Lease frei, keine stillen Deletes; Drift bleibt sichtbar |
| Desired geändert während incomplete | `--abort` nötig oder explizit `--resume --force-generation` (Alpha: nur abort + neu) |

### Parallelität / Lease

- Lease-Datei/Row: `stack_id`, `holder` (host+pid), `expires_at`.
- Zweiter `apply` bei aktivem Lease → **Exit 6**.
- Holder tot (PID weg) → `doctor`/`apply --resume` darf Lease stehlen nach Grace (z. B. 30s).
- Journal incomplete ohne Lease → Lease wird bei resume neu acquired.
- TTL: 300 Sekunden. Der CLI-Owner erneuert alle 30 Sekunden über `stack.heartbeat`
  (`id`, `stack_id`, `holder`), unabhängig von langen Pull-/Bake-/Guest-Schritten.
  Renewal ändert keinen Checkpoint. Fremde, abgelaufene oder einem anderen Journal
  zugehörige Leases dürfen weder erneuert noch per `stack.finish` abgeschlossen werden.
- Scheitert ein Heartbeat, stoppt die CLI spätestens an der nächsten Step-Grenze mit
  Exit 6. Bereits laufende externe Operationen werden dadurch nicht zurückgerollt.
  CLI und Supervisor müssen gemeinsam auf eine Version mit Heartbeat-Unterstützung
  aktualisiert werden; eine alte Gegenstelle wird vor dem ersten Step erkannt.
- Der Resume-Hash umfasst das ganze normalisierte Config-Dokument und Fingerprints
  von cloud-init-, Compose- und zugehörigen lokalen env/config/secret/include-Dateien
  sowie den verwendeten OIDC-Dateien. Inhalte und Secrets gelangen nicht in den
  Journal-Hash-Payload oder die Ressourcenlabels. Veränderungen benötigen Abort +
  neues Apply. Bereits vor dieser Erweiterung angelegte Journale ebenso.
- Gemountete Nutzdaten und veränderliche Remote-Image-Tags werden nicht als Dateien
  gehasht. Dynamische Compose-Interpolation und Build-Kontexte sind keine vollständig
  hermetisch eingefrorenen Inputs; diese müssen für reproduzierbares Resume gepinnt bleiben.

### Drift

- `vzctl diff`: Desired vs Actual (VMs, nets, DNS zone, routes/policies, ports, volumes, certs, ingress, OIDC, resilience, observability und referenzierte Input-Dateien).
- Recreate nur mit `--force` (destruktiv markiert).
- Lockfile-only Drift (zombie helper) → `vzctl adopt` oder `vzctl doctor --fix-locks`.

### Purge-Regeln (`down --purge`)

Löscht **nur** Ressourcen mit Label/Attribut `managed-by=vzctl` **und** `project=<id>`.
Helper werden hart gestoppt (`SIGKILL` / `vm delete --force`); kein graceful Shutdown.

| Ressource | Purge |
|---|---|
| Linked-Clone Disks + dataDisk | ja |
| Base-Image | **nein** |
| vmnet (Supervisor-Registry) | ja, inkl. Ref-Release |
| `/etc/resolver/<project>.vz.test` | ja |
| Docker context `vzctl-*` | ja |
| Fremde VMs / Bridges / Resolver | **nein** |

Ohne `--purge`: graceful stoppen + Actual „stopped“, Disks behalten.

### State machine

```text
                    ┌──────────────┐
         apply      │   idle       │
       ───────────► │ (no journal) │
                    └──────┬───────┘
                           │ write journal pending
                           ▼
                    ┌──────────────┐
              ┌────►│   running    │◄──── resume
              │     └──────┬───────┘
              │            │
         step ok           ├──── step fail ──► failed ──► resume|abort
              │            │
              │            ▼
              │     ┌──────────────┐
              └─────│ step done /  │
                    │ next step    │
                    └──────┬───────┘
                           │ all steps done
                           ▼
                    ┌──────────────┐
                    │   committed  │  (journal done, lease free)
                    └──────────────┘

    abort from running|failed → aborted → idle (lease free, drift possible)
```

### Exitcodes (Alpha-Vertrag)

| Code | Bedeutung |
|---|---|
| 0 | ok |
| 2 | usage / validation |
| 5 | incomplete journal (resume/abort nötig) |
| 6 | lease held |
| 10+ | step failures (doctor klassifiziert) |

## Consequences

- Reconciler (#37) implementiert Steps strikt nach dieser Spec.
- CLI muss Exit 5/6 stabil halten (Automation/Agenten).
- Purge ist bewusst eng — verhindert „rm -rf Host“.
- G0: Net-Refs bei abort/down immer releasen (ADR 0002), sonst CIDR-Leak.

## Alternatives verworfen

- **Nur File-Lease:** kein Crash-Resume-Punkt.
- **Automatisches Rollback aller Steps:** zu riskant für VMs/Disks in Alpha.
- **Purge ohne managed-by:** inakzeptabel.
