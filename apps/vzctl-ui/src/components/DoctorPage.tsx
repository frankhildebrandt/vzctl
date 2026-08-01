import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "@/lib/api";
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
    doctorQuery.isFetching;

  return (
    <section>
      <header className="detail-heading" style={{ marginBottom: "1rem" }}>
        <h2 className="section-title">{t("doctor.title")}</h2>
        <p className="muted">{t("doctor.subtitle")}</p>
      </header>

      <div className="card summary-card">
        <div className="summary-row">
          <span
            className={
              doctorQuery.data?.status === "fail"
                ? "badge danger"
                : doctorQuery.data?.status === "warn"
                  ? "badge warn"
                  : "badge ok"
            }
          >
            {doctorQuery.data?.status ?? (doctorQuery.isLoading ? t("common.ellipsis") : t("common.emDash"))}
          </span>
          {summary ? (
            <span className="muted">
              {t("doctor.summaryOk", {
                ok: String(summary.ok ?? 0),
                warnings: String(summary.warnings ?? 0),
                failures: String(summary.failures ?? 0),
              })}
            </span>
          ) : null}
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => void doctorQuery.refetch()}
          >
            {t("doctor.refresh")}
          </button>
        </div>
        {doctorQuery.isError ? (
          <p className="tile-error">{String(doctorQuery.error)}</p>
        ) : null}
        {actionMsg ? <p className="muted">{actionMsg}</p> : null}
      </div>

      <div className="card">
        <div className="summary-row">
          <h3 className="group-title">{t("doctor.bindTitle")}</h3>
          <span className={bindReady ? "badge ok" : "badge warn"}>
            {bindReady ? t("doctor.bindReady") : t("doctor.bindMissing")}
          </span>
        </div>
        <p className="muted">{t("doctor.bindHint")}</p>
        <dl className="kv">
          <div className="kv-row">
            <dt>{t("doctor.bindSocket")}</dt>
            <dd title={String(bindDetails?.socket ?? "")}>
              {bindDetails?.socket_connectable
                ? t("doctor.bindSocketOk")
                : bindDetails?.socket != null
                  ? String(bindDetails.socket)
                  : t("common.emDash")}
            </dd>
          </div>
          <div className="kv-row">
            <dt>{t("doctor.bindGuestPort")}</dt>
            <dd>
              {bindDetails?.guest_port != null
                ? String(bindDetails.guest_port)
                : t("common.emDash")}
            </dd>
          </div>
        </dl>
        <div className="doctor-actions">
          {bindNeedsInstall ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                installBindHelper.mutate();
              }}
            >
              {t("doctor.bindInstall")}
            </button>
          ) : null}
          {bindReady ? (
            <p className="muted">{t("doctor.bindActive")}</p>
          ) : null}
        </div>
      </div>

      <div className="card">
        <div className="summary-row">
          <h3 className="group-title">{t("doctor.caTitle")}</h3>
          <span className={caTrusted ? "badge ok" : caPresent ? "badge warn" : "badge ok"}>
            {caTrusted
              ? t("doctor.caTrusted")
              : caPresent
                ? t("doctor.caNotTrusted")
                : t("doctor.caNone")}
          </span>
        </div>
        <p className="muted">{t("doctor.caHint")}</p>
        <dl className="kv">
          <div className="kv-row">
            <dt>{t("doctor.caFingerprint")}</dt>
            <dd title={String(trustDetails?.fingerprint ?? "")}>
              {shortFp(
                trustDetails?.fingerprint != null
                  ? String(trustDetails.fingerprint)
                  : null,
              )}
            </dd>
          </div>
          <div className="kv-row">
            <dt>{t("doctor.caCert")}</dt>
            <dd title={String(trustDetails?.cert ?? "")}>
              {trustDetails?.cert != null ? String(trustDetails.cert) : t("common.emDash")}
            </dd>
          </div>
        </dl>
        <div className="doctor-actions">
          {!caPresent ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                initCa.mutate();
              }}
            >
              {t("doctor.caInit")}
            </button>
          ) : null}
          {caPresent && !caTrusted ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setActionMsg(null);
                installCa.mutate();
              }}
            >
              {t("doctor.caInstall")}
            </button>
          ) : null}
          {caTrusted ? (
            <p className="muted">{t("doctor.caTrustOk")}</p>
          ) : null}
        </div>
      </div>

      <div className="view-stack">
        {doctorQuery.isLoading && checks.length === 0 ? (
          <div className="card">
            <p className="muted">{t("doctor.running")}</p>
          </div>
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
}: {
  check: DoctorCheck;
  busy: boolean;
  onInstallCa: () => void;
  onInstallBindHelper: () => void;
}) {
  const t = useT();
  const details = asRecord(check.details);
  const showInstallCa =
    check.id === "certs.host_trust" &&
    Boolean(details?.present) &&
    !Boolean(details?.trusted);
  const showInstallBind =
    check.id === "dns.bind_helper" &&
    Boolean(details?.requires_helper) &&
    check.status !== "ok";

  return (
    <div className="card doctor-check">
      <div className="summary-row">
        <span className={`badge ${badgeTone(check.status)}`}>
          {check.status}
        </span>
        <code className="path">{check.id}</code>
      </div>
      <p>{check.message}</p>
      {showInstallCa ? (
        <button type="button" disabled={busy} onClick={onInstallCa}>
          {t("doctor.caInstall")}
        </button>
      ) : null}
      {showInstallBind ? (
        <button type="button" disabled={busy} onClick={onInstallBindHelper}>
          {t("doctor.bindInstall")}
        </button>
      ) : null}
    </div>
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

function badgeTone(status: string): string {
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
