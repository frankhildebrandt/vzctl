import { useEffect, useState, type FormEvent } from "react";
import {
  Card,
  FieldError,
  FormActions,
  FormCheck,
  FormField,
  FormGrid,
  LoadingState,
  Muted,
} from "@/components/ui";
import { getT, useT } from "@/lib/i18n";
import {
  OidcMessageError,
  saveProjectUplinkSecret,
  scopesToInput,
  type OidcUplinkConfig,
} from "@/lib/oidcUplink";
import { loadProjectFlexible, saveProjectFlexible } from "@/features/persistence/projectIo";
import type { Environment } from "@/domain/hypernetwork/schema";
import { Link } from "@tanstack/react-router";

const CONFIG_FILE = "hypernetwork.config.yaml";

function ProjectOidcHint() {
  const t = useT();
  return (
    <>
      {t("projectOidc.hintBefore")}
      <code>{CONFIG_FILE}</code>
      {t("projectOidc.hintAfter")}
      <Link to="/settings">{t("projectOidc.hintLink")}</Link>
      {t("projectOidc.hintEnd")}
    </>
  );
}

function formatError(error: unknown, t: ReturnType<typeof useT>): string {
  if (error instanceof OidcMessageError) return t(error.messageKey);
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Per-stack OIDC uplink overrides → hypernetwork.config.yaml */
export function ProjectOidcUplinkSection({ projectPath }: { projectPath: string }) {
  const t = useT();
  const [issuer, setIssuer] = useState("");
  const [clientID, setClientID] = useState("");
  const [scopes, setScopes] = useState("");
  const [getUserInfo, setGetUserInfo] = useState(true);
  const [useHostSecret, setUseHostSecret] = useState(true);
  const [projectSecret, setProjectSecret] = useState("");
  const [clearOverride, setClearOverride] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [env, setEnv] = useState<Environment | null>(null);

  useEffect(() => {
    if (!projectPath) {
      setEnv(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    setStatus(null);
    void (async () => {
      try {
        const loaded = await loadProjectFlexible(projectPath);
        if (cancelled) return;
        setEnv(loaded.env);
        const uplink = loaded.env.spec.oidc?.uplink;
        setIssuer(uplink?.issuer ?? "");
        setClientID(uplink?.clientID ?? "");
        setScopes(uplink?.scopes ? scopesToInput(uplink.scopes) : "");
        setGetUserInfo(uplink?.getUserInfo ?? true);
        setUseHostSecret(
          !uplink?.clientSecretFile || uplink.clientSecretFile === "host",
        );
        setClearOverride(false);
        setProjectSecret("");
      } catch (e) {
        if (!cancelled) {
          setEnv(null);
          setError(formatError(e, getT()));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectPath]);

  async function onSave(event: FormEvent) {
    event.preventDefault();
    if (!env) return;
    setError(null);
    setStatus(null);
    setSaving(true);
    try {
      const nextEnv: Environment = structuredClone(env);
      if (!nextEnv.spec.oidc) {
        throw new OidcMessageError("projectOidc.noOidcSpec");
      }

      if (clearOverride) {
        delete nextEnv.spec.oidc.uplink;
      } else {
        const uplink: OidcUplinkConfig = {};
        if (env.spec.oidc?.uplink?.type) {
          uplink.type = env.spec.oidc.uplink.type;
        }
        if (issuer.trim()) uplink.issuer = issuer.trim();
        if (clientID.trim()) uplink.clientID = clientID.trim();
        const scopeList = scopes
          .split(/[,\s]+/)
          .map((s) => s.trim())
          .filter(Boolean);
        if (scopeList.length > 0) uplink.scopes = scopeList;
        uplink.getUserInfo = getUserInfo;
        if (useHostSecret) {
          uplink.clientSecretFile = "host";
        } else {
          uplink.clientSecretFile = "uplink-secret";
          if (projectSecret.trim()) {
            await saveProjectUplinkSecret(nextEnv.spec.project, projectSecret);
          }
        }
        if (uplink.issuer && !uplink.issuer.startsWith("https://")) {
          throw new OidcMessageError("oidc.error.issuerHttps");
        }
        nextEnv.spec.oidc.uplink = uplink;
      }

      const diagram = (await loadProjectFlexible(projectPath)).diagram;
      await saveProjectFlexible(projectPath, nextEnv, diagram);
      setEnv(nextEnv);
      setProjectSecret("");
      setStatus(
        t(clearOverride ? "projectOidc.cleared" : "projectOidc.saved"),
      );
    } catch (e) {
      setError(formatError(e, t));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card
      className="settings-section"
      title={t("projectOidc.title")}
      subtitle={
        <span className="settings-section-hint">
          <ProjectOidcHint />
        </span>
      }
    >
      {loading ? (
        <LoadingState message={t("common.loading")} />
      ) : (
        <form onSubmit={onSave}>
          <FormGrid>
          <FormCheck className="form-span-2">
            <input
              type="checkbox"
              checked={clearOverride}
              onChange={(e) => setClearOverride(e.target.checked)}
            />
            {t("projectOidc.clearOverride")}
          </FormCheck>
          {!clearOverride ? (
            <>
              <FormField label={t("projectOidc.issuerOptional")}>
                <input
                  type="url"
                  value={issuer}
                  onChange={(e) => setIssuer(e.target.value)}
                  placeholder={t("projectOidc.issuerPlaceholder")}
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </FormField>
              <FormField label={t("projectOidc.clientIdOptional")}>
                <input
                  type="text"
                  value={clientID}
                  onChange={(e) => setClientID(e.target.value)}
                  placeholder={t("projectOidc.clientIdPlaceholder")}
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </FormField>
              <FormField label={t("projectOidc.scopesOptional")} span={2}>
                <input
                  type="text"
                  value={scopes}
                  onChange={(e) => setScopes(e.target.value)}
                  placeholder={t("projectOidc.scopesPlaceholder")}
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </FormField>
              <FormCheck>
                <input
                  type="checkbox"
                  checked={getUserInfo}
                  onChange={(e) => setGetUserInfo(e.target.checked)}
                  disabled={!env?.spec.oidc}
                />
                {t("settings.field.getUserInfo")}
              </FormCheck>
              <FormCheck>
                <input
                  type="checkbox"
                  checked={useHostSecret}
                  onChange={(e) => setUseHostSecret(e.target.checked)}
                  disabled={!env?.spec.oidc}
                />
                {t("projectOidc.useHostSecret")}
              </FormCheck>
              {!useHostSecret ? (
                <FormField label={t("projectOidc.stackSecret")} span={2}>
                  <input
                    type="password"
                    value={projectSecret}
                    onChange={(e) => setProjectSecret(e.target.value)}
                    placeholder={t("projectOidc.stackSecretPlaceholder")}
                    autoComplete="new-password"
                    disabled={!env?.spec.oidc}
                  />
                </FormField>
              ) : null}
            </>
          ) : null}
          <FormActions
            className="form-span-2"
            busy={saving}
            submitLabel={saving ? t("common.saving") : t("projectOidc.save")}
            submitDisabled={!env?.spec.oidc}
          >
            {!env?.spec.oidc ? (
              <Muted as="span">{t("projectOidc.noSpecOidc")}</Muted>
            ) : null}
          </FormActions>
          <FieldError className="form-span-2" message={error} />
          {status ? <Muted className="form-span-2">{status}</Muted> : null}
          </FormGrid>
        </form>
      )}
    </Card>
  );
}
