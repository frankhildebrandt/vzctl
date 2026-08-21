import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  DescriptionList,
  LoadingState,
  Mono,
  Muted,
  PageHeader,
  SummaryCard,
  type BadgeTone,
} from "@/components/ui";
import { getT, useT } from "@/lib/i18n";
import { assertEnvelopeOk, parseEnvelope } from "@/lib/vzctl";

type DoctorCheck = {
  id: string;
  status: "ok" | "warn" | "fail" | string;
  message: string;
  details?: Record<string, unknown> | null;
};

const doctorKeys = {
  all: ["vzctl", "doctor"] as const,
};

export function DoctorPage() {
  const t = useT();
  const queryClient = useQueryClient();
  const [actionMsg, setActionMsg] = useState<string | null>(null);

  const doctorQuery = useQuery({
    queryKey: doctorKeys.all,
    queryFn: loadDoctor,
  });

  const installCa = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/certs/ca/install"));
      assertEnvelopeOk(envelope, getT()("doctor.caInstallFail"));
      return envelope;
    },
    onSuccess: () => {
      setActionMsg(t("doctor.caInstallOk"));
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const initCa = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/certs/ca/init"));
      assertEnvelopeOk(envelope, getT()("doctor.caInitFail"));
      return envelope;
    },
    onSuccess: () => {
      setActionMsg(t("doctor.caInitOk"));
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const installBindHelper = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/dns/bind-helper"));
      assertEnvelopeOk(envelope, getT()("doctor.bindInstallFail"));
      return envelope;
    },
    onSuccess: () => {
      setActionMsg(t("doctor.bindInstallOk"));
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const restartEdge = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/services/edge/restart"));
      assertEnvelopeOk(envelope, getT()("doctor.edgeRestartFail"));
      return envelope;
    },
    onSuccess: () => {
      setActionMsg(t("doctor.edgeRestartOk"));
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const checks = doctorQuery.data?.checks ?? [];
  const summary = doctorQuery.data?.summary;
  const hostTrust = useMemo(
    () => checks.find((c) => c.id === "certs.host_trust"),
    [checks],
  );
  const bindHelper = useMemo(
    () => checks.find((c) => c.id === "dns.bind_helper"),
    [checks],
  );
  const trustDetails = asRecord(hostTrust?.details);
  const bindDetails = asRecord(bindHelper?.details);
  const caPresent = Boolean(trustDetails?.present);
  const caTrusted = Boolean(trustDetails?.trusted);
  const bindReady = bindHelper?.status === "ok";
  const bindNeedsInstall =
    Boolean(bindDetails?.requires_helper) && !bindReady;
  const busy =
    installCa.isPending ||
    initCa.isPending ||
    installBindHelper.isPending ||
    restartEdge.isPending ||
    doctorQuery.isFetching;

  return (
    <section>
      <PageHeader
        layout="detail"
        title={t("doctor.title")}
        subtitle={t("doctor.subtitle")}
      />

      <SummaryCard
        badge={
          <Badge tone={badgeTone(doctorQuery.data?.status ?? "ok")}>
            {doctorQuery.data?.status ?? (doctorQuery.isLoading ? t("common.ellipsis") : t("common.emDash"))}
          </Badge>
        }
        meta={
          summary ? (
            <Muted as="span">
              {t("doctor.summaryOk", {
                ok: String(summary.ok ?? 0),
                warnings: String(summary.warnings ?? 0),
                failures: String(summary.failures ?? 0),
              })}
            </Muted>
          ) : null
        }
        actions={
          <Button tone="secondary" disabled={busy} onClick={() => void doctorQuery.refetch()}>
            {t("doctor.refresh")}
          </Button>
        }
      >
        {doctorQuery.isError ? (
          <p className="tile-error">{String(doctorQuery.error)}</p>
        ) : null}
        {actionMsg ? <Muted>{actionMsg}</Muted> : null}
      </SummaryCard>

      <Card
        title={t("doctor.bindTitle")}
        titleAs="h3"
        actions={
          <Badge tone={bindReady ? "ok" : "warn"}>
            {bindReady ? t("doctor.bindReady") : t("doctor.bindMissing")}
          </Badge>
        }
      >
        <Muted>{t("doctor.bindHint")}</Muted>
        <DescriptionList
          stacked
          items={[
            {
              label: t("doctor.bindSocket"),
              value: (
                <span title={String(bindDetails?.socket ?? "")}>
                  {bindDetails?.socket_connectable
                    ? t("doctor.bindSocketOk")
                    : bindDetails?.socket != null
                      ? String(bindDetails.socket)
                      : t("common.emDash")}
                </span>
              ),
            },
            {
              label: t("doctor.bindGuestPort"),
              value:
                bindDetails?.guest_port != null
                  ? String(bindDetails.guest_port)
                  : t("common.emDash"),
            },
          ]}
        />
        <div className="doctor-actions">
          {bindNeedsInstall ? (
            <Button
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                installBindHelper.mutate();
              }}
            >
              {t("doctor.bindInstall")}
            </Button>
          ) : null}
          {bindReady ? (
            <Muted>{t("doctor.bindActive")}</Muted>
          ) : null}
        </div>
      </Card>

      <Card
        title={t("doctor.caTitle")}
        titleAs="h3"
        actions={
          <Badge tone={caTrusted ? "ok" : caPresent ? "warn" : "ok"}>
            {caTrusted
              ? t("doctor.caTrusted")
              : caPresent
                ? t("doctor.caNotTrusted")
                : t("doctor.caNone")}
          </Badge>
        }
      >
        <Muted>{t("doctor.caHint")}</Muted>
        <DescriptionList
          stacked
          items={[
            {
              label: t("doctor.caFingerprint"),
              value: (
                <span title={String(trustDetails?.fingerprint ?? "")}>
                  {shortFp(
                    trustDetails?.fingerprint != null
                      ? String(trustDetails.fingerprint)
                      : null,
                  )}
                </span>
              ),
            },
            {
              label: t("doctor.caCert"),
              value: (
                <span title={String(trustDetails?.cert ?? "")}>
                  {trustDetails?.cert != null ? String(trustDetails.cert) : t("common.emDash")}
                </span>
              ),
            },
          ]}
        />
        <div className="doctor-actions">
          {!caPresent ? (
            <Button
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                initCa.mutate();
              }}
            >
              {t("doctor.caInit")}
            </Button>
          ) : null}
          {caPresent && !caTrusted ? (
            <Button
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                installCa.mutate();
              }}
            >
              {t("doctor.caInstall")}
            </Button>
          ) : null}
          {caTrusted ? (
            <Muted>{t("doctor.caTrustOk")}</Muted>
          ) : null}
        </div>
      </Card>

      <div className="view-stack">
        {doctorQuery.isLoading && checks.length === 0 ? (
          <LoadingState card message={t("doctor.running")} />
        ) : (
          checks.map((check) => (
            <DoctorCheckRow
              key={check.id}
              check={check}
              busy={busy}
              onInstallCa={() => {
                setActionMsg(null);
                installCa.mutate();
              }}
              onInstallBindHelper={() => {
                setActionMsg(null);
                installBindHelper.mutate();
              }}
              onRestartEdge={() => {
                setActionMsg(null);
                restartEdge.mutate();
              }}
            />
          ))
        )}
      </div>
    </section>
  );
}

function DoctorCheckRow({
  check,
  busy,
  onInstallCa,
  onInstallBindHelper,
  onRestartEdge,
}: {
  check: DoctorCheck;
  busy: boolean;
  onInstallCa: () => void;
  onInstallBindHelper: () => void;
  onRestartEdge: () => void;
}) {
  const t = useT();
  const details = asRecord(check.details);
  const remediation = asRecord(details?.remediation);
  const edgeRemediation = asRecord(remediation?.edge);
  const showInstallCa =
    check.id === "certs.host_trust" &&
    Boolean(details?.present) &&
    !Boolean(details?.trusted);
  const showInstallBind =
    check.id === "dns.bind_helper" &&
    Boolean(details?.requires_helper) &&
    check.status !== "ok";
  const showRestartEdge =
    check.id === "supervisor.health" &&
    details?.vz_edge_ok === false &&
    edgeRemediation?.action === "restart";

  return (
    <Card className="doctor-check">
      <div className="summary-row">
        <Badge tone={badgeTone(check.status)}>{check.status}</Badge>
        <Mono className="path">{check.id}</Mono>
      </div>
      <p>{check.message}</p>
      {showInstallCa ? (
        <Button disabled={busy} onClick={onInstallCa}>
          {t("doctor.caInstall")}
        </Button>
      ) : null}
      {showInstallBind ? (
        <Button disabled={busy} onClick={onInstallBindHelper}>
          {t("doctor.bindInstall")}
        </Button>
      ) : null}
      {showRestartEdge ? (
        <Button disabled={busy} onClick={onRestartEdge}>
          {t("doctor.edgeRestart")}
        </Button>
      ) : null}
    </Card>
  );
}

async function loadDoctor(): Promise<{
  status: string;
  summary: Record<string, unknown>;
  checks: DoctorCheck[];
}> {
  const envelope = parseEnvelope(await api.get("/v1/doctor"));
  const checks = Array.isArray(envelope.checks)
    ? (envelope.checks as DoctorCheck[])
    : [];
  return {
    status: String(envelope.status ?? "ok"),
    summary: (envelope.summary as Record<string, unknown>) ?? {},
    checks,
  };
}

function badgeTone(status: string): BadgeTone {
  if (status === "ok") return "ok";
  if (status === "fail") return "danger";
  return "warn";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return null;
}

function shortFp(fp: string | null): string {
  if (!fp) return getT()("common.emDash");
  if (fp.length <= 16) return fp;
  return `${fp.slice(0, 8)}…${fp.slice(-6)}`;
}
