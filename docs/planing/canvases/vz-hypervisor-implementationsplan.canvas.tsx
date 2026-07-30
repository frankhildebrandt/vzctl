import {
  Callout,
  Card,
  CardBody,
  CardHeader,
  Code,
  Divider,
  Grid,
  H1,
  H2,
  H3,
  Pill,
  Row,
  Stack,
  Stat,
  Table,
  Text,
  computeDAGLayout,
  useHostTheme,
} from "cursor/canvas";

const PHASES = [
  {
    id: "P0",
    name: "Foundation",
    weeks: "1–3",
    goal: "Supervisor + VM-Helper/Prozess, vsock-Agent, doctor",
  },
  {
    id: "P1",
    name: "CLI + Clones",
    weeks: "2–4",
    goal: "JSON-CLI, Seal/clonefile, Identity-Reset",
  },
  {
    id: "P2",
    name: "Net + DNS",
    weeks: "3–6",
    goal: "vmnet, IP-Modell, Hypervisor-DNS, macOS-Resolver",
  },
  {
    id: "P3",
    name: "Stacks",
    weeks: "5–8",
    goal: "hypernetwork.config.yaml → up/down/apply + Locking",
  },
  {
    id: "P4",
    name: "Docker + Ports",
    weeks: "7–9",
    goal: "Docker-Context, Port-Forwards, virtiofs-Mounts",
  },
  {
    id: "P5",
    name: "Ingress + CA + OIDC",
    weeks: "v0.2",
    goal: "Caddy, Local CA→Guests, Dex, *.localhost Host-Alias",
  },
  {
    id: "P6",
    name: "Tauri UI",
    weeks: "v0.2",
    goal: "Stack-Browser über dieselbe API",
  },
  {
    id: "P7",
    name: "Harden",
    weeks: "ongoing",
    goal: "Signing, Snapshots, k3s-Rolle, Events-Schema",
  },
] as const;

function ArchitectureDiagram() {
  const theme = useHostTheme();
  const layout = computeDAGLayout({
    direction: "horizontal",
    nodeWidth: 110,
    nodeHeight: 36,
    rankGap: 40,
    nodeGap: 20,
    padding: 8,
    nodes: [
      { id: "git" },
      { id: "cli" },
      { id: "plan" },
      { id: "sup" },
      { id: "dns" },
      { id: "help" },
      { id: "agent" },
      { id: "guest" },
    ],
    edges: [
      { from: "git", to: "cli" },
      { from: "cli", to: "plan" },
      { from: "plan", to: "sup" },
      { from: "sup", to: "dns" },
      { from: "sup", to: "help" },
      { from: "help", to: "guest" },
      { from: "agent", to: "help" },
      { from: "dns", to: "guest" },
    ],
  });

  const labels: Record<string, string> = {
    git: "Git Env",
    cli: "vzctl CLI",
    plan: "Reconciler",
    sup: "Supervisor",
    dns: "DNS + Resolver",
    help: "VM Helper",
    agent: "vsock Agent",
    guest: "Guest VM",
  };

  return (
    <svg
      width="100%"
      height={layout.height}
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      style={{ display: "block", maxWidth: 980 }}
    >
      {layout.edges.map((e) => (
        <line
          key={`${e.from}-${e.to}`}
          x1={e.sourceX}
          y1={e.sourceY}
          x2={e.targetX}
          y2={e.targetY}
          stroke={theme.stroke.secondary}
          strokeWidth={1.5}
        />
      ))}
      {layout.nodes.map((n) => (
        <g key={n.id}>
          <rect
            x={n.x}
            y={n.y}
            width={110}
            height={36}
            rx={4}
            fill={
              n.id === "dns" || n.id === "agent" || n.id === "help"
                ? theme.fill.tertiary
                : theme.fill.secondary
            }
            stroke={theme.stroke.primary}
          />
          <text
            x={n.x + 55}
            y={n.y + 22}
            textAnchor="middle"
            fill={theme.text.primary}
            fontSize={10}
            fontFamily="system-ui, sans-serif"
          >
            {labels[n.id]}
          </text>
        </g>
      ))}
    </svg>
  );
}

export default function VzHypervisorImplementationsplan() {
  const theme = useHostTheme();

  return (
    <Stack gap={24} style={{ padding: 24, maxWidth: 1100 }}>
      <Stack gap={8}>
        <H1>Implementationsplan: Apple-VZ Hypervisor</H1>
        <Text tone="secondary">
          Devstack-Supervisor auf Virtualization.framework — Git-Environments (
          <Code>hypernetwork.config.yaml</Code>), Helper-pro-VM, vsock-Agent,
          Hypervisor-DNS + macOS-Resolver, Linked Clones, Docker. Ingress/OIDC/CA
          in v0.2. Arbeitstitel: <Code>vzctl</Code>.
        </Text>
        <Row gap={8} wrap>
          <Pill size="sm" active>
            Must-Fixes eingearbeitet
          </Pill>
          <Pill tone="success" size="sm" active>
            Helper / VM
          </Pill>
          <Pill tone="success" size="sm" active>
            vsock Agent
          </Pill>
          <Pill tone="success" size="sm" active>
            Hypervisor-DNS
          </Pill>
          <Pill tone="info" size="sm">
            macOS Resolver
          </Pill>
          <Pill tone="warning" size="sm">
            MVP v0.1 ~8–10 Wo
          </Pill>
        </Row>
      </Stack>

      <Callout tone="warning" title="Review-Must-Fixes (Fable)">
        Prozessmodell = Supervisor + ein Helper-Prozess pro VM. Guest-Komm =
        vsock-Agent (SSH Fallback). DNS = Hypervisor für interne Namen +
        macOS-Resolver; <Code>*.localhost</Code> nur Host-Alias. OIDC / Ingress /
        CA-Rollout → v0.2. Dex + Caddy embedden, nicht selbst bauen.
      </Callout>

      <Callout tone="info" title="Positionierung">
        Nicht gegen OrbStack/Multipass — Nische: „compose für VM-Topologien“ /
        Environments as Code für macOS-VMs (Multi-VM, echte Netze, git-native,
        agent-steuerbar).
      </Callout>

      <Grid columns={4} gap={12}>
        <Stat value="v0.1" label="Core Supervisor" tone="success" />
        <Stat value="1:1" label="Helper pro VM" tone="info" />
        <Stat value="DNS" label="Hypervisor + macOS" tone="success" />
        <Stat value="v0.2" label="Ingress/OIDC/CA" tone="warning" />
      </Grid>

      <Stack gap={10}>
        <H2>Architektur (Must-Fix)</H2>
        <ArchitectureDiagram />
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Supervisor (Swift) — schlank</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  UDS-RPC, SQLite Desired/Actual, Stack-Locks, spawnt/überwacht
                  VM-Helper via launchd/XPC.
                </Text>
                <Text size="small" tone="secondary">
                  Besitzt vmnet-Network-Refs + DNS-Service (interne Zone).
                </Text>
                <Text size="small" tone="secondary">
                  Crash/Update des Supervisors darf laufende Helpers nicht
                  killen (Reconnect); Helper-Crash = nur diese eine VM.
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>VM-Helper (1 Prozess / VM)</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Hält genau eine <Code>VZVirtualMachine</Code> + vsock zum
                  Guest-Agent.
                </Text>
                <Text size="small" tone="secondary">
                  Lifecycle start/stop/console; meldet State an Supervisor.
                </Text>
                <Text size="small" tone="secondary">
                  Entitlements am Helper; Upgrade-fähig ohne Global-Outage.
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>vsock Guest-Agent</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Install via cloud-init in Base/Seal. Kanal:{" "}
                  <Code>VZVirtioSocketDevice</Code>.
                </Text>
                <Text size="small" tone="secondary">
                  Capabilities: exec, IP/Health-Report, Time-Sync nach
                  Host-Sleep, CA-Inject, Log-Tail.
                </Text>
                <Text size="small" tone="secondary">
                  SSH bleibt Fallback — nicht Primärpfad für Control-Plane.
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Reconciler + Locking</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  CLI/Tauri teilen <Code>vzctl-reconcile</Code>;{" "}
                  <Code>stack.apply</Code>-Lease lebt im Supervisor.
                </Text>
                <Text size="small" tone="secondary">
                  Wahrheit: YAML desired → Daemon actual; Lockfile = lokale
                  Instanz-Map. <Code>vzctl adopt</Code> für Orphans.
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>DNS: Hypervisor intern + macOS-Resolver</H2>
        <Callout tone="danger" title="Warum nicht auth.localhost in Guests">
          <Code>*.localhost</Code> resolved im Guest auf dessen Loopback (RFC
          6761) — OIDC/Ingress-Issuer würden brechen. Kanonisch ist die{" "}
          <Text weight="semibold" as="span">
            interne Zone
          </Text>
          ; <Code>*.localhost</Code> nur Host-Alias auf denselben Ingress.
        </Callout>

        <H3>Namensschema</H3>
        <Code>{`# Kanonisch (Guests + OIDC issuer + inter-VM)
{vm}.{network}.{project}.vz
Beispiel:  web.dmz.edge-dmz.vz
           auth.svc.edge-dmz.vz      # OIDC / System-Services
           router.dmz.edge-dmz.vz

# Host-Alias (Browser am Mac) — optional, v0.2 Ingress
web.localhost  →  gleiche Upstream-IP wie web.dmz.edge-dmz.vz
auth.localhost →  auth.svc.edge-dmz.vz`}</Code>

        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Hypervisor-DNS (intern)</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Supervisor betreibt autoritativen DNS für Zone{" "}
                  <Code>*.{"{project}"}.vz</Code> (eingebettet, z. B. hickory-dns
                  / dnsmasq-äquivalent im Daemon).
                </Text>
                <Text size="small" tone="secondary">
                  Records aus Actual State: VM-Attachments (A/AAAA), Services (
                  <Code>auth.svc</Code>, <Code>docker.svc</Code>).
                </Text>
                <Text size="small" tone="secondary">
                  Guests bekommen DNS=Gateway/Hypervisor-IP per cloud-init
                  network-config (oder DHCP option 6) — nie 127.0.0.53 allein.
                </Text>
                <Text size="small" tone="secondary">
                  Search-Domain: <Code>dmz.{"{project}"}.vz</Code> → kurze Namen{" "}
                  <Code>web</Code> auflösbar.
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>macOS-Resolver (Host)</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Pro Projekt:{" "}
                  <Code>/etc/resolver/{"{project}"}.vz</Code> →{" "}
                  <Code>nameserver 127.0.0.1</Code> Port des Hypervisor-DNS
                  (oder dedizierter Loopback-Listener).
                </Text>
                <Text size="small" tone="secondary">
                  <Code>vzctl dns install-resolver</Code> /{" "}
                  <Code>uninstall-resolver</Code> (admin einmalig); Stack-up
                  aktualisiert nur Zone-Daten.
                </Text>
                <Text size="small" tone="secondary">
                  Damit resolven <Code>curl</Code>, Docker-CLI, Agents, dig am
                  Mac: <Code>web.dmz.edge-dmz.vz</Code>.
                </Text>
                <Text size="small" tone="secondary">
                  v0.2: zusätzlich <Code>/etc/resolver/localhost</Code> nur für
                  Ingress-Aliase — oder Caddy-only + Browser; curl weiter über{" "}
                  <Code>.vz</Code>.
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>

        <Code>{`# hypernetwork.config.yaml — DNS-Block
spec:
  domain: edge-dmz.vz          # Zone-Suffix
  dns:
    enabled: true              # Hypervisor-DNS an
    hostResolver: true         # /etc/resolver/<project>.vz
    # listen: 127.0.0.1:5353   # Host-Resolver target
    # guestDns: gateway        # Gateway-IP = DNS für Guests

  # v0.2
  ingress:
    hostAliases: true          # web.localhost → gleiche Backends
  oidc:
    issuer: https://auth.svc.edge-dmz.vz   # kanonisch — nie *.localhost`}</Code>

        <H3>CLI</H3>
        <Code>{`vzctl dns status
vzctl dns query web.dmz.edge-dmz.vz
vzctl dns install-resolver     # schreibt /etc/resolver/… (sudo)
vzctl dns uninstall-resolver
vzctl dns reload               # Zone aus Actual State neu laden`}</Code>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>IP-Vergabe (Spike P2 / Woche 1)</H2>
        <Table
          headers={["Mode", "Wer vergibt IP?", "Statische IPs", "macOS"]}
          rows={[
            [
              "shared (vmnet 26+)",
              "vmnet DHCP + Reservations ODER cloud-init static",
              "Reservation per MAC oder network-config — nicht beides wild",
              "≥26 first-class",
            ],
            [
              "host",
              "vmnet / Daemon",
              "wie shared",
              "≥26",
            ],
            [
              "bridged",
              "externes LAN-DHCP",
              "eingeschränkt",
              "braucht com.apple.vm.networking (Apple Approval)",
            ],
            [
              "pre-26 Fallback",
              "NAT + Router-VM / manuelle Topo",
              "cloud-init static hinter Router",
              "explizit testen oder Baseline = 26",
            ],
          ]}
          rowTone={["success", "info", "warning", "danger"]}
        />
        <Text size="small" tone="tertiary">
          Entscheidung vor P2-Ende: macOS 26 als Mindestversion ODER
          dokumentierter, getesteter Pre-26-Pfad. Schema darf{" "}
          <Code>dhcp: true</Code> und <Code>ip:</Code> nicht gleichzeitig ohne
          klare Precedence.
        </Text>
      </Stack>

      <Stack gap={12}>
        <H2>Phasenplan</H2>
        <Table
          headers={["Phase", "Name", "Zeit", "Deliverable"]}
          columnAlign={["center", "left", "center", "left"]}
          rows={PHASES.map((p) => [
            <Pill size="sm" active>
              {p.id}
            </Pill>,
            p.name,
            p.weeks,
            p.goal,
          ])}
          rowTone={[
            "success",
            "success",
            "success",
            "success",
            "info",
            "warning",
            "warning",
            "neutral",
          ]}
        />
      </Stack>

      <Stack gap={16}>
        <H2>P0 — Foundation (Must)</H2>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Scope</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Supervisor + VM-Helper-Protokoll (XPC/UDS)
                </Text>
                <Text size="small" tone="secondary">
                  Eine Linux-VM NAT booten (Helper hält VZ)
                </Text>
                <Text size="small" tone="secondary">
                  vsock Guest-Agent MVP: ping, exec, report-ip
                </Text>
                <Text size="small" tone="secondary">
                  SQLite + Labels; <Code>vzctl doctor</Code> Stub
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Exit</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Helper-Crash lässt andere VMs unberührt
                </Text>
                <Text size="small" tone="secondary">
                  <Code>vzctl vm exec</Code> über vsock (nicht SSH)
                </Text>
                <Text size="small" tone="secondary">
                  doctor meldet Entitlements / APFS / Sock
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Stack gap={16}>
        <H2>P1 — CLI + Seal/Clone</H2>
        <Code>{`vzctl vm create|start|stop|delete|list|info|exec|console --format json
vzctl image pull|seal
# Linked Clone via APFS clonefile + Identity-Reset (machine-id, MAC, ssh keys)
vzctl events subscribe   # Schema versionieren — Agent-first`}</Code>
        <Text size="small" tone="secondary">
          Seal/Clone vor dem großen Reconciler — sonst Rework an Disk-Handling.
        </Text>
      </Stack>

      <Stack gap={16}>
        <H2>P2 — Networks + Hypervisor-DNS + macOS-Resolver</H2>
        <Code>{`vzctl net create|attach|list
vzctl route add|apply          # Router-VM Default für Cross-Net
vzctl dns status|query|reload|install-resolver`}</Code>
        <Text size="small" tone="secondary">
          Guests: DNS → Hypervisor. Host: <Code>/etc/resolver/*.vz</Code>.
          Inter-VM-Namen = <Code>{"{vm}.{net}.{project}.vz"}</Code>.
        </Text>
      </Stack>

      <Stack gap={16}>
        <H2>P3 — Stacks (Git-Env)</H2>
        <Code>{`apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: edge-dmz
spec:
  project: edge-dmz
  domain: edge-dmz.vz
  dns:
    enabled: true
    hostResolver: true

  images:
    ubuntu-base:
      from: ubuntu:24.04
      role: base

  networks:
    dmz:
      cidr: 10.80.0.0/24
      mode: shared
      # IP-Precedence: attachment.ip > dhcp reservation > dhcp pool
    lan:
      cidr: 10.90.0.0/24
      mode: shared

  routes:
    - name: dmz-to-lan
      from: dmz
      to: lan
      via: router

  vms:
    router:
      from: ubuntu-base
      clone: linked
      dataDisk: 4G
      networks:
        - { name: dmz, ip: 10.80.0.1 }
        - { name: lan, ip: 10.90.0.1 }
      cloudInit: cloud-init/router.yaml
      roles: [router]

    web:
      from: ubuntu-base
      clone: linked
      dataDisk: 40G
      dependsOn: [router]
      networks:
        - { name: dmz, ip: 10.80.0.10 }
      cloudInit: cloud-init/web.yaml
      # v0.2: requires: [oidc]

    docker:
      from: ubuntu-base
      roles: [docker]
      dataDisk: 100G
      networks:
        - { name: dmz, ip: 10.80.0.50 }`}</Code>
        <Code>{`vzctl up|down|apply|diff|ps|validate|adopt
# Stack-Lease im Supervisor — kein paralleles apply`}</Code>
      </Stack>

      <Stack gap={16}>
        <H2>P4 — Docker + Ports + Mounts (noch v0.1)</H2>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Docker</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Context per SSH (Default) — kein offenes 2375.{" "}
                <Code>vzctl docker …</Code> Wrapper. DNS-Name{" "}
                <Code>docker.svc.{"{project}"}.vz</Code>.
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Ports + virtiofs</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Port-Forwards + Collision-Check. Mounts = virtiofs mit eigenem
                Perf-Ticket (Feedback-Loop-kritisch).
              </Text>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Stack gap={16}>
        <H2>P5 — v0.2: Ingress, CA-Rollout, OIDC</H2>
        <Callout tone="info" title="Scope v0.2 — nicht v0.1-Muss">
          Embed <Code>Caddy</Code> + <Code>Dex</Code>. Kanonischer Issuer{" "}
          <Code>https://auth.svc.{"{project}"}.vz</Code>. Host-Aliase{" "}
          <Code>*.localhost</Code> optional. CA-Rollout in Guests via Agent +
          NoCloud.
        </Callout>
        <Code>{`ca:
  name: vzctl-local
  rollout:
    enabled: true
    targets: all
    stores: [system]

ingress:
  enabled: true
  bind: 127.0.0.1
  hostAliases: true          # web.localhost → gleicher Upstream
  routes:
    - host: web.svc.edge-dmz.vz
      to: web:80
    - host: auth.svc.edge-dmz.vz
      to: oidc:5556

oidc:
  enabled: true
  issuer: https://auth.svc.edge-dmz.vz
  mode: embedded             # Dex
  clients: auto
  users:
    - { username: admin, passwordFile: ./secrets/admin }
  autoconfig:
    inject: cloud-init`}</Code>
        <Text size="small" tone="secondary">
          Sleep: Agent Time-Sync. Host-Reboot: Supervisor reconcilen
          should-be-running. Stack-Snapshots (APFS) = Nice/v0.3.
        </Text>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>MVP-Schnitt</H2>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>v0.1 Muss (~8–10 Wochen)</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Supervisor + Helper-pro-VM + vsock-Agent
                </Text>
                <Text size="small" tone="secondary">
                  CLI JSON + events + doctor
                </Text>
                <Text size="small" tone="secondary">
                  Netze + Router-Routing + IP-Modell
                </Text>
                <Text size="small" tone="secondary">
                  Hypervisor-DNS + macOS /etc/resolver
                </Text>
                <Text size="small" tone="secondary">
                  Stacks up/down/apply + Locking + Clones/Identity
                </Text>
                <Text size="small" tone="secondary">
                  Docker-Context + Ports (+ virtiofs Basis)
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>v0.2+</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Caddy Ingress + Local CA + Guest-Rollout
                </Text>
                <Text size="small" tone="secondary">
                  Dex OIDC + requires Autoconfig
                </Text>
                <Text size="small" tone="secondary">
                  *.localhost Host-Aliase
                </Text>
                <Text size="small" tone="secondary">
                  Tauri UI, Stack-Snapshots, k3s-Rolle
                </Text>
                <Text size="small" tone="secondary">
                  MCP über RPC (Nice)
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Stack gap={12}>
        <H2>Kickoff-Tickets</H2>
        <Table
          headers={["#", "Ticket", "Phase"]}
          columnAlign={["center", "left", "center"]}
          rows={[
            ["1", "Supervisor↔Helper Protokoll + Crash-Isolation-Test", "P0"],
            ["2", "vsock Guest-Agent: ping/exec/report-ip + cloud-init", "P0"],
            ["3", "vzctl doctor + UDS RPC health", "P0"],
            ["4", "CLI vm lifecycle --format json + events schema", "P1"],
            ["5", "image seal + APFS linked clone + identity reset", "P1"],
            ["6", "Spike: IP/DHCP Precedence + macOS 26 vs Fallback", "P2"],
            ["7", "vmnet nets + Router-VM routes", "P2"],
            ["8", "Hypervisor-DNS Zone *.project.vz aus Actual State", "P2"],
            ["9", "macOS /etc/resolver Installer + dns query", "P2"],
            ["10", "hypernetwork/v1 + reconcile up/down/apply + stack lease", "P3"],
            ["11", "Docker context + ports + virtiofs spike", "P4"],
            ["12", "v0.2: Caddy + Dex + CA rollout + hostAliases", "P5"],
          ]}
          rowTone={[
            "success",
            "success",
            "success",
            "success",
            "success",
            "warning",
            "success",
            "success",
            "success",
            "info",
            "info",
            "warning",
          ]}
        />
      </Stack>

      <Callout tone="success" title="Nächster Schritt">
        Vor Scaffolding die Top-5 bestätigen (hier als Defaults gesetzt):
        Helper-pro-VM, vsock-Agent, Domain <Code>.vz</Code> + Hypervisor-DNS +
        macOS-Resolver, IP-Spike Woche 1, P5 = v0.2 mit Dex/Caddy.
      </Callout>

      <Text size="small" tone="tertiary" style={{ color: theme.text.tertiary }}>
        Kanonische Namen: {"{vm}.{net}.{project}.vz"}. OIDC-Issuer nie auf
        *.localhost. Guests nutzen Hypervisor-DNS; Mac nutzt /etc/resolver.
        Secrets/CA unter .vzctl/ bzw. Application Support — nicht committen.
      </Text>
    </Stack>
  );
}
