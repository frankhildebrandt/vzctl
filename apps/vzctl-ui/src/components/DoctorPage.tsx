import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "@/lib/api";
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
  const queryClient = useQueryClient();
  const [actionMsg, setActionMsg] = useState<string | null>(null);

  const doctorQuery = useQuery({
    queryKey: doctorKeys.all,
    queryFn: loadDoctor,
  });

  const installCa = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/certs/ca/install"));
      assertEnvelopeOk(envelope, "CA-Installation fehlgeschlagen");
      return envelope;
    },
    onSuccess: () => {
      setActionMsg("Local CA in die Login-Keychain installiert.");
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const initCa = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/certs/ca/init"));
      assertEnvelopeOk(envelope, "CA-Init fehlgeschlagen");
      return envelope;
    },
    onSuccess: () => {
      setActionMsg("Local CA initialisiert. Jetzt in Keychain installieren.");
      void queryClient.invalidateQueries({ queryKey: doctorKeys.all });
    },
    onError: (err) => {
      setActionMsg(String(err));
    },
  });

  const installBindHelper = useMutation({
    mutationFn: async () => {
      const envelope = parseEnvelope(await api.post("/v1/dns/bind-helper"));
      assertEnvelopeOk(envelope, "DNS-Bind-Helper-Installation fehlgeschlagen");
      return envelope;
    },
    onSuccess: () => {
      setActionMsg(
        "DNS-Bind-Helper installiert. Guest-:53 ist bereit (ggf. Doctor neu prüfen).",
      );
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
        <h2 className="section-title">Doctor</h2>
        <p className="muted">
          Host-Checks wie <code>vzctl doctor</code> — inkl. Local-CA-Trust und
          DNS-Bind-Helper für Guest-<code>:53</code>.
        </p>
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
            {doctorQuery.data?.status ?? (doctorQuery.isLoading ? "…" : "—")}
          </span>
          {summary ? (
            <span className="muted">
              {String(summary.ok ?? 0)} ok · {String(summary.warnings ?? 0)} warn ·{" "}
              {String(summary.failures ?? 0)} fail
            </span>
          ) : null}
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => void doctorQuery.refetch()}
          >
            Neu prüfen
          </button>
        </div>
        {doctorQuery.isError ? (
          <p className="tile-error">{String(doctorQuery.error)}</p>
        ) : null}
        {actionMsg ? <p className="muted">{actionMsg}</p> : null}
      </div>

      <div className="card">
        <div className="summary-row">
          <h3 className="group-title">DNS Bind-Helper</h3>
          <span className={bindReady ? "badge ok" : "badge warn"}>
            {bindReady ? "ready" : "fehlt"}
          </span>
        </div>
        <p className="muted">
          Guest-DNS auf Bridge-<code>.0:53</code> braucht den Root-LaunchDaemon{" "}
          <code>vz-dns-bind</code> (SCM_RIGHTS). Ohne Helper:{" "}
          <code>Permission denied</code> im DNS-Status.
        </p>
        <dl className="kv">
          <div className="kv-row">
            <dt>Socket</dt>
            <dd title={String(bindDetails?.socket ?? "")}>
              {bindDetails?.socket_connectable
                ? "erreichbar"
                : bindDetails?.socket != null
                  ? String(bindDetails.socket)
                  : "—"}
            </dd>
          </div>
          <div className="kv-row">
            <dt>Guest-Port</dt>
            <dd>
              {bindDetails?.guest_port != null
                ? String(bindDetails.guest_port)
                : "—"}
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
              Bind-Helper installieren
            </button>
          ) : null}
          {bindReady ? (
            <p className="muted">Bind-Helper ist aktiv.</p>
          ) : null}
        </div>
      </div>

      <div className="card">
        <div className="summary-row">
          <h3 className="group-title">Local CA (Keychain)</h3>
          <span className={caTrusted ? "badge ok" : caPresent ? "badge warn" : "badge ok"}>
            {caTrusted ? "trusted" : caPresent ? "nicht trusted" : "keine CA"}
          </span>
        </div>
        <p className="muted">
          Ohne Keychain-Trust melden Browser{" "}
          <code>SEC_ERROR_UNKNOWN_ISSUER</code> für{" "}
          <code>*.svc.…vz.test</code>. Firefox/Zen brauchen zusätzlich
          System-CAs (<code>security.enterprise_roots.enabled</code>) oder
          manuellen Import.
        </p>
        <dl className="kv">
          <div className="kv-row">
            <dt>Fingerprint</dt>
            <dd title={String(trustDetails?.fingerprint ?? "")}>
              {shortFp(
                trustDetails?.fingerprint != null
                  ? String(trustDetails.fingerprint)
                  : null,
              )}
            </dd>
          </div>
          <div className="kv-row">
            <dt>Cert</dt>
            <dd title={String(trustDetails?.cert ?? "")}>
              {trustDetails?.cert != null ? String(trustDetails.cert) : "—"}
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
              CA initialisieren
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
              CA in Keychain installieren
            </button>
          ) : null}
          {caTrusted ? (
            <p className="muted">Keychain-Trust ist gesetzt.</p>
          ) : null}
        </div>
      </div>

      <div className="view-stack">
        {doctorQuery.isLoading && checks.length === 0 ? (
          <div className="card">
            <p className="muted">Doctor läuft…</p>
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
          CA in Keychain installieren
        </button>
      ) : null}
      {showInstallBind ? (
        <button type="button" disabled={busy} onClick={onInstallBindHelper}>
          Bind-Helper installieren
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
  if (!fp) return "—";
  if (fp.length <= 16) return fp;
  return `${fp.slice(0, 8)}…${fp.slice(-6)}`;
}
