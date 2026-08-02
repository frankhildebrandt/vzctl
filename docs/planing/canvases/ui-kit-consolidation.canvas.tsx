import {
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

/**
 * UI-Kit-Konsolidierung für vzctl-ui.
 * Kein Tailwind — Custom-CSS + semantische React-Komponenten unter components/ui/.
 */
export default function UiKitConsolidationCanvas() {
  const theme = useHostTheme();
  const muted = theme === "dark" ? "#9CA3AF" : "#6B7280";

  return (
    <Stack gap={24} style={{ padding: 24, maxWidth: 1100 }}>
      <Stack gap={8}>
        <H1>vzctl-ui · UI-Kit-Konsolidierung</H1>
        <Text tone="secondary" style={{ color: muted }}>
          Designgebende CSS-/JSX-Muster → wiederverwendbare React-Komponenten.
          Visuelles Erscheinungsbild bleibt über bestehende Tokens in styles.css.
        </Text>
        <Row gap={8}>
          <Pill tone="success">PR #59</Pill>
          <Pill>Branch cursor/ui-kit-consolidation-9d33</Pill>
          <Pill tone="neutral">kein Tailwind</Pill>
        </Row>
      </Stack>

      <Callout tone="info" title="Befund">
        Vorher: Ad-hoc-Klassen in Seiten (`.card`, `.badge`, `.secondary`,
        Dialog-Portal-Duplikate). Kein formales React-UI-Kit. Design-System lebte
        in CSS-Variablen + semantischen Klassen.
      </Callout>

      <Grid columns={3} gap={16}>
        <Stat value="16" label="Kit-Module" />
        <Stat value="~40" label="migrierte Dateien" />
        <Stat value="2" label="Dialoge auf Shared Shell" />
      </Grid>

      <Divider />

      <H2>Neues Kit · components/ui/</H2>
      <Table
        headers={["Komponente", "Varianten", "Ersetzt"]}
        rows={[
          ["Button", "primary | secondary | danger", "<button className=secondary|danger>"],
          ["Card", "tone, padding, as=form", ".card / .error-card / .summary-card"],
          ["PageHeader", "row | detail", "section-title + muted + row"],
          ["ActionRow", "align, gap", "inline justifyContent/gap"],
          ["Badge / StatusPill / StackPhasePill", "tone / state / phase", ".badge* / .vm-state / .stack-pill"],
          ["Alert / EmptyState / LoadingState", "tone, card?", "error-card / muted Loading"],
          ["FormGrid / FormField / FormActions", "grid | compact", ".form-grid / .topology-field"],
          ["Dialog", "shared shell", "ConfirmDialog + VmnetOrphanDialog Duplikat"],
          ["DescriptionList", "stacked", ".kv (+ .kv-row fix)"],
          ["TableCard / DataTable", "—", ".card + .vm-table / .data-table"],
          ["SelectableCard", "theme | locale | preset", "theme-card / provider-preset"],
          ["CodeBlock / CopyButton / JsonBlock", "copyable", "out-wrap / out-copy"],
          ["SummaryCard", "badgeTone", ".summary-card + .summary-row"],
          ["cx", "—", "template-string class joins"],
        ]}
      />

      <Divider />

      <H2>Priorität & Umsetzung</H2>
      <Grid columns={2} gap={16}>
        <Card>
          <CardHeader>
            <H3>Umgesetzt</H3>
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text>• Kit + Barrel-Exports</Text>
              <Text>• Seiten/Forms/Dialoge migriert</Text>
              <Text>• NetworksPage: undefinierte Klassen bereinigt</Text>
              <Text>• Dialog-Shell zusammengeführt</Text>
              <Text>• .kv-row → display: contents</Text>
              <Text>• Topology Error/Buttons/NameField</Text>
              <Text>• ApplyProgress Cards</Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader>
            <H3>Bewusst offen</H3>
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text>• Stack DetailHeader (Tabs/Toolbar-Sonderfall)</Text>
              <Text>• Topology Palette/ContextMenu (feature-nah)</Text>
              <Text>• ingress-chip / action-chip (fachlich eng)</Text>
              <Text>• AppShell Layout-Struktur</Text>
              <Text>• X6-Canvas-Styling</Text>
            </Stack>
          </CardBody>
        </Card>
      </Grid>

      <Divider />

      <H2>Validierung</H2>
      <Table
        headers={["Check", "Ergebnis"]}
        rows={[
          ["tsc + vite build", "OK"],
          ["vitest (UI-Kit cx/contracts)", "neu"],
          ["vitest domain.test edge-dmz CIDR", "pre-existing Fail (unabhängig)"],
          ["Visuelle Regression", "manuell im PR-Testplan"],
        ]}
      />

      <Callout tone="warning" title="Technische Schuld">
        Weitere Kandidaten nur mit Design-Entscheidung: generisches ContextMenu,
        DetailHeader für Stack-Tabs, Zusammenführung Badge↔ActionChip. Nicht
        automatisch zusammengeführt, um APIs nicht zu überfrachten.
      </Callout>
    </Stack>
  );
}
