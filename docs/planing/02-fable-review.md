# Fable Review: vzctl Implementationsplan

> Reviewer: Claude Fable 5 High (Thinking)
> Datum: 2026-07-30
> Agent: [d5e51cc5-5849-42f6-8a0e-4ca03403aa02](https://cursor.com)
> Kontext: Review des VZ-Devstack-Plans + UTM/Multipass/HyperKit-Vergleich

Die Must-Fixes aus dieser Review sind im [Decision Log](04-decision-log.md) und im [Implementation Plan](01-implementation-plan.md) übernommen.

---

# Review: vzctl Implementationsplan (VZ Devstack Supervisor)

## 1. Gesamturteil & Scores

Der Plan ist ungewöhnlich reif für ein Pre-Scaffolding-Dokument: klare Schichtung (Daemon besitzt VZ, Reconciler besitzt Compose-Semantik, Git besitzt Desired State), Labels/Instanzen für PR-Labs sind durchdacht, die Identity-Reset-Tabelle ist besser als das, was Lima/Multipass dokumentieren. Aber: Er ist auf der Netzwerk- und Guest-Kommunikationsebene zu dünn, genau dort, wo VZ-Projekte in der Praxis sterben — und der MVP-Scope ist um ~40 % zu fett.

| Dimension | Score | Kommentar |
|---|---|---|
| Architektur-Soundness | 4/5 | Schichtung stimmt; Prozessmodell und Guest-Agent fehlen komplett |
| Sequencing | 3/5 | P4 (Clones/Identity) nach P3 (Stacks) erzeugt Rework; CA/OIDC zu früh im Muss |
| MVP-Realismus | 2/5 | 14–16 Wochen für Reconciler + Clones + Ingress + CA-Rollout + OIDC ist Solo unrealistisch; eher 2x |
| Risiko-Ehrlichkeit | 3/5 | macOS-26-vmnet-Abhängigkeit ist benannt, aber die Fallback-Semantik ist handgewedelt |
| DX produktiver Alltag | 3/5 | up/down/apply/diff-UX top; Mounts, Sleep/Clock, Logs, Locking unbehandelt |
| Agentic/Scriptability | 4/5 | JSON-first, plan/dry-run, Event-Stream — gut; Event-Schema fehlt noch |

## 2. Die drei größten fachlichen Löcher

### a) Kein Prozessmodell für VZVirtualMachine — kritischste offene Frage
Der Plan sagt "Swift-Daemon besitzt VZ-Refs". Wenn ein monolithischer Daemon alle `VZVirtualMachine`-Instanzen hält, gilt: Daemon-Crash oder Daemon-Update = alle VMs aller Stacks tot. Das widerspricht direkt dem "Zuverlässigkeit im Sprint"-Kriterium aus eurem eigenen Vergleichscanvas. Tart und Lima laufen faktisch pro VM als eigener Prozess. Empfehlung: schlanker Supervisor-Daemon + ein Helper-Prozess pro VM (launchd-spawned), RPC via UDS/XPC. Das muss in P0 entschieden werden, nicht in P7 — es bestimmt Persistenzmodell, Upgrades und Entitlement-Verteilung.

### b) `*.localhost` funktioniert in den Guests nicht — Designfehler
Der Plan verspricht: Guests vertrauen der CA und nutzen `https://auth.localhost` (OIDC-Issuer, Inter-VM-Calls). Aber `auth.localhost` resolved **im Guest** auf dessen eigenes Loopback, nicht auf den Host-Ingress. RFC 6761 gilt überall, gerade deshalb bricht es. Damit ist die gesamte OIDC-Autoconfig-Kette (Issuer-URL in Token = Discovery-URL im Guest) so wie beschrieben kaputt. Optionen:
- Eigener Stack-DNS (dnsmasq auf der Router-VM oder DNS im Daemon) mit Domain wie `*.edge-dmz.internal`, `*.localhost` nur host-seitig als Alias, oder
- systemd-resolved/hosts-Injection pro Guest: `*.localhost → Gateway-IP` (fragil), oder
- Split: Host nutzt `*.localhost`, Guests nutzen den internen Namen; OIDC-Issuer muss dann von beiden Seiten identisch erreichbar sein — spricht stark für die interne Domain als kanonischen Issuer.
Zusatz: Auch am **Host** resolven nicht alle Tools `foo.localhost` (Browser ja, `getaddrinfo`/curl je nach macOS-Version nicht garantiert) — `/etc/resolver/` einplanen.

### c) Kein Guest-Agent — SSH allein trägt die Featureliste nicht
`vzctl exec`, CA-Rollout "per SSH/exec", IP-Discovery, `logs`, Health, Docker-Context: alles hängt implizit an SSH + DHCP-Lease-Raterei. Robust ist ein Mini-Guest-Agent über **vsock** (`VZVirtioSocketDevice`): exec, IP/Health-Report, Time-Sync nach Host-Sleep (klassisches VZ-Problem, im Plan unerwähnt), CA-Injection ohne SSH-Bootstrap-Henne-Ei. Der Agent kann per cloud-init installiert werden. Das gehört als eigenes Deliverable in P0/P1, nicht als implizite Annahme.

## 3. Weitere Kritikpunkte (konkret)

- **Netzwerk-Isolation vor macOS 26 ist unbewiesen.** "shared" vmnet pre-26: eigene Subnetze + DHCP-Kontrolle brauchen `vmnet` Host-Mode mit Custom-Range → Root oder Entitlement. Und: `dhcp: true` im Schema kollidiert mit statischen IPs via cloud-init network-config, wenn vmnet selbst DHCP macht. Wer vergibt die IP wirklich? Das Schema tut so, als sei das trivial — es ist der schwierigste Teil von P2. Bridged Networking (`mode: bridged`) braucht zudem das restriktive `com.apple.vm.networking`-Entitlement (Apple-Approval) — als Risiko nirgends gelistet.
- **Zwei Reconciler-Clients, kein Locking.** Reconciler lebt in CLI *und* Tauri; zwei gleichzeitige `apply` auf denselben Stack sind undefiniert. `stack.apply` im Daemon braucht Lease/Lock pro Stack-Instanz. Generell: Desired State (YAML) + Lockfile (Repo) + SQLite (Daemon) sind drei Wahrheiten — definieren, welche bei Konflikt gewinnt, und `vzctl adopt` für verwaiste Instanzen (Default-Instance = Pfad-Hash → Repo verschieben orphaned den Stack).
- **`mounts:` ohne Implementierungsaussage.** virtiofs ist der einzige realistische Pfad, hat aber bekannte Perf-/Kohärenz-Grenzen — und Feedback-Loop war im eigenen Vergleich das Top-Kriterium. Braucht ein eigenes Deliverable inkl. Perf-Messung, sonst verliert vzctl im Alltag gegen Multipass/OrbStack genau dort, wo es wehtut.
- **Eigenen OIDC-Provider schreiben ist eine Scope-Falle.** Auth-Code+PKCE+Discovery+JWKS+UserInfo+Login-UI ist ein eigenes Produkt. Dex (oder rauthy) einbetten, `vzctl` macht nur Autoconfig/Client-Provisionierung. Gleiche Frage beim Ingress: gebündeltes Caddy löst Proxy + Cert-Mint + Reload in einem, statt Hyper+rustls selbst zu bauen. Beides sind Build-vs-Embed-Entscheidungen, die vor P5 fallen müssen.
- **Host-Realität fehlt:** Verhalten bei Host-Sleep (Clock-Drift → TLS/Token-Validierung bricht!), Host-Reboot (Daemon muss "should-be-running" reconcilen), RAM-Budget über alle Stacks (Ballooning?). Ironisch: Der Vergleichscanvas nennt "Sleep, VPN, MDM" explizit als Self-made-Schwäche — der Plan adressiert sie nicht.
- **Sequencing:** Base/Seal/Clone (P4) nach dem Reconciler (P3) heißt: P3 wird gegen Wegwerf-Disk-Handling gebaut. Minimal-Seal + clonefile in P1 vorziehen; `image pull` steht ohnehin schon in P1.
- **Docker:** `socketForward: 2375` unverschlüsselt auch loopback-only nur mit lautem Warning; SSH-Context als einziger Default ist richtig. BuildKit/Registry-Cache fehlt.

## 4. Gaps für einen "super" Devstack-Supervisor

- **Snapshots/Restore ganzer Stacks** (`vzctl snapshot create/restore`) — für Labs der Killer-Feature-Kandidat Nr. 1, per APFS-Clones billig zu haben (bei gestoppten VMs).
- **Interner DNS-/Service-Discovery-Layer** (`web.dmz.internal`) — folgt direkt aus Loch 2b.
- **`vzctl events` / strukturierter Event-Stream** — für Agents wichtiger als die Tauri-UI; Event-Schema versionieren.
- **Log-Aggregation** (`vzctl logs` stackweit, nicht nur pro VM) und minimale Metriken (CPU/RAM pro VM).
- **k3s-Rolle** analog `roles: [docker]` — der eigene Vergleich gewichtet K8s hoch, der Plan erwähnt es nie.
- **Image-Lifecycle:** Checksums/Signaturverifikation der Cloud-Images, Cache-GC, Base-Upgrade-Policy (rebase vs recreate ist erwähnt, aber nicht entschieden).
- **Doctor ab P0**, nicht P7 — vmnet-Entitlements, Rosetta, APFS-Fähigkeit, Portkollisionen sind Setup-Blocker, die früh diagnostizierbar sein müssen.
- **CI-Story:** läuft `vzctl` auf GitHub-Actions-macOS-Runnern (Nested-Virt-Grenzen)? Wenn nein, ehrlich dokumentieren — dort sitzt Tart.

## 5. Verbesserungen priorisiert

**Must (vor Code):**
- Prozessmodell entscheiden: Helper-Prozess pro VM statt Monolith.
- vsock-Guest-Agent als P0/P1-Deliverable (exec, IP, Health, Time-Sync, CA-Inject).
- DNS-/Domain-Konzept fixen: interne Domain als kanonischer OIDC-Issuer, `*.localhost` nur als Host-Alias.
- IP-Vergabe-Modell pro vmnet-Mode spezifizieren (wer macht DHCP, wie erzwingt man statische IPs, was geht pre-macOS-26 wirklich) — als Spike in Woche 1, nicht als Annahme.
- MVP schneiden: OIDC + CA-Rollout + Ingress von "Muss v0.1" nach v0.2. v0.1 = Daemon, CLI, Netze+Router, Stacks, Clones+Identity, Ports, Docker-Context. Das ist allein schon ambitioniert.

**Should:**
- Dex + Caddy einbetten statt IdP/Proxy selbst schreiben.
- Stack-Locking im Daemon (`stack.apply`-Lease), `vzctl adopt`.
- virtiofs-Mounts mit Perf-Benchmark als eigenes Ticket.
- Sleep/Reboot-Semantik: Clock-Resync via Agent, Autostart-Reconcile beim Daemon-Boot.
- `doctor` und `events` vorziehen.

**Nice:**
- Stack-Snapshots, k3s-Rolle, Registry-Cache, MCP-Server-Interface über der RPC-Schicht, Java-Truststore-Rollout (aus v1 streichen, `stores: [java]` ist Randfall).

## 6. Positionierung

Nicht gegen OrbStack (Single-VM-Docker-DX, poliert) oder Multipass/Lima (eine unsichtbare Ubuntu-VM) antreten — dort verliert vzctl auf Jahre. Die unbesetzte Nische ist real: **"docker compose für VM-Topologien"** — mehrere VMs, echte Netze, Routing, Ingress, Identity, alles git-versioniert und agentensteuerbar. Zielgruppe: Platform-Engineers/Infra-Devs, die Ansible-Rollen, Netz-Setups und Multi-Service-Stacks lokal wie ein Mini-Rechenzentrum testen — plus Agent-Workflows, die eine deklarative, JSON-planbare VM-API brauchen. Tart besetzt CI/Image-Distribution; bewusst abgrenzen statt überlappen. Tagline-Richtung: "Environments as Code für macOS-VMs" — nicht "besseres Docker Desktop".

## 7. Top 5 Entscheidungen vor dem Scaffolding

1. **Prozessmodell:** Ein Helper-Prozess pro VM oder monolithischer Daemon? (Bestimmt Crash-Isolation, Upgrades, P0-Design — meine Empfehlung: pro VM.)
2. **Guest-Kommunikation:** vsock-Agent als First-Class-Komponente ja/nein? (Empfehlung: ja; SSH nur als Fallback.)
3. **Domain-/DNS-Schema:** Kanonische interne Domain für Guests + OIDC-Issuer; `*.localhost` nur host-seitig. Welche Domain, wer serviert DNS?
4. **Mindest-macOS + Netz-Fallback:** macOS 26 als Baseline (einfacher, weniger Code) oder ehrlicher Pre-26-Support mit definierter, getesteter Isolation-Semantik?
5. **Build vs Embed + MVP-Schnitt:** Dex/Caddy einbetten und P5 auf v0.2 schieben — oder am 14–16-Wochen-Voll-MVP festhalten (dann realistisch neu schätzen)?

I'm thinking through the technical architecture from a platform architect's perspective, focusing on the key design decisions. The split between the Swift daemon handling virtualization and the Rust reconciler as a shared CLI with Tauri makes sense structurally, but there's a concurrency issue—multiple clients could try to apply changes simultaneously, which means the daemon needs proper locking and plan serialization. There's also a potential drift problem between what the client-side lockfile tracks and what the daemon actually has.

Looking at the data layer, having three sources of truth across the lockfile, SQLite, and YAML configuration is problematic. The plan and apply logic should really live in the daemon, or at minimum the daemon needs to handle apply operations transactionally with leases to maintain consistency.

The VZVirtualMachine process model is a critical architectural gap. If all VMs run within a single daemon process, a daemon crash takes down every VM—that's a serious availability issue. Other tools like Tart and Lima run one process per VM for isolation and resilience. The plan doesn't address whether to go monolithic or per-VM helper processes, which affects crash isolation, per-VM entitlements, and resource management.

On entitlements, the virtualization framework entitlement is straightforward, but bridged networking via VZBridgedNetworkDeviceAttachment requires Apple's restricted com.apple.vm.networking entitlement, which isn't guaranteed. The plan mentions relying on custom vmnet and assumes macOS 26 APIs will be available—that's a risky dependency to build on.

For exec and serial access, the plan has a major blind spot: there's no guest agent mentioned, yet it implicitly assumes SSH will work. A vsock-based guest agent (like Lima or OrbStack use) is essential for robust exec, certificate rollout, health checks, and IP reporting. The plan talks about CA rollout via SSH but doesn't explain where SSH keys come from or how IP discovery works on shared networks with DHCP. That needs to be a must-have.

Virtiofs for mounts is mentioned in the config but the plan never discusses the actual implementation or its performance tradeoffs. Time sync after host sleep is a classic Virtualization.framework problem that's completely missing. And VM state management—save/restore on macOS 14+ for Linux VMs, or pause/resume for in-memory state—isn't addressed either.

The daemon also needs to handle host reboots gracefully by reconciling the "should-be-running" state against what actually exists.

Looking at the MVP timeline and scope, 14–16 weeks for ingress, CA, OIDC, Docker, clones, reconciler, and multi-network routing is unrealistic for a solo dev. The scope creep is real: embedding OIDC and CA rollout as must-haves inflates the MVP. Realistically, P0–P3 plus basic P4 features should be the MVP, with P5 as the differentiator for v0.2. The sequencing also has a problem—testing P3 (stacks) before P4 (identity/clones) means rework later. And base image handling needs to be sorted before cloning can work properly.

The macOS vmnet dependency is a blocker: the plan relies on an unreleased API, and the fallback of NAT with a router VM changes the network topology. On shared vmnet, all VMs see each other, so the claimed dmz/lan isolation needs explicit enforcement through separate vmnet interfaces with distinct subnets—doable with host mode and custom subnets but requires root/entitlements. The isolation strategy is still vague.

There's also a DHCP conflict: cloud-init network-config tries to set static IPs while vmnet's built-in DHCP hands out its own range. Need to pick one approach.

For state management, I'm using a gitignored lockfile in the repo plus SQLite in the daemon with labels. Instance identity defaults to a hash derived from the repo path, so moving the directory orphans the stack—needs documentation and an adopt command.

Secrets are stored plaintext in a gitignored JSON file for now, which works for dev but they want a Keychain-like store eventually. Docker forwarding uses unencrypted TCP 2375, even on loopback—should warn users about that, though the SSH context is safer. The host also needs the Docker CLI installed.

For ingress, I'm weighing Caddy against a custom Rust proxy with rustls and hyper. Caddy bundled is more pragmatic despite adding a Go binary. The tricky part is *.localhost resolution—browsers handle it per RFC 6761, but system resolvers on macOS don't guarantee it without /etc/resolver/localhost or local DNS. Inside guests, they need to resolve auth.localhost too, which complicates things.

The real issue: guests resolving auth.localhost will hit their own loopback instead of the host gateway IP. This breaks the plan—I need DNS injection via dnsmasq on a router VM or systemd-resolved entries mapping *.localhost to the host gateway, or switch to a different domain like *.project.internal. Same problem applies to OIDC token validation from guests if the issuer is https://auth.localhost. I'm considering embedded IdP options like Dex or Keycloak in a micro VM.

Beyond OIDC, there are several critical gaps for a full devstack supervisor: guest agent communication over vsock for execution and health checks, a file sharing strategy (virtiofs performance vs NFS or rsync alternatives), snapshot and restore capabilities for entire environments, resource budgeting across the stack with memory ballooning and host pressure handling, clock drift correction, and internal DNS resolution for VM-to-VM communication.

The platform also needs a Kubernetes story (likely k3s), centralized logging with event streaming, a Docker registry cache, image supply chain verification with garbage collection, daemon upgrades without stopping VMs, CLI concurrency locking to prevent simultaneous operations, CI integration testing (especially on GitHub Actions macOS runners where nested virtualization is limited), and a JSON-first agent interface with explicit MCP server support and machine-readable plan output.

I'm thinking through scoring dimensions and positioning strategy. DNS and domain naming for guests—whether to use *.localhost or *.lab.internal and how the host resolves those. Network fallback behavior on older macOS versions and what isolation guarantees dmz/lan modes actually provide. And the OIDC/ingress decision: whether to embed Dex and Caddy or write custom implementations, plus whether to cut MVP scope to push some features to v0.2. I should also note the MVP scope reduction as a key decision and present all five with a scoring table in German with headings and bullets, keeping it substantial but readable.
