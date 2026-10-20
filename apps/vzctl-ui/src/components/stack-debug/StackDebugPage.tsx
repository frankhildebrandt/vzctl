import { useMutation, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Alert, Button, EmptyState, PageHeader } from "@/components/ui";
import {
  fetchDebugDoctor,
  fetchDebugSources,
  fetchNatsStreams,
  runMysqlQuery,
  runNatsKvGet,
  runNatsSub,
  runPostgresQuery,
  type DebugSourceTypeSummary,
} from "@/lib/debugSources";
import { useT } from "@/lib/i18n";
import { getProject } from "@/lib/projects";
import { useEditorStore } from "@/store/editorStore";

type Props = {
  stackPath: string;
};

type SourceKind = "postgres" | "mysql" | "nats";

function pickDefault(
  summary: DebugSourceTypeSummary | undefined,
): string | undefined {
  return summary?.default;
}

export function StackDebugPage({ stackPath }: Props) {
  const t = useT();
  const project = getProject(stackPath);
  const stackId = project?.id;
  const env = useEditorStore((s) => s.env);
  const [kind, setKind] = useState<SourceKind>("postgres");
  const [sourceName, setSourceName] = useState("");
  const [sql, setSql] = useState("SELECT 1");
  const [subject, setSubject] = useState(">");
  const [bucket, setBucket] = useState("");
  const [kvKey, setKvKey] = useState("");
  const [resultJson, setResultJson] = useState("");

  const configured = env?.spec.observability?.dataSources ?? {};

  const sourcesQuery = useQuery({
    queryKey: ["debug-sources", stackId],
    queryFn: () => fetchDebugSources(stackId!),
    enabled: Boolean(stackId),
  });

  const doctorQuery = useQuery({
    queryKey: ["debug-doctor", stackId],
    queryFn: () => fetchDebugDoctor(stackId!),
    enabled: Boolean(stackId),
  });

  const typeSummary = sourcesQuery.data?.sources?.dataSources?.[kind] as
    | DebugSourceTypeSummary
    | undefined;

  const entries = useMemo(() => typeSummary?.entries ?? [], [typeSummary]);
  const activeSource = sourceName || pickDefault(typeSummary) || entries[0] || "";

  const runSql = useMutation({
    mutationFn: async () => {
      if (!stackId) throw new Error("stack not registered");
      const body = {
        source: activeSource || undefined,
        sql,
        maxRows: 100,
        timeoutMs: 30_000,
      };
      return kind === "mysql"
        ? runMysqlQuery(stackId, body)
        : runPostgresQuery(stackId, body);
    },
    onSuccess: (data) => setResultJson(JSON.stringify(data, null, 2)),
  });

  const runStreams = useMutation({
    mutationFn: async () => {
      if (!stackId) throw new Error("stack not registered");
      return fetchNatsStreams(stackId, activeSource || undefined);
    },
    onSuccess: (data) => setResultJson(JSON.stringify(data, null, 2)),
  });

  const runSub = useMutation({
    mutationFn: async () => {
      if (!stackId) throw new Error("stack not registered");
      return runNatsSub(stackId, {
        source: activeSource || undefined,
        subject,
        count: 10,
        timeoutMs: 15_000,
      });
    },
    onSuccess: (data) => setResultJson(JSON.stringify(data, null, 2)),
  });

  const runKvGet = useMutation({
    mutationFn: async () => {
      if (!stackId) throw new Error("stack not registered");
      return runNatsKvGet(stackId, {
        source: activeSource || undefined,
        bucket,
        key: kvKey,
      });
    },
    onSuccess: (data) => setResultJson(JSON.stringify(data, null, 2)),
  });

  if (!stackId) {
    return (
      <EmptyState
        title={t("debug.noStackIdTitle")}
        message={t("debug.noStackIdDesc")}
      />
    );
  }

  const doctorOk = doctorQuery.data?.doctor?.ok;
  const hasAnyConfig =
    configured.postgres || configured.mysql || configured.nats;

  return (
    <section className="detail">
      <PageHeader
        layout="detail"
        title={t("debug.title")}
        subtitle={t("debug.subtitle")}
        actions={
          <Button
            tone="secondary"
            onClick={() => void doctorQuery.refetch()}
            disabled={doctorQuery.isFetching}
          >
            {t(doctorQuery.isFetching ? "debug.doctorRunning" : "debug.doctorBtn")}
          </Button>
        }
      />

      {!hasAnyConfig ? (
        <Alert title={t("debug.noSourcesTitle")}>{t("debug.noSourcesDesc")}</Alert>
      ) : null}

      {sourcesQuery.error && (
        <Alert title={t("debug.sourcesErrorTitle")}>{String(sourcesQuery.error)}</Alert>
      )}
      {doctorQuery.isFetching ? (
        <Alert tone="info" role="status" title={t("debug.doctorRunning")} />
      ) : doctorQuery.error ? (
        <Alert title={t("debug.doctorErrorTitle")}>{String(doctorQuery.error)}</Alert>
      ) : doctorQuery.data ? (
        <Alert
          tone={doctorOk === true ? "info" : "danger"}
          role="status"
          title={t(doctorOk === true ? "debug.doctorReady" : "debug.doctorFailTitle")}
        >
          {doctorQuery.data.doctor?.checks?.length ? (
            <ul>
              {doctorQuery.data.doctor.checks.map((check, index) => (
                <li key={`${check.type}:${check.vmId ?? check.via}:${index}`}>
                  <strong>{check.tool ?? check.type}</strong>
                  {check.vmId || check.via ? ` (${check.vmId ?? check.via})` : ""}
                  {`: ${t(check.status === "ok" ? "debug.doctorCheckReady" : "debug.doctorCheckFailed")}`}
                  {check.message ? ` — ${check.message}` : ""}
                </li>
              ))}
            </ul>
          ) : (
            <p>{t(doctorOk === true ? "debug.doctorNoChecks" : "debug.doctorFailDesc")}</p>
          )}
        </Alert>
      ) : null}

      <div className="debug-toolbar" role="tablist" aria-label={t("debug.typeTabsAria")}>
        {(["postgres", "mysql", "nats"] as const).map((entry) => (
          <Button
            key={entry}
            tone={kind === entry ? "primary" : "secondary"}
            onClick={() => setKind(entry)}
            disabled={!configured[entry]}
          >
            {entry}
          </Button>
        ))}
      </div>

      {typeSummary ? (
        <p className="muted detail-meta">
          {t("debug.meta", {
            via: typeSummary.via,
            defaultSource: typeSummary.default,
          })}
        </p>
      ) : null}

      <label className="field">
        <span>{t("debug.sourceLabel")}</span>
        <select
          value={activeSource}
          onChange={(event) => setSourceName(event.target.value)}
        >
          {entries.map((entry) => (
            <option key={entry} value={entry}>
              {entry}
              {entry === typeSummary?.default ? ` (${t("debug.defaultBadge")})` : ""}
            </option>
          ))}
        </select>
      </label>

      {kind === "postgres" || kind === "mysql" ? (
        <>
          <label className="field">
            <span>{t("debug.sqlLabel")}</span>
            <textarea
              className="debug-sql"
              rows={6}
              value={sql}
              onChange={(event) => setSql(event.target.value)}
            />
          </label>
          <Button
            onClick={() => runSql.mutate()}
            disabled={runSql.isPending || !sql.trim()}
          >
            {t("debug.runQuery")}
          </Button>
        </>
      ) : (
        <>
          <div className="debug-nats-actions">
            <Button onClick={() => runStreams.mutate()} disabled={runStreams.isPending}>
              {t("debug.natsStreams")}
            </Button>
          </div>
          <label className="field">
            <span>{t("debug.natsSubject")}</span>
            <input value={subject} onChange={(e) => setSubject(e.target.value)} />
          </label>
          <Button onClick={() => runSub.mutate()} disabled={runSub.isPending}>
            {t("debug.natsSub")}
          </Button>
          <label className="field">
            <span>{t("debug.natsBucket")}</span>
            <input value={bucket} onChange={(e) => setBucket(e.target.value)} />
          </label>
          <label className="field">
            <span>{t("debug.natsKey")}</span>
            <input value={kvKey} onChange={(e) => setKvKey(e.target.value)} />
          </label>
          <Button
            onClick={() => runKvGet.mutate()}
            disabled={runKvGet.isPending || !bucket || !kvKey}
          >
            {t("debug.natsKvGet")}
          </Button>
        </>
      )}

      {(runSql.error ||
        runStreams.error ||
        runSub.error ||
        runKvGet.error) && (
        <Alert title={t("debug.errorTitle")}>
          {String(
            runSql.error ??
              runStreams.error ??
              runSub.error ??
              runKvGet.error,
          )}
        </Alert>
      )}

      {resultJson ? (
        <pre className="debug-result" aria-label={t("debug.resultAria")}>
          {resultJson}
        </pre>
      ) : null}
    </section>
  );
}
