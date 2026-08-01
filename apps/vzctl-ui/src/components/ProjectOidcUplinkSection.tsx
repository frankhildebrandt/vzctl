import { useEffect, useState, type FormEvent } from "react";
import {
  saveProjectUplinkSecret,
  scopesToInput,
  type OidcUplinkConfig,
} from "@/lib/oidcUplink";
import { loadProjectFlexible, saveProjectFlexible } from "@/features/persistence/projectIo";
import type { Environment } from "@/domain/hypernetwork/schema";
import { Link } from "@tanstack/react-router";

/** Per-stack OIDC uplink overrides → hypernetwork.config.yaml */
export function ProjectOidcUplinkSection({ projectPath }: { projectPath: string }) {
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
          setError(e instanceof Error ? e.message : String(e));
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
        throw new Error(
          "Stack hat kein spec.oidc — zuerst OIDC in hypernetwork.config.yaml aktivieren.",
        );
      }

      if (clearOverride) {
        delete nextEnv.spec.oidc.uplink;
      } else {
        const uplink: OidcUplinkConfig = {};
        // Keep existing type if set; otherwise omit so Host-Type geerbt wird.
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
          throw new Error("Issuer muss mit https:// beginnen.");
        }
        nextEnv.spec.oidc.uplink = uplink;
      }

      const diagram = (await loadProjectFlexible(projectPath)).diagram;
      await saveProjectFlexible(projectPath, nextEnv, diagram);
      setEnv(nextEnv);
      setProjectSecret("");
      setStatus(
        clearOverride
          ? "Stack-Override entfernt."
          : "Uplink in hypernetwork.config.yaml gespeichert.",
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="card settings-section">
      <h2>OIDC Uplink</h2>
      <p className="muted settings-section-hint">
        Optionale Overrides für diesen Stack in{" "}
        <code>hypernetwork.config.yaml</code>. Leere Felder erben{" "}
        <Link to="/settings">Host-Defaults</Link>.
      </p>
      {loading ? (
        <p className="muted">Lade…</p>
      ) : (
        <form className="form-grid" onSubmit={onSave}>
          <label className="form-check form-span-2">
            <input
              type="checkbox"
              checked={clearOverride}
              onChange={(e) => setClearOverride(e.target.checked)}
            />
            Override entfernen (nur Host-Defaults)
          </label>
          {!clearOverride ? (
            <>
              <label>
                Issuer (optional)
                <input
                  type="url"
                  value={issuer}
                  onChange={(e) => setIssuer(e.target.value)}
                  placeholder="Host-Default übernehmen"
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </label>
              <label>
                Client ID (optional)
                <input
                  type="text"
                  value={clientID}
                  onChange={(e) => setClientID(e.target.value)}
                  placeholder="Host-Default übernehmen"
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </label>
              <label className="form-span-2">
                Scopes (optional)
                <input
                  type="text"
                  value={scopes}
                  onChange={(e) => setScopes(e.target.value)}
                  placeholder="Host-Default übernehmen"
                  autoComplete="off"
                  disabled={!env?.spec.oidc}
                />
              </label>
              <label className="form-check">
                <input
                  type="checkbox"
                  checked={getUserInfo}
                  onChange={(e) => setGetUserInfo(e.target.checked)}
                  disabled={!env?.spec.oidc}
                />
                getUserInfo
              </label>
              <label className="form-check">
                <input
                  type="checkbox"
                  checked={useHostSecret}
                  onChange={(e) => setUseHostSecret(e.target.checked)}
                  disabled={!env?.spec.oidc}
                />
                Host-Secret nutzen
              </label>
              {!useHostSecret ? (
                <label className="form-span-2">
                  Stack Secret
                  <input
                    type="password"
                    value={projectSecret}
                    onChange={(e) => setProjectSecret(e.target.value)}
                    placeholder="Secret für diesen Stack"
                    autoComplete="new-password"
                    disabled={!env?.spec.oidc}
                  />
                </label>
              ) : null}
            </>
          ) : null}
          <div className="form-span-2 settings-form-actions">
            <button type="submit" disabled={saving || !env?.spec.oidc}>
              {saving ? "Speichern…" : "Uplink speichern"}
            </button>
            {!env?.spec.oidc ? (
              <span className="muted">Kein spec.oidc in diesem Stack</span>
            ) : null}
          </div>
          {error ? <p className="form-error form-span-2">{error}</p> : null}
          {status ? <p className="muted form-span-2">{status}</p> : null}
        </form>
      )}
    </div>
  );
}
