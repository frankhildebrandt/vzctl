import {
  BarChart,
  Callout,
  Card,
  CardBody,
  CardHeader,
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
  useHostTheme,
} from "cursor/canvas";

type Score = 1 | 2 | 3 | 4 | 5;

/** Baseline: menschlicher Bau-/Wartungsaufwand zählt gegen Self-made. */
const SCORES = {
  ansible: { utm: 2, multipass: 5, hyperkit: 4 },
  network: { utm: 4, multipass: 3, hyperkit: 5 },
  k8s: { utm: 2, multipass: 5, hyperkit: 3 },
  docker: { utm: 3, multipass: 5, hyperkit: 4 },
  agentic: { utm: 2, multipass: 5, hyperkit: 4 },
  scriptability: { utm: 2, multipass: 5, hyperkit: 4 },
  flexibility: { utm: 5, multipass: 3, hyperkit: 5 },
} as const;

/**
 * Capability unter Agent/∞ Tokens — Peak-Fähigkeit, nicht Alltag.
 */
const SCORES_AGENT = {
  ansible: { utm: 2, multipass: 5, hyperkit: 5 },
  network: { utm: 4, multipass: 3, hyperkit: 5 },
  k8s: { utm: 2, multipass: 5, hyperkit: 5 },
  docker: { utm: 3, multipass: 5, hyperkit: 5 },
  agentic: { utm: 2, multipass: 5, hyperkit: 5 },
  scriptability: { utm: 2, multipass: 5, hyperkit: 5 },
  flexibility: { utm: 5, multipass: 3, hyperkit: 5 },
} as const;

/**
 * Produktive Dev-Arbeit: Zuverlässigkeit, Feedback-Loop, Unblock-Zeit,
 * Mounts, Ressourcen, Team — nicht Peak-Capability.
 * Self-made+Agent: Capability hoch, Ownership-Risiko bleibt.
 */
const SCORES_PROD = {
  reliability: { utm: 3, multipass: 5, hyperkit: 2, agent: 3 },
  feedback: { utm: 2, multipass: 5, hyperkit: 3, agent: 4 },
  resources: { utm: 2, multipass: 4, hyperkit: 3, agent: 4 },
  hostReality: { utm: 3, multipass: 4, hyperkit: 2, agent: 3 },
  unblock: { utm: 3, multipass: 5, hyperkit: 1, agent: 3 },
  team: { utm: 2, multipass: 5, hyperkit: 1, agent: 3 },
  ansible: { utm: 2, multipass: 5, hyperkit: 3, agent: 4 },
  network: { utm: 3, multipass: 4, hyperkit: 4, agent: 5 },
  k8s: { utm: 2, multipass: 4, hyperkit: 2, agent: 4 },
  docker: { utm: 2, multipass: 5, hyperkit: 3, agent: 4 },
  agentic: { utm: 2, multipass: 4, hyperkit: 3, agent: 5 },
} as const;

function avg(vals: number[]): number {
  return Math.round((vals.reduce((a, b) => a + b, 0) / vals.length) * 10) / 10;
}

const utmOverall = avg(Object.values(SCORES).map((s) => s.utm));
const mpOverall = avg(Object.values(SCORES).map((s) => s.multipass));
const hkOverall = avg(Object.values(SCORES).map((s) => s.hyperkit));
const hkAgentOverall = avg(Object.values(SCORES_AGENT).map((s) => s.hyperkit));

const utmProd = avg(Object.values(SCORES_PROD).map((s) => s.utm));
const mpProd = avg(Object.values(SCORES_PROD).map((s) => s.multipass));
const hkProd = avg(Object.values(SCORES_PROD).map((s) => s.hyperkit));
const hkAgentProd = avg(Object.values(SCORES_PROD).map((s) => s.agent));

const CATEGORIES = [
  "Ansible",
  "Netzwerk",
  "Kubernetes",
  "Docker",
  "Agentisch",
  "Scriptbarkeit",
  "Flexibilität",
] as const;

const PROD_CATEGORIES = [
  "Zuverlässigkeit",
  "Feedback-Loop",
  "Ressourcen",
  "Host-Realität",
  "Unblock",
  "Team",
  "Ansible",
  "Netz (Prod)",
  "K8s (Prod)",
  "Docker (Prod)",
  "Agentisch",
] as const;

function ScorePill({ n }: { n: Score }) {
  const tone = n >= 5 ? "success" : n >= 4 ? "info" : n >= 3 ? "warning" : "deleted";
  return (
    <Pill tone={tone} size="sm" active={n >= 4}>
      {n}/5
    </Pill>
  );
}

function Delta({ from, to }: { from: Score; to: Score }) {
  if (to === from) {
    return (
      <Text size="small" tone="tertiary">
        —
      </Text>
    );
  }
  const up = to > from;
  return (
    <Pill tone={up ? "success" : "deleted"} size="sm" active>
      {from}→{to}
    </Pill>
  );
}

function CandidateCard({
  title,
  subtitle,
  overall,
  tone,
  bullets,
  caveat,
}: {
  title: string;
  subtitle: string;
  overall: number;
  tone: "success" | "info" | "warning" | "danger";
  bullets: string[];
  caveat: string;
}) {
  return (
    <Card>
      <CardHeader trailing={<Stat value={String(overall)} label="/ 5 Ø" tone={tone} />}>
        {title}
      </CardHeader>
      <CardBody>
        <Stack gap={10}>
          <Text size="small" tone="tertiary">
            {subtitle}
          </Text>
          <Stack gap={6}>
            {bullets.map((b) => (
              <Text size="small" tone="secondary">
                {b}
              </Text>
            ))}
          </Stack>
          <Text size="small" weight="semibold" tone="tertiary">
            {caveat}
          </Text>
        </Stack>
      </CardBody>
    </Card>
  );
}

export default function UtmMultipassHyperkitVergleich() {
  const theme = useHostTheme();

  return (
    <Stack gap={24} style={{ padding: 24, maxWidth: 1100 }}>
      <Stack gap={8}>
        <H1>UTM · Multipass · HyperKit Self-made</H1>
        <Text tone="secondary">
          Drei Bewertungsebenen: Capability-Baseline, Agent/∞-Peak, und
          produktive Dev-Arbeit (Zuverlässigkeit, Feedback-Loop, Unblock).
        </Text>
        <Row gap={8} wrap>
          <Pill size="sm">1–5 Skala</Pill>
          <Pill tone="info" size="sm">
            Jul 2026
          </Pill>
          <Pill tone="success" size="sm" active>
            Fokus: produktive Dev-Arbeit
          </Pill>
        </Row>
      </Stack>

      <Callout tone="warning" title="Produktiver Alltag ≠ Peak-Capability">
        Unlimited Tokens heben, was Agents bauen können. Produktive Arbeit
        fragt: bricht die Umgebung mitten im Sprint? Wie schnell ist
        Edit→Test? Wer unblocked dich in 10 Minuten? Unter dieser Linse bleibt
        Multipass oft Primär — auch wenn Self-made+Agent auf dem Papier 5/5
        Capability hat.
      </Callout>

      <Stack gap={8}>
        <H2>Drei Gesamt-Scores im Vergleich</H2>
        <Grid columns={3} gap={12}>
          <Card>
            <CardHeader>Capability Baseline</CardHeader>
            <CardBody>
              <Grid columns={3} gap={8}>
                <Stat value={String(utmOverall)} label="UTM" tone="warning" />
                <Stat value={String(mpOverall)} label="MP" tone="success" />
                <Stat value={String(hkOverall)} label="Self" tone="info" />
              </Grid>
              <Text size="small" tone="tertiary">
                Gewinner: Multipass
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Capability Agent/∞</CardHeader>
            <CardBody>
              <Grid columns={3} gap={8}>
                <Stat value={String(utmOverall)} label="UTM" tone="warning" />
                <Stat value={String(mpOverall)} label="MP" tone="info" />
                <Stat value={String(hkAgentOverall)} label="Self+" tone="success" />
              </Grid>
              <Text size="small" tone="tertiary">
                Gewinner: Self-made+Agent
              </Text>
            </CardBody>
          </Card>
          <Card>
            <CardHeader>Produktive Dev-Arbeit</CardHeader>
            <CardBody>
              <Grid columns={2} gap={8}>
                <Stat value={String(mpProd)} label="Multipass" tone="success" />
                <Stat value={String(hkAgentProd)} label="Self+Agent" tone="info" />
              </Grid>
              <Text size="small" tone="tertiary">
                UTM {utmProd} · Self Baseline {hkProd} · Gewinner: Multipass
              </Text>
            </CardBody>
          </Card>
        </Grid>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>Produktive Dev-Arbeit — Kriterien</H2>
        <Text size="small" tone="tertiary">
          Was zählt im Tagesgeschäft: nicht „kann man k3s orchestrieren?“,
          sondern „läuft mein Stack während ich Features shippe?“
        </Text>

        <Table
          headers={[
            "Kriterium",
            "UTM",
            "Multipass",
            "Self",
            "Self+Agent",
            "Warum für Prod",
          ]}
          columnAlign={["left", "center", "center", "center", "center", "left"]}
          rows={[
            [
              "Tages-Zuverlässigkeit",
              <ScorePill n={SCORES_PROD.reliability.utm} />,
              <ScorePill n={SCORES_PROD.reliability.multipass} />,
              <ScorePill n={SCORES_PROD.reliability.hyperkit} />,
              <ScorePill n={SCORES_PROD.reliability.agent} />,
              "Broken Hypervisor = toter Sprint. Fremdprodukt > Eigenbau.",
            ],
            [
              "Feedback-Loop (Mount/Sync)",
              <ScorePill n={SCORES_PROD.feedback.utm} />,
              <ScorePill n={SCORES_PROD.feedback.multipass} />,
              <ScorePill n={SCORES_PROD.feedback.hyperkit} />,
              <ScorePill n={SCORES_PROD.feedback.agent} />,
              "multipass mount ist langweilig-gut; UTM oft Reibung.",
            ],
            [
              "CPU/RAM/Akku neben IDE",
              <ScorePill n={SCORES_PROD.resources.utm} />,
              <ScorePill n={SCORES_PROD.resources.multipass} />,
              <ScorePill n={SCORES_PROD.resources.hyperkit} />,
              <ScorePill n={SCORES_PROD.resources.agent} />,
              "QEMU/GUI schwer; schlanke Cloud-VMs besser für Parallel-Arbeit.",
            ],
            [
              "Sleep, VPN, MDM, Updates",
              <ScorePill n={SCORES_PROD.hostReality.utm} />,
              <ScorePill n={SCORES_PROD.hostReality.multipass} />,
              <ScorePill n={SCORES_PROD.hostReality.hyperkit} />,
              <ScorePill n={SCORES_PROD.hostReality.agent} />,
              "Corporate-Mac-Realität bricht DIY-Netz härter als Canonical.",
            ],
            [
              "Unblock in Minuten",
              <ScorePill n={SCORES_PROD.unblock.utm} />,
              <ScorePill n={SCORES_PROD.unblock.multipass} />,
              <ScorePill n={SCORES_PROD.unblock.hyperkit} />,
              <ScorePill n={SCORES_PROD.unblock.agent} />,
              "∞ Tokens ≠ Fix in 10 Min. Community/Docs schlagen Eigenbau.",
            ],
            [
              "Team / Onboarding",
              <ScorePill n={SCORES_PROD.team.utm} />,
              <ScorePill n={SCORES_PROD.team.multipass} />,
              <ScorePill n={SCORES_PROD.team.hyperkit} />,
              <ScorePill n={SCORES_PROD.team.agent} />,
              "brew install multipass; Self-made = Plattform schulen.",
            ],
            [
              "Ansible im Alltag",
              <ScorePill n={SCORES_PROD.ansible.utm} />,
              <ScorePill n={SCORES_PROD.ansible.multipass} />,
              <ScorePill n={SCORES_PROD.ansible.hyperkit} />,
              <ScorePill n={SCORES_PROD.ansible.agent} />,
              "Prod = wiederholbare VMs ohne Plattform-Debug dazwischen.",
            ],
            [
              "Netz für Services",
              <ScorePill n={SCORES_PROD.network.utm} />,
              <ScorePill n={SCORES_PROD.network.multipass} />,
              <ScorePill n={SCORES_PROD.network.hyperkit} />,
              <ScorePill n={SCORES_PROD.network.agent} />,
              "Prod braucht oft nur Host↔Guest-Ports — nicht Lab-Topologie.",
            ],
            [
              "K8s produktiv lokal",
              <ScorePill n={SCORES_PROD.k8s.utm} />,
              <ScorePill n={SCORES_PROD.k8s.multipass} />,
              <ScorePill n={SCORES_PROD.k8s.hyperkit} />,
              <ScorePill n={SCORES_PROD.k8s.agent} />,
              "Cluster darf nicht selbst das Ticket sein.",
            ],
            [
              "Docker produktiv",
              <ScorePill n={SCORES_PROD.docker.utm} />,
              <ScorePill n={SCORES_PROD.docker.multipass} />,
              <ScorePill n={SCORES_PROD.docker.hyperkit} />,
              <ScorePill n={SCORES_PROD.docker.agent} />,
              "Build/Compose-Loop; Mount-Latenz zählt mehr als Hypervisor-Flex.",
            ],
            [
              "Agenten im Dev-Flow",
              <ScorePill n={SCORES_PROD.agentic.utm} />,
              <ScorePill n={SCORES_PROD.agentic.multipass} />,
              <ScorePill n={SCORES_PROD.agentic.hyperkit} />,
              <ScorePill n={SCORES_PROD.agentic.agent} />,
              "Self+Agent API stark — solange die VM nicht tot ist.",
            ],
          ]}
          rowTone={[
            "success",
            "success",
            "info",
            "warning",
            "success",
            "success",
            "success",
            "info",
            "info",
            "success",
            "info",
          ]}
        />
      </Stack>

      <Stack gap={8}>
        <H2>Prod-Score-Radar</H2>
        <Text size="small" tone="tertiary">
          Produktive Dev-Arbeit · Self Baseline vs Self+Agent vs Multipass/UTM
        </Text>
        <BarChart
          categories={[...PROD_CATEGORIES]}
          series={[
            {
              name: "UTM",
              data: [
                SCORES_PROD.reliability.utm,
                SCORES_PROD.feedback.utm,
                SCORES_PROD.resources.utm,
                SCORES_PROD.hostReality.utm,
                SCORES_PROD.unblock.utm,
                SCORES_PROD.team.utm,
                SCORES_PROD.ansible.utm,
                SCORES_PROD.network.utm,
                SCORES_PROD.k8s.utm,
                SCORES_PROD.docker.utm,
                SCORES_PROD.agentic.utm,
              ],
              tone: "warning",
            },
            {
              name: "Multipass",
              data: [
                SCORES_PROD.reliability.multipass,
                SCORES_PROD.feedback.multipass,
                SCORES_PROD.resources.multipass,
                SCORES_PROD.hostReality.multipass,
                SCORES_PROD.unblock.multipass,
                SCORES_PROD.team.multipass,
                SCORES_PROD.ansible.multipass,
                SCORES_PROD.network.multipass,
                SCORES_PROD.k8s.multipass,
                SCORES_PROD.docker.multipass,
                SCORES_PROD.agentic.multipass,
              ],
              tone: "success",
            },
            {
              name: "Self+Agent",
              data: [
                SCORES_PROD.reliability.agent,
                SCORES_PROD.feedback.agent,
                SCORES_PROD.resources.agent,
                SCORES_PROD.hostReality.agent,
                SCORES_PROD.unblock.agent,
                SCORES_PROD.team.agent,
                SCORES_PROD.ansible.agent,
                SCORES_PROD.network.agent,
                SCORES_PROD.k8s.agent,
                SCORES_PROD.docker.agent,
                SCORES_PROD.agentic.agent,
              ],
              tone: "info",
            },
          ]}
          yMax={5}
          height={300}
        />
      </Stack>

      <Grid columns={2} gap={12}>
        <Card>
          <CardHeader>Wo Prod Multipass gewinnt</CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small" tone="secondary">
                Feature-Arbeit an Apps/APIs/Ansible-Rollen mit Linux-Target
              </Text>
              <Text size="small" tone="secondary">
                Docker Compose / Dev-Services neben Cursor/IDE
              </Text>
              <Text size="small" tone="secondary">
                Team-Repos, CI-ähnliche lokale Smoke-Tests
              </Text>
              <Text size="small" tone="secondary">
                Wenn der Hypervisor unsichtbar bleiben soll
              </Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader>Wo Prod Self+Agent trotzdem gewinnt</CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small" tone="secondary">
                Das Produkt ist Netzwerk-/Infra-Topologie (Multi-NIC, Firewall)
              </Text>
              <Text size="small" tone="secondary">
                Agent-Orchestrierung ist das Produkt — eigene Tool-API Pflicht
              </Text>
              <Text size="small" tone="secondary">
                Festes Platform-Team hält den Stack (nicht jeder Dev)
              </Text>
              <Text size="small" tone="secondary">
                Canonical-Grenzen (nur Ubuntu, Netz-Decke) blockieren Features
              </Text>
            </Stack>
          </CardBody>
        </Card>
      </Grid>

      <Callout tone="info" title="Prod-Regel">
        Self-made+Agent nur als produktive Primärumgebung, wenn die
        Virtualisierungs-Flexibilität Teil des Liefergegenstands ist — oder ein
        dediziertes Platform-Team den Break/Fix-Loop trägt. Sonst: Multipass für
        den Coding-Tag, Self-made/UTM als spezialisiertes Lab daneben.
      </Callout>

      <Divider />

      <Stack gap={8}>
        <H2>Capability Agent/∞ (Erinnerung)</H2>
        <Text size="small" tone="tertiary">
          Peak-Fähigkeit — hier kippt das Ranking zugunsten Self-made. Prod
          oben bewertet Alltag, nicht Maximum.
        </Text>
        <BarChart
          categories={[...CATEGORIES]}
          series={[
            {
              name: "UTM",
              data: [
                SCORES_AGENT.ansible.utm,
                SCORES_AGENT.network.utm,
                SCORES_AGENT.k8s.utm,
                SCORES_AGENT.docker.utm,
                SCORES_AGENT.agentic.utm,
                SCORES_AGENT.scriptability.utm,
                SCORES_AGENT.flexibility.utm,
              ],
              tone: "warning",
            },
            {
              name: "Multipass",
              data: [
                SCORES_AGENT.ansible.multipass,
                SCORES_AGENT.network.multipass,
                SCORES_AGENT.k8s.multipass,
                SCORES_AGENT.docker.multipass,
                SCORES_AGENT.agentic.multipass,
                SCORES_AGENT.scriptability.multipass,
                SCORES_AGENT.flexibility.multipass,
              ],
              tone: "info",
            },
            {
              name: "Self-made (Agent/∞)",
              data: [
                SCORES_AGENT.ansible.hyperkit,
                SCORES_AGENT.network.hyperkit,
                SCORES_AGENT.k8s.hyperkit,
                SCORES_AGENT.docker.hyperkit,
                SCORES_AGENT.agentic.hyperkit,
                SCORES_AGENT.scriptability.hyperkit,
                SCORES_AGENT.flexibility.hyperkit,
              ],
              tone: "success",
            },
          ]}
          yMax={5}
          height={240}
        />
      </Stack>

      <Stack gap={12}>
        <H2>Self-made: Capability vs. Prod unter Agent/∞</H2>
        <Table
          headers={["Dimension", "Capability Agent/∞", "Prod Agent/∞", "Gap"]}
          columnAlign={["left", "center", "center", "center"]}
          rows={[
            [
              "Ansible",
              <ScorePill n={SCORES_AGENT.ansible.hyperkit} />,
              <ScorePill n={SCORES_PROD.ansible.agent} />,
              <Delta from={5} to={4} />,
            ],
            [
              "Netzwerk",
              <ScorePill n={SCORES_AGENT.network.hyperkit} />,
              <ScorePill n={SCORES_PROD.network.agent} />,
              <Text size="small" tone="tertiary">
                —
              </Text>,
            ],
            [
              "Kubernetes",
              <ScorePill n={SCORES_AGENT.k8s.hyperkit} />,
              <ScorePill n={SCORES_PROD.k8s.agent} />,
              <Delta from={5} to={4} />,
            ],
            [
              "Docker",
              <ScorePill n={SCORES_AGENT.docker.hyperkit} />,
              <ScorePill n={SCORES_PROD.docker.agent} />,
              <Delta from={5} to={4} />,
            ],
            [
              "Agentisch",
              <ScorePill n={SCORES_AGENT.agentic.hyperkit} />,
              <ScorePill n={SCORES_PROD.agentic.agent} />,
              <Text size="small" tone="tertiary">
                —
              </Text>,
            ],
            [
              "Zuverlässigkeit / Unblock",
              <Text size="small" tone="tertiary">
                n/a in Capability
              </Text>,
              <Row gap={6}>
                <ScorePill n={SCORES_PROD.reliability.agent} />
                <ScorePill n={SCORES_PROD.unblock.agent} />
              </Row>,
              <Pill tone="deleted" size="sm" active>
                Prod-Bremse
              </Pill>,
            ],
          ]}
          rowTone={["warning", "success", "warning", "warning", "success", "danger"]}
        />
        <Text size="small" tone="tertiary">
          Der Gap entsteht nicht aus fehlender Feature-Fähigkeit, sondern aus
          Ownership: wer trägt den Break während der Feature-Arbeit?
        </Text>
      </Stack>

      <Divider />

      <Stack gap={12}>
        <H2>Empfehlung nach Arbeitsmodus</H2>
        <Table
          headers={["Arbeitsmodus", "Primär", "Daneben", "Warum"]}
          rows={[
            [
              "Tägliche Feature-Dev (App/API/Rollen)",
              <Pill tone="success" size="sm" active>
                Multipass
              </Pill>,
              "UTM nur für spezielle Guests",
              "Unsichtbarer Linux-Host; Mounts; Team-fähig",
            ],
            [
              "Ansible/Docker/K8s Smoke im Sprint",
              <Pill tone="success" size="sm" active>
                Multipass
              </Pill>,
              "Self+Agent Lab",
              "Prod-Scores; Lab nur wenn Canonical eng wird",
            ],
            [
              "Netz-Topologie ist das Feature",
              <Pill tone="info" size="sm" active>
                Self+Agent
              </Pill>,
              "UTM",
              "Einziger voller Multi-NIC-Pfad — Prod-Risiko akzeptieren",
            ],
            [
              "Agent-Orchestrierung ist das Produkt",
              <Pill tone="info" size="sm" active>
                Self+Agent
              </Pill>,
              "Multipass Bootstrap",
              "Eigene Tool-API; Multipass bis V1 steht",
            ],
            [
              "Windows / ISO / GUI-Debugging",
              <Pill tone="warning" size="sm" active>
                UTM
              </Pill>,
              "—",
              "Nicht der tägliche App-Loop",
            ],
            [
              "Solo + ∞ Tokens, Platform = Hobby",
              <Pill tone="info" size="sm" active>
                Self+Agent
              </Pill>,
              "Multipass Fallback",
              "Capability-Sieg ok, solange du den Break selbst trägst",
            ],
            [
              "Team ohne Platform-Owner",
              <Pill tone="success" size="sm" active>
                Multipass
              </Pill>,
              "Shared cloud-init Repo",
              "Onboarding und Unblock schlagen Peak-Flex",
            ],
          ]}
          rowTone={[
            "success",
            "success",
            "info",
            "info",
            "warning",
            "info",
            "success",
          ]}
        />
      </Stack>

      <Grid columns={3} gap={12}>
        <CandidateCard
          title="UTM"
          subtitle={`Prod Ø ${utmProd}`}
          overall={utmProd}
          tone="warning"
          bullets={[
            "Zu schwer und GUI-lastig für den Coding-Tag.",
            "Gut als Neben-Lab für Windows/exotische Guests.",
            "Nicht die Umgebung, in der Features entstehen.",
          ]}
          caveat="Prod-Rolle: Spezialwerkzeug, nicht Primary."
        />
        <CandidateCard
          title="Multipass"
          subtitle={`Prod Ø ${mpProd}`}
          overall={mpProd}
          tone="success"
          bullets={[
            "Gewinner produktiver Dev-Arbeit — auch gegen Self+Agent.",
            "Mounts, CLI, Team, Unblock, „langweilig stabil“.",
            "Netz-Decke und Ubuntu-Fokus bleiben die Grenzen.",
          ]}
          caveat="Default für den Sprint — Capability-Max ist Nebensache."
        />
        <CandidateCard
          title="Self+Agent"
          subtitle={`Prod Ø ${hkAgentProd} · Cap ${hkAgentOverall}`}
          overall={hkAgentProd}
          tone="info"
          bullets={[
            "Capability-König, Prod-Zweiter: Ownership kostet Fokus.",
            "Primär wenn Netz/API das Lieferobjekt ist.",
            "Sonst: Lab + Platform-Team, nicht jeder Dev-Laptop.",
          ]}
          caveat="∞ Tokens heben Bau — nicht den Interrupt mitten im PR."
        />
      </Grid>

      <Callout tone="success" title="Pragmatischer Default (Prod)">
        Für produktive Dev-Arbeit: Multipass als tägliche Primärumgebung.
        Self-made (VZ, agentisch) als Platform-/Lab-Track, wenn Topologie oder
        Agent-API zum Produkt gehören — mit klarem Owner für Break/Fix. UTM für
        Guests außerhalb Ubuntu-Cloud. Agent/∞ ändert Peak-Capability, nicht die
        Tatsache, dass unterbrochene Entwickler teurer sind als Tokens.
      </Callout>

      <Text size="small" tone="tertiary" style={{ color: theme.text.tertiary }}>
        Annahmen Prod: Solo oder kleines Team auf macOS; parallele IDE-Nutzung;
        Corporate-VPN/Sleep möglich; „produktiv“ = Feature-Durchsatz, nicht
        Hypervisor-Perfektion. HyperKit-Legacy → VZ-Muster.
      </Text>
    </Stack>
  );
}
