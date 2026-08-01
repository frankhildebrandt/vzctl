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
  { id: "G0", name: "Spike", weeks: "0–1", goal: "Netz+DNS+Crash Go/No-Go (Host ≥26)", done: true },
  { id: "P0", name: "Foundation", weeks: "1–3", goal: "Ownership ADR, Helper, Agent-in-Base, Journal, doctor", done: true },
  { id: "P1", name: "CLI + Clones", weeks: "2–4", goal: "JSON/Events/Exitcodes, Seal/clonefile, Identity", done: true },
  { id: "P2", name: "Net + DNS", weeks: "3–5", goal: "vmnet, Dual-DNS *.vz.test, Router+Policies", done: true },
  { id: "P3", name: "Stacks", weeks: "5–7", goal: "#34 closed · #36/#37/#38/#52 ✅", done: true },
  { id: "P4", name: "Docker + Ports", weeks: "7–9", goal: "SSH Docker-Context, Ports basic", done: true },
  { id: "P4b", name: "v0.1.x", weeks: "nach Alpha", goal: "virtiofs ✅, Docker-Polish, Diagnose", done: false },
  { id: "P5", name: "Ingress + OIDC", weeks: "v0.2", goal: "Caddy, Dex, CA→Guests, *.localhost Alias", done: false },
] as const;

const EPICS = [
  { n: 1, title: "G0 Spike", ms: "G0", done: true },
  { n: 7, title: "Ownership / Helper", ms: "P0", done: true },
  { n: 12, title: "vsock Guest-Agent", ms: "P0", done: true },
  { n: 17, title: "CLI Contracts", ms: "P1", done: "partial" as const },
  { n: 21, title: "Clones / Identity", ms: "P1", done: true },
  { n: 25, title: "Dual-DNS + Resolver", ms: "P2", done: true },
  { n: 30, title: "Net + Policies", ms: "P2", done: true },
  { n: 34, title: "Stack Reconciler", ms: "P3", done: true },
  { n: 39, title: "Docker + Ports", ms: "P4", done: true },
  { n: 43, title: "Ingress / CA / OIDC", ms: "v0.2", done: false },
  { n: 48, title: "DX Logs / Docs", ms: "P1", done: false },
] as const;

function Struck({ children }: { children: string }) {
  return (
    <Text
      as="span"
      size="small"
      tone="secondary"
      style={{ textDecoration: "line-through" }}
    >
      {children}
    </Text>
  );
}

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
            G0–P2 done
          </Pill>
          <Pill tone="success" size="sm" active>
            Helper 1:1
          </Pill>
          <Pill tone="success" size="sm" active>
            Agent Spec+Base
          </Pill>
          <Pill tone="success" size="sm" active>
            Dual-DNS *.vz.test
          </Pill>
          <Pill tone="success" size="sm" active>
            Net+Policies
          </Pill>
          <Pill tone="success" size="sm" active>
            #36 schema
          </Pill>
          <Pill tone="success" size="sm" active>
            #52 pull
          </Pill>
          <Pill tone="success" size="sm" active>
            #37 apply
          </Pill>
          <Pill tone="warning" size="sm">
            v0.1 Alpha
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

      <Callout tone="success" title="P0 + P1 + P2 Netzwerk-Epic #30">
        P0 Foundation closed. P1: #18/#19 Contracts sowie #21 mit #22 seal,
        #23 linked clone + dataDisk und #24 Identity-Reset ✅. P2 #31/#32:
        vmnet CRUD, Router-Apply, #51 Default-Netz mit vollem NAT-Egress und
        #33 nftables Forward-Policies mit Plan/Status ✅.
      </Callout>

      <Callout tone="success" title="P3 Stack-Reconciler Epic #34 closed">
        #36 implementiert <Code>hypernetwork/v1</Code> als Serde-Typen plus
        exportierbares JSON Schema. <Code>vzctl validate -C</Code> prüft Schema,
        Referenzen, IPv4/CIDR, DependsOn-DAG und DHCP/static-Kollisionen mit
        JSON-Pfaden. #52 ergänzt 14 ARM64-<Code>*-latest</Code>-Aliase,
        Digestprüfung und den content-addressed Raw-Store. #37 ergänzt
        Plan/Diff/Up/Down/Apply, SQLite-Journal/Lease, Resume/Abort und
        UI-Events. #38 ergänzt edge-dmz Cloud-Init, README sowie reale
        Validate/Plan/Diff-CI. <Code>vzctl ps</Code> und <Code>adopt</Code>
        report-only sind im DoD; reclaim/<Code>doctor --fix-locks</Code> bleibt deferred.
      </Callout>

      <Callout tone="success" title="Builder-VM Bake/Seal auf macOS">
        Decision 25: Bake/Seal nutzen lokales <Code>virt-customize</Code> oder
        eine gepinnte Builder-Appliance (ephemerer Helper, Ziel als Data-Disk).
        Workflow <Code>pull → bake → seal</Code>. Contracts getrennt
        (<Code>bake-contract-v1</Code>, <Code>seal-contract-v1</Code>). Ops-Residual:
        Appliance einmalig auf ARM64-Linux bauen und unter{" "}
        <Code>images/builder/</Code> cachen.
      </Callout>

      <Grid columns={6} gap={12}>
        <Stat value="G0–P3" label="phases done" tone="success" />
        <Stat value="#34✓" label="stack epic" tone="success" />
        <Stat value="#36✓" label="validate" tone="success" />
        <Stat value="#38✓" label="edge-dmz" tone="success" />
        <Stat value="#52✓" label="image pull" tone="success" />
        <Stat value="#37✓" label="reconciler" tone="success" />
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
          <CardHeader trailing={<Pill tone="success" size="sm" active>ADR 0003✓</Pill>}>
            Apply-Vertrag
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Struck>
                Desired=YAML · Actual=SQLite · Spec ADR 0003 · validate (#36)
              </Struck>
              <Struck>
                Journal-Runtime · apply --resume|--abort · Lease · Lockfile (#37)
              </Struck>
              <Struck>
                edge-dmz + CI validate/plan/diff (#38) · ps · adopt report-only
              </Struck>
            </Stack>
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
            ["16–18", "IP / Isolation", "static · DNS/gw=.0 UDP · Host/Ingress .1 + PF · Router .2 · policies"],
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
            ["Guest → Host .1", "G0 FAIL → Alias", "Root-Helper + PF, nur Ingress"],
            ["FE → BE via Router .2", "ICMP OK", "DMZ-Topologie machbar"],
            ["kill -9 Monolith", "VM dead + CIDR leak", "Helper 1:1 Pflicht"],
          ]}
          rowTone={["success", "success", "warning", "danger", "success", "danger"]}
        />
      </Stack>

      <Stack gap={12}>
        <H2>DNS — Dual Listener + *.vz.test</H2>
        <Callout tone="success" title="#26–#29 implementiert">
          Supervisor-owned UDP-Listener, Actual-State A-Records, TTL 5–30s,
          Hot-Reload, System-/expliziter Forwarder sowie DNS-Health und Events.
          macOS-Resolver werden atomar, idempotent und collision-safe pro
          Projekt/Config verwaltet. Direkte UDP-Queries liefern A/AAAA,
          RCODE/Answers und CLI-v1-Exitcodes ohne /etc/resolver. Guest-NoCloud
          setzt Bridge-.0 als einzigen DNS und die Projekt-Search-Domain.
        </Callout>
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
                <Code>/etc/resolver/{"{project}"}.vz.test</Code> via{" "}
                <Code>dns install-resolver|uninstall-resolver</Code>.{" "}
                <Code>vzctl dns query</Code> spricht DNS direkt (dig umgeht oft
                Resolver).
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Guest</CardHeader>
            <CardBody>
              <Text size="small" tone="secondary">
                Listener auf Host-Bridge-<Code>.0:53/UDP</Code> (gemessen;
                Dev-Port 15353). Split-Horizon liefert je Listener nur die
                lokale Host-/Ingress-<Code>.1</Code>; ein PF-Anchor erlaubt dort
                ausschließlich konfigurierte Ingress-Ports. NoCloud setzt{" "}
                <Code>via .0 on-link</Code>, <Code>nameservers: [.0]</Code> und{" "}
                <Code>{"search: [{project}.vz.test]"}</Code>.
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
          Host-DNS/gw = <Code>.0</Code> (UDP). Geschützter Host-/Ingress-Alias ={" "}
          <Code>.1</Code>. Router = <Code>.2</Code> je Net.
          Guests <Code>.10+</Code>. <Code>routes</Code> + <Code>policies</Code>{" "}
          für DMZ.
        </Text>
      </Stack>

      <Stack gap={12}>
        <H2>Plattenmodell</H2>
        <Grid columns={3} gap={12}>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>#22✓</Pill>}>
              1. Base
            </CardHeader>
            <CardBody>
              <Struck>
                Sealed, immutable, Guest-Agent vorinstalliert. pull → bake →
                seal (Builder-VM oder lokal virt-customize).
              </Struck>
            </CardBody>
          </Card>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>#23✓</Pill>}>
              2. Linked Clone
            </CardHeader>
            <CardBody>
              <Struck>
                APFS clonefile COW Root-Disk. Pro Clone neue MAC, machine-id,
                SSH keys und instance-id.
              </Struck>
            </CardBody>
          </Card>
          <Card>
            <CardHeader trailing={<Pill tone="success" size="sm" active>#24✓ #52✓</Pill>}>
              3. dataDisk + Pull
            </CardHeader>
            <CardBody>
              <Struck>
                dataDisk pro VM · image pull *-latest Aliase · Digest/Raw-Store
              </Struck>
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
            <Pill size="sm" active={p.done || p.id === "P4"}>
              {p.done ? `${p.id}✓` : p.id}
            </Pill>,
            p.done ? <Struck>{p.name}</Struck> : p.name,
            p.weeks,
            p.done ? <Struck>{p.goal}</Struck> : p.goal,
          ])}
          rowTone={[
            "success",
            "success",
            "success",
            "success",
            "success",
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
                <Struck>Host macOS 26+ · G0 bestanden</Struck>
                <Struck>Supervisor + Helper + Agent-in-Base</Struck>
                <Struck>Dual-DNS *.vz.test + Resolver</Struck>
                <Struck>Clones · Seal · Identity · Net/Policies · image pull</Struck>
                <Struck>hypernetwork Schema + validate (#36)</Struck>
                <Struck>Stacks up/down/apply + Journal/Resume (#37)</Struck>
                <Struck>Docker SSH-Context + Ports basic (#39/#40/#41)</Struck>
                <Struck>`vzctl vm logs` (#49)</Struck>
                <Text size="small" tone="secondary">
                  v0.1.x residual: Docker-Polish / Diagnose (#48); virtiofs (#42) landed
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
    ubuntu-base: { from: ubuntu-latest, role: base }
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
        <Stack gap={4}>
          <Struck>vzctl validate · --schema</Struck>
          <Struck>vzctl vm create|… (create/from/data-disk/role)</Struck>
          <Struck>vzctl image pull|bake|seal</Struck>
          <Struck>vzctl net create|attach|list|detach|delete|default</Struck>
          <Struck>vzctl route apply|plan|status</Struck>
          <Struck>vzctl dns query|install-resolver|uninstall-resolver</Struck>
          <Struck>vzctl events subscribe · vzctl doctor</Struck>
          <Struck>vzctl plan|diff|up|down|apply [--force|--resume|--abort]</Struck>
          <Struck>vzctl ps · adopt (report-only) · vm logs</Struck>
          <Text size="small" tone="secondary">
            Offen: <Code>docker …</Code> · reclaim/fix-locks · v0.2{" "}
            <Code>ingress|certs|oidc</Code>
          </Text>
        </Stack>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>GitHub Tracking</H2>
        <Text size="small" tone="secondary">
          50 Issues · Sub-Issues · blocked-by · Labels type/priority/area/phase/finding ·
          Docs: docs/planing/06-github-tracking.md
        </Text>
        <Table
          headers={["#", "Epic", "Milestone", "Status"]}
          columnAlign={["center", "left", "left", "left"]}
          rows={EPICS.map((e) => [
            <Pill size="sm" active={e.done === true || e.done === "partial"}>
              {e.done === true ? `#${e.n}✓` : `#${e.n}`}
            </Pill>,
            e.done === true ? <Struck>{e.title}</Struck> : e.title,
            e.ms,
            e.done === true
              ? "done"
              : e.done === "partial"
                ? "contract ✓ · CLI surface rest"
                : "open",
          ])}
          rowTone={[
            "success",
            "success",
            "success",
            "info",
            "success",
            "success",
            "success",
            "success",
            "neutral",
            "warning",
            "neutral",
          ]}
        />
        <Callout tone="success" title="P4 Epic #39 closed · Next v0.1.x / v0.2">
          #40/#41: Docker SSH-Context (`vzctl docker`), cloudInit-Merge,
          DNS <Code>docker.svc</Code>, Userspace Port-Forwards
          (`vzctl port list`) und Collision-Check. Logische Docker-Netze nutzen
          für Ingress die geschützte <Code>.1</Code> der primären vmnet-NIC ihrer
          Docker-VM. #42 virtiofs (live share-swap +
          `vm mount`) ist gelandet. Weiter: DX (#48 residual) oder Ingress/OIDC (#43).
        </Callout>
      </Stack>

      <Stack gap={12}>
        <H2>Repo-Layout (Ist)</H2>
        <Code>{`vzctl/
  crates/vzctl/     # Rust CLI (validate, dns, net, image, doctor, …)
  daemon/           # vz-supervisor + vz-helper (ADR 0002)
  guest-agent/      # Go vzctl-agent + systemd + NoCloud seed
  docs/adr/         # 0001–0003 Accepted
  docs/specs/       # guest-agent-v1, cli-contract-v1, hypernetwork-v1, events-v1
  docs/images/      # seal + bake + pull contracts
  docs/spikes/      # g0/p0/p1 spikes
  examples/edge-dmz/
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
