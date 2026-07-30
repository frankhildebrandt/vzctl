#!/usr/bin/env python3
"""Create vzctl GitHub epics/stories and wire sub-issues + blocked-by deps."""

from __future__ import annotations

import json
import subprocess
import textwrap
import time
from dataclasses import dataclass, field

OWNER = "frankhildebrandt"
REPO = "vzctl"


def run(args: list[str], input_text: str | None = None) -> str:
    r = subprocess.run(
        args,
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if r.returncode != 0:
        raise RuntimeError(f"cmd failed: {args}\n{r.stderr}\n{r.stdout}")
    return r.stdout.strip()


def gh_api(method: str, path: str, fields: dict | None = None) -> dict | list:
    args = ["gh", "api", "-X", method, path]
    if fields:
        for k, v in fields.items():
            if isinstance(v, (dict, list)):
                args += ["-f", f"{k}={json.dumps(v)}"]
            else:
                args += ["-f", f"{k}={v}"]
    return json.loads(run(args) or "null")


MILESTONE_TITLE = {
    1: "G0 — Spike Gate",
    2: "P0 — Foundation",
    3: "P1 — CLI + Clones",
    4: "P2 — Net + DNS",
    5: "P3 — Stacks",
    6: "P4 — Docker + Ports",
    7: "v0.1.x — Polish",
    8: "v0.2 — Ingress/OIDC/CA",
}


def create_issue(
    title: str,
    body: str,
    labels: list[str],
    milestone: int,
) -> int:
    args = [
        "gh",
        "issue",
        "create",
        "--repo",
        f"{OWNER}/{REPO}",
        "--title",
        title,
        "--body",
        body.strip() + "\n",
        "--milestone",
        MILESTONE_TITLE[milestone],
    ]
    for lab in labels:
        args += ["--label", lab]
    out = run(args)
    # URL ends with /number
    num = int(out.rstrip("/").split("/")[-1])
    print(f"  #{num} {title}")
    time.sleep(0.35)
    return num


def node_id(number: int) -> str:
    q = """
    query($o:String!,$n:String!,$num:Int!) {
      repository(owner:$o,name:$n) {
        issue(number:$num) { id }
      }
    }
    """
    raw = run(
        [
            "gh",
            "api",
            "graphql",
            "-f",
            f"query={q}",
            "-F",
            f"o={OWNER}",
            "-F",
            f"n={REPO}",
            "-F",
            f"num={number}",
        ]
    )
    return json.loads(raw)["data"]["repository"]["issue"]["id"]


def add_sub_issue(parent: int, child: int) -> None:
    q = """
    mutation($parent:ID!,$child:ID!) {
      addSubIssue(input:{issueId:$parent,subIssueId:$child}) {
        issue { number }
      }
    }
    """
    run(
        [
            "gh",
            "api",
            "graphql",
            "-f",
            f"query={q}",
            "-F",
            f"parent={node_id(parent)}",
            "-F",
            f"child={node_id(child)}",
        ]
    )
    time.sleep(0.2)


def add_blocked_by(issue: int, blocker: int) -> None:
    """Mark `issue` as blocked by `blocker` (blocker must finish first)."""
    q = """
    mutation($issue:ID!,$blocking:ID!) {
      addBlockedBy(input:{issueId:$issue,blockingIssueId:$blocking}) {
        issue { number }
      }
    }
    """
    try:
        run(
            [
                "gh",
                "api",
                "graphql",
                "-f",
                f"query={q}",
                "-F",
                f"issue={node_id(issue)}",
                "-F",
                f"blocking={node_id(blocker)}",
            ]
        )
    except RuntimeError as e:
        print(f"  warn blockedBy #{issue}<-#{blocker}: {e}")
    time.sleep(0.2)


DOC = "https://github.com/frankhildebrandt/vzctl/blob/main/docs/planing"


def refs(*names: str) -> str:
    lines = ["## Quellen"]
    for n in names:
        lines.append(f"- [{n}]({DOC}/{n})")
    return "\n".join(lines)


@dataclass
class Story:
    key: str
    title: str
    body: str
    labels: list[str]
    milestone: int
    blocked_by_keys: list[str] = field(default_factory=list)
    number: int | None = None


@dataclass
class Epic:
    key: str
    title: str
    body: str
    labels: list[str]
    milestone: int
    stories: list[Story]
    blocked_by_keys: list[str] = field(default_factory=list)
    number: int | None = None


# Milestone numbers from earlier create
G0, P0, P1, P2, P3, P4, V01X, V02 = 1, 2, 3, 4, 5, 6, 7, 8

EPICS: list[Epic] = [
    Epic(
        key="g0",
        title="Epic: G0 Netzwerk-/DNS-/Crash-Spike (Go/No-Go)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Vertikaler Spike **vor** jedem Scaffolding: zwei Netze, Router, feste IP, Host↔Guest,
            Dual-DNS-Probe, Sleep, Supervisor-Crash. Ergebnis = Go/No-Go + Baseline-Entscheidung.

            ## Exit-Kriterien
            - [ ] Zwei vmnet shared Netze mit VMs die sich (ggf. via Router) erreichen
            - [ ] Feste Guest-IP reproduzierbar (cloud-init static)
            - [ ] Host kann Guest erreichen und umgekehrt
            - [ ] DNS-Probe: Guest-Listener-IP identifiziert (nicht nur 127.0.0.1)
            - [ ] Sleep/Wake: Clock-Drift beobachtet/dokumentiert
            - [ ] Supervisor-Kill: Verhalten von VM/vmnet/DNS dokumentiert
            - [ ] ADR-Entwurf: macOS 26-only vs Pre-26

            ## Abbruch
            Isolation/Entitlements unmöglich → Stopp, Plan anpassen.

            {refs('01-implementation-plan.md','05-gpt-sol-review.md','04-decision-log.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:g0", "area:network", "finding:sol"],
        milestone=G0,
        stories=[
            Story(
                "g0-baseline",
                "ADR: macOS-Baseline (Empfehlung 26-only) + Bridged out-of-scope",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #14/#15)
                    Mindest-macOS und Bridged bestimmen P0-Architektur. Bridged braucht
                    `com.apple.vm.networking` (Apple Approval) → v0.1 out of scope.

                    ## Deliverable
                    - [ ] `docs/adr/0001-macos-baseline.md`
                    - [ ] Entscheidung: **26-only** oder exakter Pre-26-Modus
                    - [ ] Explizit: kein stiller Fallback zwischen Modi

                    {refs('04-decision-log.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:adr", "priority:p0", "phase:g0", "area:docs", "finding:sol"],
                G0,
            ),
            Story(
                "g0-vmnet-two-nets",
                "Spike: zwei vmnet shared Netze + Router-VM Erreichbarkeit",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable Netz / SOL #3)
                    Netzwerkisolation und vmnet Custom-Topologien sind der größte Blocker.

                    ## Tasks
                    - [ ] Zwei Netze (z.B. 10.80/24, 10.90/24) mit VZ + vmnet (macOS 26 APIs)
                    - [ ] Router-VM mit zwei NICs, IP-Forwarding
                    - [ ] Ping/HTTP Cross-Net verifizieren
                    - [ ] Entitlements/`vmnet` Privilegien dokumentieren

                    ## Risiken
                    Pre-26 shared mode ≠ Custom Range; Root/Entitlement nötig?

                    {refs('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:spike", "priority:p0", "phase:g0", "area:network", "finding:sol", "finding:fable"],
                G0,
            ),
            Story(
                "g0-ip-gateway-convention",
                "Spike: IP-Precedence + Gateway-/Router-IP-Konvention",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #16/#17, Decision Log offen)
                    `dhcp: true` + static `ip:` kollidieren semantisch. Router auf `.1` kann mit
                    vmnet-Gateway kollidieren.

                    ## Tasks
                    - [ ] Precedence festlegen: **cloud-init static Primär**
                    - [ ] Gateway-IP vs Router-IP messen (wer ist `.1`?)
                    - [ ] Konvention vorschlagen (z.B. Gateway `.1`, Router `.2`)
                    - [ ] Schema-Regeln: kein wildes DHCP+static Mix

                    {refs('04-decision-log.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:spike", "priority:p0", "phase:g0", "area:network", "finding:sol"],
                G0,
                blocked_by_keys=["g0-vmnet-two-nets"],
            ),
            Story(
                "g0-dns-listener-probe",
                "Spike: Guest-erreichbare DNS-Bind-Adresse (Dual-Listener)",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #2 DNS)
                    Loopback-DNS (`127.0.0.1`) ist aus Guests **nicht** erreichbar.
                    Separater Listener auf Host-/Gateway-IP nötig.

                    ## Tasks
                    - [ ] Welche IP auf welchem Interface ist aus Guest erreichbar?
                    - [ ] UDP/TCP Port, Privilegien (53 vs high port 15353)
                    - [ ] Firewall/mdns Konflikte notieren
                    - [ ] Ergebnis → Spec für Dual Listener (Host + Guest)

                    {refs('05-gpt-sol-review.md','01-implementation-plan.md')}
                    """
                ),
                ["type:spike", "priority:p0", "phase:g0", "area:dns", "finding:sol"],
                G0,
                blocked_by_keys=["g0-vmnet-two-nets"],
            ),
            Story(
                "g0-sleep-crash",
                "Spike: Host-Sleep Clock-Drift + Supervisor-Crash Semantik",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable Host-Realität / SOL Prozess)
                    Sleep → Clock-Drift bricht TLS/Tokens. Supervisor-Crash: VMs laufen,
                    aber DNS/vmnet können tot sein → „Crash-Isolation“ nur teilweise.

                    ## Tasks
                    - [ ] Sleep/Wake: Guest-Uhr vs Host messen
                    - [ ] Supervisor kill -9: Helper/VM/DNS/vmnet Verhalten
                    - [ ] Akzeptanzkriterien für Alpha dokumentieren

                    {refs('02-fable-review.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:spike", "priority:p0", "phase:g0", "area:supervisor", "finding:sol", "finding:fable"],
                G0,
                blocked_by_keys=["g0-vmnet-two-nets"],
            ),
        ],
    ),
    Epic(
        key="ownership",
        title="Epic: Process-Modell & Ownership (Supervisor + Helper)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Fable Must: Helper-pro-VM. SOL Must: Ownership ADR — wer besitzt VZ, vmnet, DNS,
            Journal; Reconnect/Upgrade/Doppel-Helper.

            ## Abhängig von
            G0 Spike (Netz-/Crash-Findings fließen in ADR)

            {refs('01-implementation-plan.md','04-decision-log.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:p0", "area:supervisor", "finding:fable", "finding:sol"],
        milestone=P0,
        blocked_by_keys=["g0"],
        stories=[
            Story(
                "adr-ownership",
                "ADR: Ressourcen-Ownership (VZ=Helper, vmnet/DNS/Journal=Supervisor)",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #1 / Decision #19)
                    Prozessmodell war entschieden, aber technisch nicht geschlossen.

                    ## Deliverable `docs/adr/0002-process-ownership.md`
                    - [ ] VZVirtualMachine → Helper
                    - [ ] vmnet refs → Supervisor; Attachment-Handle an Helper
                    - [ ] DNS → Supervisor; Verhalten nach Crash
                    - [ ] Journal/Lease → Supervisor
                    - [ ] launchd Job pro VM-ID; Reconnect UDS; Doppel-Helper Lock
                    - [ ] Alpha-Upgrade: nur gestoppte VMs

                    {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:adr", "priority:p0", "phase:p0", "area:docs", "area:supervisor", "finding:sol"],
                P0,
            ),
            Story(
                "supervisor-uds-rpc",
                "Supervisor: UDS JSON-RPC + health + SQLite Stub",
                textwrap.dedent(
                    f"""\
                    ## Scope
                    - [ ] Unix Domain Socket unter Application Support
                    - [ ] `daemon.health` RPC
                    - [ ] SQLite Schema stub (resources, labels, journal)
                    - [ ] Peer-Cred prüfen (User-owned socket)

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:supervisor", "finding:plan"],
                P0,
                blocked_by_keys=["adr-ownership"],
            ),
            Story(
                "helper-launchd",
                "VM-Helper: launchd Lifecycle + eine VZVirtualMachine",
                textwrap.dedent(
                    f"""\
                    ## Scope
                    - [ ] Helper-Binary hält genau eine VZ VM (NAT boot Ubuntu)
                    - [ ] Spawn vom Supervisor mit Config-Pfad + net-handle
                    - [ ] Crash einer Helper-Instanz lässt andere VMs unberührt (Test)
                    - [ ] State-Report an Supervisor

                    ## Finding
                    Fable: Monolith-Daemon = alle VMs tot bei Crash — deshalb 1:1 Helper.

                    {refs('02-fable-review.md','01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:helper", "finding:fable"],
                P0,
                blocked_by_keys=["adr-ownership", "supervisor-uds-rpc"],
            ),
            Story(
                "helper-reconnect",
                "Helper↔Supervisor Reconnect nach Supervisor-Restart",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL)
                    Wenn Supervisor stirbt: DNS/vmnet weg, VMs „laufen“ aber unbenutzbar.
                    Reconnect + `net_orphaned` Meldeweg spezifizieren/implementieren.

                    ## Acceptance
                    - [ ] Helper reconnectet UDS
                    - [ ] Meldet net_orphaned wenn Attachments tot
                    - [ ] Supervisor kann Net neu aufbauen und Helper re-attach (Alpha-Pfad)

                    {refs('05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:helper", "area:supervisor", "finding:sol"],
                P0,
                blocked_by_keys=["helper-launchd"],
            ),
        ],
    ),
    Epic(
        key="agent",
        title="Epic: vsock Guest-Agent (in sealed Base)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Control-Plane über vsock, nicht SSH. Agent **vorinstalliert in Base** (SOL #21),
            cloud-init nur Identity.

            Capabilities: ping, exec, report-ip, health, time-sync, CA-inject, log-tail.

            {refs('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:p0", "area:agent", "finding:fable", "finding:sol"],
        milestone=P0,
        blocked_by_keys=["ownership"],
        stories=[
            Story(
                "agent-protocol",
                "Guest-Agent: vsock Protokoll + Auth-Token Spec",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #5 Bootstrap-Zirkel / Fable Guest-Agent)
                    SSH allein trägt exec/CA/IP nicht. vsock Auth + Versionierung nötig.

                    ## Deliverable
                    - [ ] Framing/RPC Spec (JSON oder protobuf)
                    - [ ] Auth: Token aus NoCloud / shared secret
                    - [ ] Version Handshake + Upgrade-Strategie
                    - [ ] Agent-ready Event für Bootstrap-Fenster

                    {refs('05-gpt-sol-review.md','02-fable-review.md')}
                    """
                ),
                ["type:story", "priority:p0", "phase:p0", "area:agent", "finding:sol"],
                P0,
            ),
            Story(
                "agent-in-base",
                "Guest-Agent in Ubuntu Base vorinstallieren + seal-ready",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #21)
                    Nicht First-Boot cloud-init Install — Agent muss im sealed Base liegen.

                    ## Tasks
                    - [ ] Build-Pipeline: Agent-Binary → Base Image
                    - [ ] systemd unit enabled
                    - [ ] cloud-init nur Config (hostname, token path)
                    - [ ] Dokumentieren im Seal-Prozess

                    {refs('04-decision-log.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:agent", "area:images", "finding:sol"],
                P0,
                blocked_by_keys=["agent-protocol"],
            ),
            Story(
                "agent-exec-ip",
                "Helper↔Agent: ping, exec, report-ip End-to-End",
                textwrap.dedent(
                    f"""\
                    ## Acceptance
                    - [ ] `vzctl vm exec` über vsock (nicht SSH)
                    - [ ] report-ip liefert Attachment-IPs
                    - [ ] Health endpoint
                    - [ ] Serial Fallback dokumentiert wenn Agent down

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:agent", "area:helper", "finding:plan"],
                P0,
                blocked_by_keys=["agent-in-base", "helper-launchd"],
            ),
            Story(
                "agent-timesync",
                "Guest-Agent Time-Sync nach Host-Sleep",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable/SOL Sleep)
                    Clock-Drift nach Sleep bricht TLS/OIDC später.

                    ## Acceptance
                    - [ ] Nach Wake: Agent korrigiert Guest-Uhr (chrony/sntp/host hint)
                    - [ ] Akzeptanztest Sleep/Wake

                    {refs('02-fable-review.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:agent", "finding:fable", "finding:sol"],
                P0,
                blocked_by_keys=["agent-exec-ip", "g0-sleep-crash"],
            ),
        ],
    ),
    Epic(
        key="cli",
        title="Epic: CLI Contracts (JSON, Exitcodes, Events, doctor)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Agent-freundliche CLI. SOL Should: versioniertes Event-Schema, Exitcodes,
            Timeouts, dry-run. doctor ab P0 (Fable).

            {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p1", "phase:p1", "area:cli", "finding:plan", "finding:sol"],
        milestone=P1,
        blocked_by_keys=["ownership"],
        stories=[
            Story(
                "cli-json-exit",
                "CLI: --format json + stabile Exitcodes Spec",
                textwrap.dedent(
                    f"""\
                    ## Exitcodes (Plan)
                    0 ok, 2 not found, 3 invalid, 4 daemon down, 5 guest error

                    ## Tasks
                    - [ ] Spec in docs + schema crate
                    - [ ] vm list/info/create/start/stop mit JSON
                    - [ ] stderr vs stdout Konvention

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:cli", "finding:plan"],
                P1,
            ),
            Story(
                "cli-events",
                "versioniertes Event-Schema + `vzctl events subscribe`",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable Gap / SOL Widerspruch P1 vs P7)
                    Events gehören früh — Agent-first wichtiger als UI.

                    ## Tasks
                    - [ ] Schema v1 (vm.*, net.*, apply.*)
                    - [ ] NDJSON stream RPC
                    - [ ] Version-Header

                    {refs('02-fable-review.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:cli", "finding:fable", "finding:sol"],
                P1,
                blocked_by_keys=["supervisor-uds-rpc"],
            ),
            Story(
                "cli-doctor",
                "`vzctl doctor` — Entitlements, APFS, Sock, vmnet, DNS",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable)
                    doctor ab P0, nicht P7 — Setup-Blocker früh sichtbar.

                    ## Checks
                    - [ ] Socket erreichbar
                    - [ ] Entitlements / Virtualization
                    - [ ] APFS clonefile Fähigkeit
                    - [ ] DNS Listener / Resolver-Dateien
                    - [ ] Port-Kollisionen (später)

                    {refs('02-fable-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p0", "area:cli", "finding:fable"],
                P0,
                blocked_by_keys=["supervisor-uds-rpc"],
            ),
        ],
    ),
    Epic(
        key="images",
        title="Epic: Base Seal, Linked Clones, Identity-Reset",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Shared Base + APFS clonefile + dataDisk. Identity nie von Base übernehmen.
            Agent bereits in Base (Abhängigkeit Epic Agent).

            {refs('01-implementation-plan.md')}
            """
        ),
        labels=["type:epic", "priority:p1", "phase:p1", "area:images", "finding:plan"],
        milestone=P1,
        blocked_by_keys=["agent"],
        stories=[
            Story(
                "image-seal",
                "`vzctl image seal` — clone-safe Base (machine-id/ssh clean)",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] cloud-init clean / truncate machine-id / remove ssh host keys
                    - [ ] Base read-only + sealed label
                    - [ ] Guest-Agent bleibt installiert
                    - [ ] Docs Seal-Pipeline

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:images", "finding:plan"],
                P1,
                blocked_by_keys=["agent-in-base"],
            ),
            Story(
                "image-clonefile",
                "APFS linked clone + leeres dataDisk attach",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] clonefile(2) Base → VM root disk
                    - [ ] Sparse/ASIF dataDisk neu
                    - [ ] Fallback `clone: full`
                    - [ ] Purge löscht Clone+dataDisk, Base bleibt

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:images", "finding:plan"],
                P1,
                blocked_by_keys=["image-seal", "helper-launchd"],
            ),
            Story(
                "image-identity",
                "Identity-Reset: MAC, machine-id, hostname, SSH keys, instance-id",
                textwrap.dedent(
                    f"""\
                    ## Finding (Plan / Fable Identity-Tabelle)
                    Niemals MAC/machine-id aus Disk lesen — immer neu.

                    ## Acceptance
                    - [ ] Neue local-admin MACs pro NIC
                    - [ ] NoCloud instance-id UUID neu
                    - [ ] machine-id regeneriert
                    - [ ] ssh_deletekeys + genkeytypes
                    - [ ] Hostname aus YAML

                    {refs('01-implementation-plan.md','02-fable-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:images", "finding:plan", "finding:fable"],
                P1,
                blocked_by_keys=["image-clonefile"],
            ),
        ],
    ),
    Epic(
        key="dns",
        title="Epic: Hypervisor Dual-DNS + macOS Resolver (*.vz.test)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Kanonische Zone `*.{{project}}.vz.test`. Dual Listener (Host Loopback + Guest IP).
            Forward für externe Namen. `vzctl dns query` spricht vzctl-DNS direkt.

            ## Finding
            Fable: `*.localhost` in Guests kaputt. SOL: Loopback-only + `.vz` TLD-Risiko.

            {refs('01-implementation-plan.md','05-gpt-sol-review.md','04-decision-log.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:p2", "area:dns", "finding:sol", "finding:fable"],
        milestone=P2,
        blocked_by_keys=["g0", "ownership"],
        stories=[
            Story(
                "dns-zone-server",
                "DNS Server: autoritative Zone + Forwarder + TTL",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] Zone `*.{{project}}.vz.test` aus Actual State (A records)
                    - [ ] Services `auth.svc`, `docker.svc`
                    - [ ] Forward upstream=system; VPN-Verhalten dokumentieren
                    - [ ] TTL 5–30s
                    - [ ] Host listener 127.0.0.1:15353
                    - [ ] Guest listener laut G0-Spike

                    {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p0", "phase:p2", "area:dns", "finding:sol"],
                P2,
                blocked_by_keys=["g0-dns-listener-probe", "supervisor-uds-rpc"],
            ),
            Story(
                "dns-macos-resolver",
                "macOS `/etc/resolver/{{project}}.vz.test` install/cleanup",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] `vzctl dns install-resolver` (sudo)
                    - [ ] `uninstall-resolver` + purge entfernt verwaiste Dateien
                    - [ ] Projektkollisionen behandeln
                    - [ ] Port in resolver-Datei korrekt

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p2", "area:dns", "finding:sol"],
                P2,
                blocked_by_keys=["dns-zone-server"],
            ),
            Story(
                "dns-query-cli",
                "`vzctl dns query` — direkter Query gegen vzctl-DNS",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL)
                    dig/getaddrinfo umgehen teilweise Systemresolver — CLI muss DNS-Server direkt fragen.

                    {refs('05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p2", "area:dns", "area:cli", "finding:sol"],
                P2,
                blocked_by_keys=["dns-zone-server"],
            ),
            Story(
                "dns-guest-cloudinit",
                "Guests: nameservers=Hypervisor-DNS + search domain via cloud-init",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] network-config setzt DNS auf Guest-Listener-IP
                    - [ ] search: `dmz.{{project}}.vz.test`
                    - [ ] Resolve `web.dmz.{{project}}.vz.test` aus anderem Guest

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p2", "area:dns", "finding:plan"],
                P2,
                blocked_by_keys=["dns-zone-server", "image-identity"],
            ),
        ],
    ),
    Epic(
        key="network",
        title="Epic: vmnet Networks, Router Routes + Firewall Policies",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Deklarative Netze, Cross-Net via Router-VM, **policies** für echte DMZ-Semantik
            (SOL: routes allein ≠ Isolation).

            {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:p2", "area:network", "finding:sol"],
        milestone=P2,
        blocked_by_keys=["g0", "ownership"],
        stories=[
            Story(
                "net-crud",
                "vmnet Network CRUD + Attachments + Labels",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] net create/list/attach/detach/delete
                    - [ ] Desired State persistieren (vmnet nicht kernel-persistent)
                    - [ ] rebuildNetworks bei Supervisor-Start
                    - [ ] Labels managed-by=vzctl

                    ## Finding (Fable/WWDC)
                    vmnet Networks nicht persistent; App muss Desired State speichern.

                    {refs('01-implementation-plan.md','02-fable-review.md')}
                    """
                ),
                ["type:story", "priority:p0", "phase:p2", "area:network", "finding:fable"],
                P2,
                blocked_by_keys=["g0-vmnet-two-nets", "adr-ownership"],
            ),
            Story(
                "net-router-routes",
                "Router-VM Template + `vzctl route apply`",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] roles: [router] cloud-init (forwarding, nftables/sysctl)
                    - [ ] route apply pusht via Agent
                    - [ ] Gateway-Konvention aus G0 einhalten

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p2", "area:network", "finding:plan"],
                P2,
                blocked_by_keys=["net-crud", "g0-ip-gateway-convention", "agent-exec-ip"],
            ),
            Story(
                "net-policies",
                "Firewall `policies:` Forward allow/deny (DMZ-Semantik)",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #3)
                    `routes: from/to/via` sagt nicht ob Firewall entsteht — „DMZ“ suggeriert Isolation.

                    ## Tasks
                    - [ ] Schema `policies`
                    - [ ] Apply auf Router-VM
                    - [ ] Default deny-all + allow list Beispiel

                    {refs('05-gpt-sol-review.md','04-decision-log.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p2", "area:network", "finding:sol"],
                P2,
                blocked_by_keys=["net-router-routes"],
            ),
        ],
    ),
    Epic(
        key="stack",
        title="Epic: Stack Reconciler (hypernetwork.config.yaml)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Git-Env up/down/apply/diff/ps/validate/adopt. Lease **und** Journal/Resume (SOL #20).

            {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p0", "phase:p3", "area:stack", "finding:plan", "finding:sol"],
        milestone=P3,
        blocked_by_keys=["dns", "network", "images", "cli"],
        stories=[
            Story(
                "adr-apply",
                "ADR/Spec: Apply-Journal, Idempotenz, Resume/Abort, Purge-Regeln",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL #4)
                    Lease verhindert paralleles apply, nicht inkonsistente Zwischenzustände.

                    ## Deliverable `docs/adr/0003-apply-state.md`
                    - [ ] Journal Felder (id, gen, step, status)
                    - [ ] Resume/Abort Semantik
                    - [ ] Drift YAML↔SQLite↔Lockfile
                    - [ ] down vs down --purge Destruktionsregeln
                    - [ ] adopt für Orphans

                    {refs('05-gpt-sol-review.md','01-implementation-plan.md')}
                    """
                ),
                ["type:adr", "priority:p0", "phase:p0", "area:stack", "area:docs", "finding:sol"],
                P0,
            ),
            Story(
                "stack-schema",
                "hypernetwork/v1 JSON Schema + serde (domain .vz.test, dns, policies)",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] Schema crate + gute Fehlermeldungen
                    - [ ] `vzctl validate`
                    - [ ] Felder: domain, dns, networks, routes, policies, vms, images

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p3", "area:stack", "finding:plan"],
                P3,
            ),
            Story(
                "stack-reconcile",
                "Reconciler: plan/diff/up/down/apply + stack lease + journal resume",
                textwrap.dedent(
                    f"""\
                    ## Order
                    Bases → Networks → VMs → CA(optional) → Ports → Docker → DNS reload → Routes/Policies → Hooks

                    ## Acceptance
                    - [ ] Zwei parallele apply → eines blocked (lease)
                    - [ ] Crash mid-apply → `--resume` oder `--abort`
                    - [ ] Idempotentes zweites `up` = no-op
                    - [ ] Lockfile `.vzctl/stack.lock.json`

                    {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p0", "phase:p3", "area:stack", "finding:sol"],
                P3,
                blocked_by_keys=["adr-apply", "stack-schema", "net-crud", "image-clonefile", "dns-zone-server"],
            ),
            Story(
                "stack-example",
                "examples/edge-dmz Referenz-Env + CI validate/diff",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] Beispiel-YAML laut Plan
                    - [ ] CI: validate + diff dry-run
                    - [ ] README im example

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p3", "area:stack", "area:docs", "finding:plan"],
                P3,
                blocked_by_keys=["stack-schema"],
            ),
        ],
    ),
    Epic(
        key="docker",
        title="Epic: Docker Context + Ports (Alpha basic)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Docker SSH-Context (kein offenes 2375). Ports basic. virtiofs → v0.1.x (SOL G4).

            {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p1", "phase:p4", "area:docker", "finding:plan", "finding:sol"],
        milestone=P4,
        blocked_by_keys=["stack"],
        stories=[
            Story(
                "docker-context",
                "Docker VM role + SSH docker context + `vzctl docker` wrapper",
                textwrap.dedent(
                    f"""\
                    ## Finding (Plan / SOL Docker)
                    2375 auch loopback nur mit Warning — SSH Default.

                    ## Acceptance
                    - [ ] roles: [docker] cloud-init
                    - [ ] Context `vzctl-{{project}}`
                    - [ ] `vzctl docker ps` funktioniert
                    - [ ] DNS Name `docker.svc.{{project}}.vz.test`

                    {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p4", "area:docker", "finding:sol"],
                P4,
                blocked_by_keys=["stack-reconcile", "dns-zone-server"],
            ),
            Story(
                "ports-forward",
                "Port-Forwards + Collision-Check (vmnet / fallback)",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] VM- und Stack-`ports`
                    - [ ] Collision detection bei apply
                    - [ ] `vzctl port list --format json`
                    - [ ] macOS 26 vmnet forward oder SSH -L Fallback

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p4", "area:docker", "area:network", "finding:plan"],
                P4,
                blocked_by_keys=["net-crud"],
            ),
            Story(
                "virtiofs",
                "v0.1.x: virtiofs Mounts + Perf-Benchmark",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable mounts / SOL G4)
                    Feedback-Loop-kritisch, aber Alpha-Scope sprengend → v0.1.x

                    ## Acceptance
                    - [ ] mounts: in YAML
                    - [ ] Perf-Messung dokumentiert
                    - [ ] Kohärenz-Grenzen in Docs

                    {refs('02-fable-review.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p2", "phase:v01x", "area:helper", "finding:fable", "finding:sol"],
                V01X,
                blocked_by_keys=["helper-launchd"],
            ),
        ],
    ),
    Epic(
        key="v02",
        title="Epic: v0.2 Ingress (Caddy) + Local CA + OIDC (Dex)",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            Embed Caddy + Dex. Issuer `https://auth.svc.{{project}}.vz.test`.
            `*.localhost` nur Host-Alias. CA-Rollout in Guests via Agent.

            ## Nicht
            Eigenen IdP/Proxy schreiben (Fable/SOL Scope-Falle).

            {refs('01-implementation-plan.md','02-fable-review.md','05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p2", "phase:v02", "area:ingress", "area:oidc", "area:ca", "finding:fable", "finding:sol"],
        milestone=V02,
        blocked_by_keys=["stack", "dns", "agent"],
        stories=[
            Story(
                "caddy-ingress",
                "Caddy Ingress auf 127.0.0.1 + Routes aus YAML",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] Embed/bundle Caddy
                    - [ ] Routes host → vm:port
                    - [ ] reload bei apply
                    - [ ] hostAliases *.localhost optional

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p2", "phase:v02", "area:ingress", "finding:plan"],
                V02,
            ),
            Story(
                "ca-rollout",
                "Local CA + Auto-Rollout in Guest Trust-Stores",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] ca init unter Application Support
                    - [ ] NoCloud + Agent rollout (`update-ca-certificates`)
                    - [ ] `vzctl certs verify`
                    - [ ] Fingerprint im Lockfile / Drift

                    ## Finding
                    Guests müssen auth.svc.*.vz.test ohne insecure-skip-verify erreichen.

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p2", "phase:v02", "area:ca", "finding:plan"],
                V02,
                blocked_by_keys=["agent-exec-ip"],
            ),
            Story(
                "dex-oidc",
                "Dex embedded OIDC + clients:auto + requires inject",
                textwrap.dedent(
                    f"""\
                    ## Finding (Fable Scope-Falle)
                    Keinen eigenen OIDC-Provider schreiben — Dex embedden.

                    ## Critical
                    Issuer **nie** *.localhost — kanonisch `https://auth.svc.{{project}}.vz.test`

                    ## Tasks
                    - [ ] Dex bundle + config gen
                    - [ ] clients: auto aus requires + ingress hosts
                    - [ ] Autoconfig inject OIDC_* via cloud-init
                    - [ ] Login UI über Ingress

                    {refs('02-fable-review.md','04-decision-log.md')}
                    """
                ),
                ["type:story", "priority:p2", "phase:v02", "area:oidc", "finding:fable", "finding:sol"],
                V02,
                blocked_by_keys=["caddy-ingress", "ca-rollout", "dns-zone-server"],
            ),
            Story(
                "tauri-ui",
                "Tauri UI: Open Env, Up/Down/Diff, DNS/OIDC Status",
                textwrap.dedent(
                    f"""\
                    ## Finding (SOL Nice)
                    Tauri erst nach belastbarer CLI-DX.

                    ## Rules
                    - [ ] Keine Business-Logik — gleiche Reconcile-Engine
                    - [ ] Kein Feature ohne CLI-Äquivalent

                    {refs('01-implementation-plan.md','05-gpt-sol-review.md')}
                    """
                ),
                ["type:story", "priority:p2", "phase:v02", "area:ui", "finding:sol"],
                V02,
                blocked_by_keys=["stack-reconcile"],
            ),
        ],
    ),
    Epic(
        key="dx",
        title="Epic: DX — Logs, Diagnose, Docs Index",
        body=textwrap.dedent(
            f"""\
            ## Ziel
            SOL Should: logs, Diagnose-Bundles, Recovery-Hinweise. Docs-Index pflegen.

            {refs('05-gpt-sol-review.md')}
            """
        ),
        labels=["type:epic", "priority:p1", "phase:p1", "area:docs", "area:cli", "finding:sol"],
        milestone=P1,
        stories=[
            Story(
                "vm-logs",
                "`vzctl vm logs` (serial/agent) Mindesthilfe Alpha",
                textwrap.dedent(
                    f"""\
                    ## Acceptance
                    - [ ] logs -f über Serial und/oder Agent
                    - [ ] JSON optional

                    {refs('01-implementation-plan.md')}
                    """
                ),
                ["type:story", "priority:p1", "phase:p1", "area:cli", "finding:sol"],
                P1,
                blocked_by_keys=["helper-launchd"],
            ),
            Story(
                "docs-tracking",
                "Docs: Issues↔Plan Mapping in docs/planing/README aktualisieren",
                textwrap.dedent(
                    f"""\
                    ## Tasks
                    - [ ] Tabelle Epic/Issue-Nummern → Plan-Abschnitte
                    - [ ] Link auf GitHub Milestones

                    {refs('README.md')}
                    """
                ),
                ["type:chore", "priority:p2", "phase:p0", "area:docs", "finding:plan"],
                P0,
            ),
        ],
    ),
]


def main() -> None:
    key_to_num: dict[str, int] = {}

    print("=== Create Epics + Stories ===")
    for epic in EPICS:
        epic.number = create_issue(epic.title, epic.body, epic.labels, epic.milestone)
        key_to_num[epic.key] = epic.number
        for st in epic.stories:
            st.number = create_issue(st.title, st.body, st.labels, st.milestone)
            key_to_num[st.key] = st.number

    print("=== Sub-issues (Epic ← Story) ===")
    for epic in EPICS:
        assert epic.number
        for st in epic.stories:
            assert st.number
            add_sub_issue(epic.number, st.number)
            print(f"  #{epic.number} ⊃ #{st.number}")

    print("=== blocked-by dependencies ===")
    # epic-level
    for epic in EPICS:
        assert epic.number
        for bk in epic.blocked_by_keys:
            if bk not in key_to_num:
                print(f"  missing key {bk}")
                continue
            add_blocked_by(epic.number, key_to_num[bk])
            print(f"  #{epic.number} blocked by #{key_to_num[bk]} ({bk})")
        for st in epic.stories:
            assert st.number
            for bk in st.blocked_by_keys:
                if bk not in key_to_num:
                    print(f"  missing key {bk}")
                    continue
                add_blocked_by(st.number, key_to_num[bk])
                print(f"  #{st.number} blocked by #{key_to_num[bk]} ({bk})")

    # Persist map
    mapping = {k: v for k, v in key_to_num.items()}
    path = "/Users/frank/Projects/vzctl/docs/planing/06-github-issue-map.json"
    with open(path, "w") as f:
        json.dump(mapping, f, indent=2, sort_keys=True)
    print(f"Wrote {path}")


if __name__ == "__main__":
    main()
