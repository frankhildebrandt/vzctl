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
    id: "G0",
    name: "Spike",
    weeks: "0–1",
    goal: "Netz+DNS+Crash Go/No-Go · macOS-Baseline",
  },
  {
    id: "P0",
    name: "Foundation",
    weeks: "1–3",
    goal: "Supervisor+Helper ADR, Agent-in-Base, Journal, doctor",
  },
  {
    id: "P1",
    name: "CLI + Clones",
    weeks: "2–4",
    goal: "JSON-CLI, Exitcodes, events, Seal/clonefile",
  },
  {
    id: "P2",
    name: "Net + DNS",
    weeks: "3–5",
    goal: "vmnet, Dual-DNS, Resolver, Router+Policies",
  },
  {
    id: "P3",
    name: "Stacks",
    weeks: "5–7",
    goal: "up/down/apply + Lease + Resume",
  },
  {
    id: "P4",
    name: "Docker + Ports",
    weeks: "7–9",
    goal: "Docker SSH-Context, Ports (basic)",
  },
  {
    id: "P4b",
    name: "v0.1.x",
    weeks: "nach Alpha",
    goal: "virtiofs + Docker-Polish + Diagnose",
  },
  {
    id: "P5",
    name: "Ingress + OIDC",
    weeks: "v0.2",
    goal: "Caddy, CA→Guests, Dex, *.localhost Alias",
  },
] as const;

function ArchitectureDiagram() {
  const theme = useHostTheme();
  const layout = computeDAGLayout({
    direction: "horizontal",
    nodeWidth: 108,
    nodeHeight: 36,
    rankGap: 40,
    nodeGap: 18,
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
    dns: "Dual DNS",
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
            width={108}
            height={36}
            rx={4}
            fill={
              n.id === "dns" || n.id === "agent" || n.id === "help" || n.id === "sup"
                ? theme.fill.tertiary
                : theme.fill.secondary
            }
            stroke={theme.stroke.primary}
          />
          <text
            x={n.x + 54}
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
        <H1>vzctl — Implementationsplan</H1>
        <Text tone="secondary">
          Apple-VZ Devstack-Supervisor · Fable + GPT-SOL Must-Fixes · v0.1 ={" "}
          <Text weight="semibold" as="span">
            Alpha
          </Text>
          . Repo: frankhildebrandt/vzctl · docs/planing/
        </Text>
        <Row gap={8} wrap>
          <Pill size="sm" active>
            SOL Must-Fixes
          </Pill>
          <Pill tone="success" size="sm" active>
            Helper / VM
          </Pill>
          <Pill tone="success" size="sm" active>
            Dual-DNS
          </Pill>
          <Pill tone="info" size="sm">
            *.vz.test
          </Pill>
          <Pill tone="warning" size="sm">
            G0 vor P0
          </Pill>
          <Pill tone="warning" size="sm">
            Alpha ~8–10 Wo
          </Pill>
        </Row>
      </Stack>

      <Callout tone="warning" title="GPT-SOL: noch nicht scaffolden">
        Vor Code: G0 Netz-Spike, Ownership-ADR, Apply-Journal-Spez,
        macOS-Baseline. Loopback-DNS allein reicht nicht für Guests — Dual
        Listener Pflicht. v0.1 ist Walking Skeleton, kein Alltagsprodukt.
      </Callout>

      <Grid columns={4} gap={12}>
        <Stat value="G0" label="Spike Gate" tone="warning" />
        <Stat value="1:1" label="Helper pro VM" tone="success" />
        <Stat value="Dual" label="DNS Host+Guest" tone="info" />
        <Stat value="α" label="v0.1 Alpha" tone="warning" />
      </Grid>

      <Stack gap={10}>
        <H2>Gates vor Scaffolding</H2>
        <Table
          headers={["Gate", "Inhalt", "Abbruch"]}
          rows={[
            [
              <Pill size="sm" active>
                G0
              </Pill>,
              "2 Netze, Router, feste IP, Host↔Guest, Sleep, Supervisor-Crash",
              "Isolation/Entitlements unmöglich",
            ],
            [
              <Pill size="sm" active>
                G1
              </Pill>,
              "macOS-Baseline (Empfehlung: 26-only für v0.1)",
              "—",
            ],
            [
              <Pill size="sm" active>
                G2
              </Pill>,
              "Process-/Ownership-ADR (VZ, vmnet, DNS, Helper)",
              "—",
            ],
            [
              <Pill size="sm" active>
                G3
              </Pill>,
              "State/Apply: Journal, Idempotenz, Resume, Purge",
              "—",
            ],
            [
              <Pill size="sm" active>
                G4
              </Pill>,
              "MVP-Gates; virtiofs + Docker-Polish → v0.1.x",
              "Scope > 8–10 Wo",
            ],
          ]}
          rowTone={["danger", "warning", "info", "info", "warning"]}
        />
      </Stack>

      <Divider />

      <Stack gap={10}>
        <H2>Architektur</H2>
        <ArchitectureDiagram />
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Ownership (SOL)</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  VZVirtualMachine → VM-Helper (überlebt Supervisor-Crash)
                </Text>
                <Text size="small" tone="secondary">
                  vmnet-Refs + DNS + Journal → Supervisor; Helper bekommt
                  Attachment-Handle beim Spawn
                </Text>
                <Text size="small" tone="secondary">
                  Net orphaned nach Supervisor-Crash → Reconnect; DNS down bis
                  Restart (Alpha-akzeptiert, dokumentiert)
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Helper-Lifecycle</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  launchd-Job pro VM-ID; UDS Reconnect + State-Report
                </Text>
                <Text size="small" tone="secondary">
                  Doppel-Helper: Lockfile + adopt/kill stale
                </Text>
                <Text size="small" tone="secondary">
                  Alpha-Upgrade: nur gestoppte VMs rolling replace
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>vsock Agent</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                In sealed Base vorinstalliert — nicht First-Boot-Install.
                cloud-init nur Identity. exec / IP / Health / Time-Sync /
                CA-Inject. SSH = Fallback.
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Apply-Vertrag</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Journal (id, gen, step, status) + Lease.{" "}
                <Code>apply --resume|--abort</Code>. Purge nur managed-by=vzctl
                + Resolver-Dateien des Projekts.
              </Text>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>DNS — Dual Listener + .vz.test</H2>
        <Callout tone="danger" title="SOL: Loopback reicht nicht für Guests">
          Host: <Code>127.0.0.1:15353</Code> +{" "}
          <Code>/etc/resolver/{"{project}"}.vz.test</Code>. Guests: Listener auf
          Gateway/Hypervisor-IP. Zone:{" "}
          <Code>{"{vm}.{net}.{project}.vz.test"}</Code>. Forward für externe
          Namen. <Code>vzctl dns query</Code> spricht den vzctl-DNS direkt.
        </Callout>
        <Code>{`spec:
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }

# kanonisch:  web.dmz.edge-dmz.vz.test
# OIDC v0.2:  https://auth.svc.edge-dmz.vz.test
# Host-Alias: web.localhost → gleicher Upstream (nur v0.2)`}</Code>
      </Stack>

      <Stack gap={12}>
        <H2>Netzwerk (G0 Spike)</H2>
        <Table
          headers={["Thema", "v0.1 Default"]}
          rows={[
            ["Baseline", "macOS 26-only (Empfehlung)"],
            ["Bridged", "out of scope"],
            ["IP-Vergabe", "cloud-init static Primär"],
            ["Router-IP", "nicht Gateway .1 — Konvention aus Spike"],
            ["Isolation", "routes + policies (forward allow/deny)"],
            ["Akzeptanztests", "Sleep, VPN, Supervisor-Crash"],
          ]}
          rowTone={["warning", "danger", "success", "warning", "info", "info"]}
        />
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>Phasen</H2>
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
            "danger",
            "success",
            "success",
            "success",
            "info",
            "info",
            "warning",
            "warning",
          ]}
        />
      </Stack>

      <Stack gap={12}>
        <H2>MVP-Schnitt</H2>
        <Grid columns={3} gap={12}>
          <Card>
            <CardHeader>v0.1 Alpha Muss</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  G0 bestanden + Ownership-ADR
                </Text>
                <Text size="small" tone="secondary">
                  Helper-pro-VM + Agent-in-Base
                </Text>
                <Text size="small" tone="secondary">
                  Dual-DNS + Resolver *.vz.test
                </Text>
                <Text size="small" tone="secondary">
                  Stacks + Journal/Resume + Clones
                </Text>
                <Text size="small" tone="secondary">
                  Docker SSH-Context + Ports basic
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>v0.1.x</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  virtiofs + Perf
                </Text>
                <Text size="small" tone="secondary">
                  Docker-Polish
                </Text>
                <Text size="small" tone="secondary">
                  Diagnose-Bundles
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>v0.2+</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Caddy + CA-Rollout + Dex
                </Text>
                <Text size="small" tone="secondary">
                  *.localhost Aliase
                </Text>
                <Text size="small" tone="secondary">
                  Tauri, Snapshots, k3s
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Stack gap={12}>
        <H2>Config-Skizze</H2>
        <Code>{`apiVersion: hypernetwork/v1
kind: Environment
metadata:
  name: edge-dmz
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
    lan: { cidr: 10.90.0.0/24, mode: shared }
  routes:
    - { name: dmz-to-lan, from: dmz, to: lan, via: router }
  policies:
    - name: dmz-default
      network: dmz
      forward: deny-all
  vms:
    router:
      from: ubuntu-base
      clone: linked
      dataDisk: 4G
      networks:
        - { name: dmz, ip: 10.80.0.2 }
        - { name: lan, ip: 10.90.0.2 }
      roles: [router]
    web:
      from: ubuntu-base
      clone: linked
      dataDisk: 40G
      dependsOn: [router]
      networks:
        - { name: dmz, ip: 10.80.0.10 }`}</Code>
      </Stack>

      <Stack gap={12}>
        <H2>Kickoff (aktualisiert)</H2>
        <Table
          headers={["#", "Ticket", "Phase"]}
          columnAlign={["center", "left", "center"]}
          rows={[
            ["0", "G0 Netz-/DNS-/Crash-Spike + Baseline ADR", "G0"],
            ["1", "Ownership-ADR + Helper launchd Lifecycle", "P0"],
            ["2", "Apply-Spez: Journal, Resume, Purge", "P0"],
            ["3", "Guest-Agent in Base + vsock ping/exec", "P0"],
            ["4", "doctor + UDS health", "P0"],
            ["5", "CLI JSON + events + Exitcodes", "P1"],
            ["6", "seal + linked clone + identity", "P1"],
            ["7", "vmnet + Router + policies", "P2"],
            ["8", "Dual-DNS + forward + dns query", "P2"],
            ["9", "/etc/resolver/*.vz.test install/cleanup", "P2"],
            ["10", "reconcile up/down/apply + lease + resume", "P3"],
            ["11", "Docker SSH-context + ports basic", "P4"],
            ["12", "virtiofs + Docker polish", "P4b"],
            ["13", "Caddy + Dex + CA + hostAliases", "P5"],
          ]}
          rowTone={[
            "danger",
            "success",
            "success",
            "success",
            "success",
            "success",
            "success",
            "info",
            "info",
            "info",
            "info",
            "info",
            "warning",
            "warning",
          ]}
        />
      </Stack>

      <Callout tone="success" title="Nächster Schritt">
        G0 vertikaler Spike: zwei Netze, Router, feste IP, Guest-DNS,
        Host-Resolver, Sleep, Supervisor-Crash — dann Ownership-ADR und erst
        Scaffolding.
      </Callout>

      <Text size="small" tone="tertiary" style={{ color: theme.text.tertiary }}>
        Sync mit docs/planing/01-implementation-plan.md ·
        04-decision-log.md · 05-gpt-sol-review.md · Repo
        github.com/frankhildebrandt/vzctl
      </Text>
    </Stack>
  );
}
