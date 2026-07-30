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
  { id: "G0", name: "Spike", weeks: "0–1", goal: "Netz+DNS+Crash Go/No-Go (Host ≥26)" },
  { id: "P0", name: "Foundation", weeks: "1–3", goal: "Ownership ADR, Helper, Agent-in-Base, Journal, doctor" },
  { id: "P1", name: "CLI + Clones", weeks: "2–4", goal: "JSON/Events/Exitcodes, Seal/clonefile, Identity" },
  { id: "P2", name: "Net + DNS", weeks: "3–5", goal: "vmnet, Dual-DNS *.vz.test, Router+Policies" },
  { id: "P3", name: "Stacks", weeks: "5–7", goal: "hypernetwork up/down/apply + Lease + Resume" },
  { id: "P4", name: "Docker + Ports", weeks: "7–9", goal: "SSH Docker-Context, Ports basic" },
  { id: "P4b", name: "v0.1.x", weeks: "nach Alpha", goal: "virtiofs, Docker-Polish, Diagnose" },
  { id: "P5", name: "Ingress + OIDC", weeks: "v0.2", goal: "Caddy, Dex, CA→Guests, *.localhost Alias" },
] as const;

const EPICS = [
  { n: 1, title: "G0 Spike", ms: "G0" },
  { n: 7, title: "Ownership / Helper", ms: "P0" },
  { n: 12, title: "vsock Guest-Agent", ms: "P0" },
  { n: 17, title: "CLI Contracts", ms: "P1" },
  { n: 21, title: "Clones / Identity", ms: "P1" },
  { n: 25, title: "Dual-DNS + Resolver", ms: "P2" },
  { n: 30, title: "Net + Policies", ms: "P2" },
  { n: 34, title: "Stack Reconciler", ms: "P3" },
  { n: 39, title: "Docker + Ports", ms: "P4" },
  { n: 43, title: "Ingress / CA / OIDC", ms: "v0.2" },
  { n: 48, title: "DX Logs / Docs", ms: "P1" },
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
          Apple Virtualization.framework Devstack-Supervisor · Environments as
          Code · Repo{" "}
          <Code>frankhildebrandt/vzctl</Code> · Stand nach Fable + GPT-SOL + ADR
          0001
        </Text>
        <Row gap={8} wrap>
          <Pill tone="success" size="sm" active>
            macOS 26+
          </Pill>
          <Pill tone="success" size="sm" active>
            G1 Baseline closed
          </Pill>
          <Pill tone="success" size="sm" active>
            Helper 1:1
          </Pill>
          <Pill tone="success" size="sm" active>
            Agent Spec+Base
          </Pill>
          <Pill size="sm" active>
            Dual-DNS *.vz.test
          </Pill>
          <Pill tone="success" size="sm" active>
            G0 Go
          </Pill>
          <Pill tone="warning" size="sm">
            v0.1 Alpha
          </Pill>
          <Pill tone="info" size="sm">
            50 Issues tracked
          </Pill>
        </Row>
      </Stack>

      <Callout tone="success" title="ADR 0001 accepted — Mindest-Host macOS 26">
        Pre-26 unsupported. Bridged out of scope. doctor/Validierung: Host &lt; 26
        = hard fail. Issue #2 geschlossen. G0-Spike und alle Phasen zielen nur
        auf macOS 26+ APIs (vmnet Custom inkl. VZVmnetNetworkDeviceAttachment).
      </Callout>

      <Callout tone="success" title="G0 Go — Netz/DNS/Router/Crash">
        Dual-Net, UDP-DNS auf <Code>.0</Code>, Router <Code>.2</Code>, Crash-Semantik
        gemessen. <Code>kill -9</Code> Monolith = VM tot + Subnet-Leak → Helper-Modell
        (ADR 0002 Accepted). Sleep: manuelle Prozedur / Alpha-Risiko.
        Protokoll: <Code>docs/spikes/g0-network.md</Code>
      </Callout>

      <Callout tone="success" title="P0 + CLI Contracts">
        P0 Foundation closed. P1: #18 CLI-v1 + #19 Events ✅. Nächster Slice:
        #21 Seal / Linked Clones / Identity.
      </Callout>

      <Grid columns={4} gap={12}>
        <Stat value="26+" label="min. macOS" tone="success" />
        <Stat value="Go" label="G0 Gate" tone="success" />
        <Stat value="#18+#19" label="CLI+Events" tone="success" />
        <Stat value="#21" label="Clones next" tone="info" />
      </Grid>

      <Stack gap={10}>
        <H2>Positionierung</H2>
        <Text tone="secondary">
          Nicht gegen OrbStack/Multipass: Nische = „compose für VM-Topologien“ /
          Multi-VM, echte Netze, git-native, agent-steuerbar.
        </Text>
      </Stack>

      <Divider />

      <Stack gap={10}>
        <H2>Gates vor Scaffolding</H2>
        <Table
          headers={["Gate", "Status", "Inhalt"]}
          rows={[
            [
              <Pill size="sm" active>
                G0
              </Pill>,
              <Pill tone="success" size="sm" active>
                Go
              </Pill>,
              "Dual-Net+DNS-UDP+Router+Crash ✓ · Sleep manuell · Epic #1",
            ],
            [
              <Pill size="sm" active>
                G1
              </Pill>,
              <Pill tone="success" size="sm" active>
                done
              </Pill>,
              "macOS 26+ · ADR 0001 · Issue #2 closed",
            ],
            [
              <Pill size="sm" active>
                G2
              </Pill>,
              <Pill tone="success" size="sm" active>
                done
              </Pill>,
              "ADR 0002 accepted · Helper 1:1 · Net-Registry",
            ],
            [
              <Pill size="sm" active>
                G3
              </Pill>,
              <Pill tone="success" size="sm" active>
                done
              </Pill>,
              "ADR 0003 Apply-Journal · Resume/Abort/Purge",
            ],
            [
              <Pill size="sm" active>
                G4
              </Pill>,
              <Pill size="sm">set</Pill>,
              "virtiofs + Docker-Polish → v0.1.x (#42)",
            ],
          ]}
          rowTone={["success", "success", "success", "success", "neutral"]}
        />
      </Stack>

      <Divider />

      <Stack gap={10}>
        <H2>Architektur</H2>
        <ArchitectureDiagram />
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>#7✓</Pill>}>
              Ownership
            </CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  VZVirtualMachine → VM-Helper (überlebt Supervisor-Crash)
                </Text>
                <Text size="small" tone="secondary">
                  vmnet-Refs + DNS + Journal → Supervisor; Helper bekommt
                  Attachment-Handle
                </Text>
                <Text size="small" tone="secondary">
                  launchd 1 Job / vm-id · Reconnect UDS · Doppel-Helper Lock
                </Text>
                <Text size="small" tone="secondary">
                  Alpha: DNS down nach Supervisor-Crash bis Restart (dokumentiert)
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>#12✓</Pill>}>
              vsock Guest-Agent
            </CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Spec + Base + Helper E2E + <Code>time_hint</Code> Clock-Step
                </Text>
                <Text size="small" tone="secondary">
                  Port <Code>21950</Code> · Token <Code>0600</Code> ·{" "}
                  <Code>vm.clock_corrected</Code>
                </Text>
                <Text size="small" tone="secondary">
                  Live-Boot/Sleep Residual bis Base-Raw vom Builder
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
        <Card>
          <CardHeader>Apply-Vertrag</CardHeader>
          <CardBody>
            <Text size="small" tone="secondary">
              Desired=YAML · Actual=SQLite · Lockfile=Instanz-Map · Journal
              (id/gen/step/status) · <Code>apply --resume|--abort</Code> · Lease
              gegen Parallelität · purge nur <Code>managed-by=vzctl</Code> +
              Resolver-Dateien · <Code>adopt</Code> für Orphans
            </Text>
          </CardBody>
        </Card>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>Entscheidungen (Auszug)</H2>
        <Table
          headers={["#", "Thema", "Default"]}
          columnAlign={["center", "left", "left"]}
          rows={[
            ["1–2", "Prozess / Agent", "Helper 1:1 · vsock first-class"],
            ["9–12", "DNS", "*.vz.test · Dual Listener · Forward · dns query direkt"],
            ["14", "Baseline", "macOS 26+ (ADR 0001)"],
            ["15", "Bridged", "out of scope"],
            ["16–18", "IP / Isolation", "static · DNS/gw=.0 UDP · Router .2 · .1 unused · policies"],
            ["20–22", "Apply / Agent / MVP", "Journal · Agent-in-Base · Alpha + v0.1.x"],
            ["6–7", "v0.2 Embed", "Caddy + Dex · Issuer nie *.localhost"],
          ]}
          rowTone={["info", "success", "success", "warning", "success", "warning", "info"]}
        />
        <Text size="small" tone="tertiary">
          G0: UDP DNS auf .0 ✓ · Cross-Net via Router .2 ✓ · TCP Host-.0 fail ·
          Sleep noch messen.
        </Text>
      </Stack>

      <Stack gap={12}>
        <H2>G0 Spike — Messstand</H2>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>Go</Pill>}>
              Netz / DNS / Router
            </CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  2× shared vmnet · Ubuntu EFI/NoCloud · static .10
                </Text>
                <Text size="small" tone="secondary">
                  Guest→Host <Code>.0</Code>: ICMP + <Code>UDP:15353</Code> OK ·
                  TCP fail · <Code>.1</Code> tot
                </Text>
                <Text size="small" tone="secondary">
                  Router-VM dual-NIC <Code>.2</Code> ·{" "}
                  <Code>ip_forward</Code> · Cross-Net Ping OK (ttl=63)
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>Crash ✓</Pill>}>
              Phase D + ADR
            </CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  Kill -9: VM tot, Subnet verbrannt, frische CIDR OK
                </Text>
                <Text size="small" tone="secondary">
                  ADR+Ownership+Agent+doctor ✓ · P0 Foundation abgeschlossen
                </Text>
                <Text size="small" tone="secondary">
                  <Code>phase-d-crash.sh</Code> ·{" "}
                  <Code>docs/adr/0002-process-ownership.md</Code>
                </Text>
              </Stack>
            </CardBody>
          </Card>
        </Grid>
        <Table
          headers={["Pfad", "Ergebnis", "Implikation"]}
          rows={[
            ["Host ↔ Guest .10", "ICMP OK", "static cloud-init"],
            ["Guest → Host .0 UDP", "OK echo :15353", "Guest-DNS Listener"],
            ["Guest → Host .0 TCP", "FAIL", "kein TCP-Service auf .0 nötig für DNS"],
            ["Guest → Host .1", "FAIL", "nicht als gw/DNS"],
            ["FE → BE via Router .2", "ICMP OK", "DMZ-Topologie machbar"],
            ["kill -9 Monolith", "VM dead + CIDR leak", "Helper 1:1 Pflicht"],
          ]}
          rowTone={["success", "success", "warning", "danger", "success", "danger"]}
        />
      </Stack>

      <Stack gap={12}>
        <H2>DNS — Dual Listener + *.vz.test</H2>
        <Callout tone="danger" title="Nicht auth.localhost in Guests">
          RFC 6761: *.localhost = Guest-Loopback. Kanonisch{" "}
          <Code>{"{vm}.{net}.{project}.vz.test"}</Code>. OIDC Issuer ={" "}
          <Code>https://auth.svc.{"{project}"}.vz.test</Code>. *.localhost nur
          Host-Alias in v0.2.
        </Callout>
        <Grid columns={2} gap={12}>
          <Card>
            <CardHeader>Host</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Listener <Code>127.0.0.1:15353</Code> +{" "}
                <Code>/etc/resolver/{"{project}"}.vz.test</Code>.{" "}
                <Code>vzctl dns query</Code> spricht DNS direkt (dig umgeht oft
                Resolver).
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Guest</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Listener auf Host-Bridge-<Code>.0:UDP</Code> (gemessen).{" "}
                <Code>.1</Code> nicht. cloud-init <Code>nameservers: [.0]</Code>.
                Host parallel <Code>127.0.0.1</Code> +{" "}
                <Code>/etc/resolver</Code>.
              </Text>
            </CardBody>
          </Card>
        </Grid>
        <Code>{`spec:
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
# kanonisch: web.dmz.edge-dmz.vz.test`}</Code>
      </Stack>

      <Stack gap={12}>
        <H2>Netzwerk & IP (Host ≥ 26)</H2>
        <Table
          headers={["Mode", "IP-Vergabe", "Hinweis"]}
          rows={[
            ["shared", "cloud-init static Primär", "vmnet 26+ first-class"],
            ["host", "wie shared", ""],
            ["bridged", "—", "out of scope"],
            ["pre-26", "—", "unsupported (ADR 0001)"],
          ]}
          rowTone={["success", "info", "warning", "danger"]}
        />
        <Text size="small" tone="secondary">
          Host-DNS/gw = <Code>.0</Code> (UDP). Router = <Code>.2</Code> je Net.
          Guests <Code>.10+</Code>. <Code>routes</Code> + <Code>policies</Code>{" "}
          für DMZ.
        </Text>
      </Stack>

      <Stack gap={12}>
        <H2>Plattenmodell</H2>
        <Grid columns={3} gap={12}>
          <Card>
            <CardHeader>1. Base</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Sealed, immutable, Guest-Agent vorinstalliert.{" "}
                <Code>vzctl image seal</Code>
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>2. Linked Clone</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                APFS <Code>clonefile</Code> COW Root-Disk. Identity-Reset: MAC,
                machine-id, SSH keys, instance-id.
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>3. dataDisk</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Neues leeres Image pro VM. Purge löscht Clone+dataDisk; Base
                bleibt.
              </Text>
            </CardBody>
          </Card>
        </Grid>
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
                  Host macOS 26+ · G0 bestanden
                </Text>
                <Text size="small" tone="secondary">
                  Supervisor + Helper + Agent-in-Base
                </Text>
                <Text size="small" tone="secondary">
                  Dual-DNS *.vz.test + Resolver
                </Text>
                <Text size="small" tone="secondary">
                  Stacks + Journal/Resume + Clones
                </Text>
                <Text size="small" tone="secondary">
                  Docker SSH-Context + Ports basic + logs
                </Text>
              </Stack>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>v0.1.x</CardHeader>
            <CardBody>
              <Stack gap={6}>
                <Text size="small" tone="secondary">
                  virtiofs + Perf (#42)
                </Text>
                <Text size="small" tone="secondary">
                  Docker-Polish / BuildKit
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
                  Caddy + Local CA-Rollout + Dex
                </Text>
                <Text size="small" tone="secondary">
                  *.localhost Host-Aliase
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
        <H2>Config-Skizze (v0.1)</H2>
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
  images:
    ubuntu-base: { from: ubuntu:24.04, role: base }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
    lan: { cidr: 10.90.0.0/24, mode: shared }
  routes:
    - { name: dmz-to-lan, from: dmz, to: lan, via: router }
  policies:
    - name: dmz-default
      network: dmz
      forward: deny-all
      allow: [{ to: lan, proto: tcp, ports: [5432] }]
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
        - { name: dmz, ip: 10.80.0.10 }
    docker:
      from: ubuntu-base
      roles: [docker]
      dataDisk: 100G
      networks:
        - { name: dmz, ip: 10.80.0.50 }`}</Code>
      </Stack>

      <Stack gap={12}>
        <H2>CLI (Zielbild)</H2>
        <Code>{`vzctl up|down|apply|diff|ps|validate|adopt
vzctl apply --resume|--abort
vzctl vm create|start|stop|delete|list|info|exec|console|logs
vzctl image pull|seal|list
vzctl net create|attach|list|delete
vzctl route add|apply   &&   vzctl policy apply
vzctl dns status|query|reload|install-resolver
vzctl docker …   &&   vzctl events subscribe   &&   vzctl doctor
# v0.2: ingress | certs | oidc`}</Code>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>GitHub Tracking</H2>
        <Text size="small" tone="secondary">
          50 Issues · Sub-Issues · blocked-by · Labels type/priority/area/phase/finding ·
          Docs: docs/planing/06-github-tracking.md
        </Text>
        <Table
          headers={["#", "Epic", "Milestone"]}
          columnAlign={["center", "left", "left"]}
          rows={EPICS.map((e) => [
            <Pill size="sm" active>
              #{e.n}
            </Pill>,
            e.title,
            e.ms,
          ])}
          rowTone={[
            "success",
            "success",
            "success",
            "info",
            "info",
            "success",
            "success",
            "info",
            "info",
            "warning",
            "neutral",
          ]}
        />
        <Callout tone="info" title="Nächster Schritt">
          P1 #21: Base Seal / APFS Linked Clones / Identity-Reset. Einstieg über
          #22 <Code>image seal</Code> (Agent in Base erhalten), dann #23{" "}
          <Code>clonefile</Code> + #24 Identity. Events (#19) und CLI-v1 (#18)
          sind closed.
        </Callout>
      </Stack>

      <Stack gap={12}>
        <H2>Repo-Layout (Ist)</H2>
        <Code>{`vzctl/
  crates/vzctl/     # Rust CLI (doctor + supervisor health)
  daemon/           # vz-supervisor + vz-helper (ADR 0002)
  guest-agent/      # Go vzctl-agent + systemd + NoCloud seed
  docs/adr/         # 0001–0003 Accepted
  docs/specs/       # guest-agent-v1.md
  docs/spikes/      # g0-network, p0-helper, p0-guest-agent-base
  spikes/g0/        # G0 measurement harness
  scripts/          # build/smoke guest-agent-base`}</Code>
      </Stack>

      <Text size="small" tone="tertiary" style={{ color: theme.text.tertiary }}>
        Sync: docs/planing/01-implementation-plan.md · 04-decision-log.md ·
        05-gpt-sol-review.md · 06-github-tracking.md · adr/0001-macos-baseline.md
        · github.com/frankhildebrandt/vzctl
      </Text>
    </Stack>
  );
}
