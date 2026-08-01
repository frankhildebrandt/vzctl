import { useEffect, useState, type FormEvent } from "react";
import { THEME_OPTIONS, type ThemeId } from "@/lib/settings";
import { useSettingsStore } from "@/store/settingsStore";
import {
  loadHostOidcUplink,
  presetFor,
  PROVIDER_PRESETS,
  saveHostOidcUplink,
  scopesToInput,
  validateUplinkDraft,
  type UplinkType,
} from "@/lib/oidcUplink";

export function SettingsPage() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

  return (
    <section>
      <h2 className="section-title">Settings</h2>
      <p className="muted">Globale Einstellungen für die vzctl UI.</p>

      <div className="card settings-section">
        <h2>Appearance</h2>
        <p className="muted settings-section-hint">
          Theme gilt für die gesamte App und bleibt lokal gespeichert.
        </p>
        <div
          className="theme-grid"
          role="radiogroup"
          aria-label="Theme"
        >
          {THEME_OPTIONS.map((option) => (
            <ThemeCard
              key={option.id}
              id={option.id}
              label={option.label}
              description={option.description}
              selected={theme === option.id}
              onSelect={setTheme}
            />
          ))}
        </div>
      </div>

      <HostOidcUplinkSection />
    </section>
  );
}

function ThemeCard({
  id,
  label,
  description,
  selected,
  onSelect,
}: {
  id: ThemeId;
  label: string;
  description: string;
  selected: boolean;
  onSelect: (theme: ThemeId) => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={`theme-card${selected ? " selected" : ""}`}
      data-preview={id}
      onClick={() => onSelect(id)}
    >
      <span className="theme-preview" aria-hidden>
        <span className="theme-preview-sidebar" />
        <span className="theme-preview-main">
          <span className="theme-preview-bar" />
          <span className="theme-preview-card" />
        </span>
      </span>
      <span className="theme-card-meta">
        <span className="theme-card-label">{label}</span>
        <span className="theme-card-desc muted">{description}</span>
      </span>
    </button>
  );
}

function HostOidcUplinkSection() {
  const [type, setType] = useState<UplinkType>("oidc");
  const [issuer, setIssuer] = useState("");
  const [tenant, setTenant] = useState("common");
  const [clientID, setClientID] = useState("");
  const [scopes, setScopes] = useState("openid, profile, email");
  const [getUserInfo, setGetUserInfo] = useState(true);
  const [clientSecret, setClientSecret] = useState("");
  const [secretPresent, setSecretPresent] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const preset = presetFor(type);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const state = await loadHostOidcUplink();
        if (cancelled) return;
        if (state.uplink) {
          const t = state.uplink.type ?? "oidc";
          setType(t);
          setIssuer(state.uplink.issuer ?? "");
          setTenant(state.uplink.tenant ?? "common");
          setClientID(state.uplink.clientID ?? "");
          setScopes(scopesToInput(state.uplink.scopes, t));
          setGetUserInfo(state.uplink.getUserInfo ?? true);
        }
        setSecretPresent(state.secretPresent);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  function applyPreset(next: UplinkType) {
    const p = presetFor(next);
    setType(next);
    setScopes(p.scopes);
    setGetUserInfo(p.getUserInfo);
    if (p.issuer !== undefined) setIssuer(p.issuer);
    else if (!p.showIssuer) setIssuer("");
    if (p.tenant !== undefined) setTenant(p.tenant);
    else if (!p.showTenant) setTenant("");
    setStatus(null);
    setError(null);
  }

  async function onSave(event: FormEvent) {
    event.preventDefault();
    setError(null);
    setStatus(null);
    const validation = validateUplinkDraft({ type, issuer, tenant, clientID });
    if (validation) {
      setError(validation);
      return;
    }
    setSaving(true);
    try {
      await saveHostOidcUplink({
        type,
        issuer,
        tenant,
        clientID,
        scopes,
        getUserInfo,
        clientSecret,
      });
      setClientSecret("");
      setSecretPresent(true);
      setStatus("Host-Uplink gespeichert.");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="card settings-section">
      <h2>OIDC Uplink (Host)</h2>
      <p className="muted settings-section-hint">
        Dex federiert zu diesem Upstream-IdP. Defaults gelten für alle Stacks;
        Overrides liegen unter Stack Config (Zahnrad). Secrets unter Application
        Support, nicht in Git.
      </p>
      {loading ? (
        <p className="muted">Lade…</p>
      ) : (
        <form className="form-grid" onSubmit={onSave}>
          <div className="form-span-2">
            <p className="muted settings-section-hint" style={{ marginBottom: "0.5rem" }}>
              Konfig-Hilfe — Preset wählt Connector und Default-Scopes:
            </p>
            <div className="provider-preset-row" role="radiogroup" aria-label="Provider">
              {PROVIDER_PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  role="radio"
                  aria-checked={type === p.id}
                  className={`provider-preset${type === p.id ? " selected" : ""}`}
                  onClick={() => applyPreset(p.id)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          </div>

          <div className="form-span-2 uplink-help">
            <p className="uplink-help-redirect">
              <strong>Callback / Redirect URI:</strong>{" "}
              <code>{preset.help.redirectHint}</code>
            </p>
            <ol className="uplink-help-steps">
              {preset.help.steps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
            {preset.help.createUrl ? (
              <p className="muted">
                <a href={preset.help.createUrl} target="_blank" rel="noreferrer">
                  {preset.help.createLabel} ↗
                </a>
              </p>
            ) : null}
          </div>

          {preset.showIssuer ? (
            <label>
              Issuer
              <input
                type="url"
                value={issuer}
                onChange={(e) => setIssuer(e.target.value)}
                placeholder="https://login.example.com"
                autoComplete="off"
              />
            </label>
          ) : null}
          {preset.showTenant ? (
            <label>
              Tenant
              <input
                type="text"
                value={tenant}
                onChange={(e) => setTenant(e.target.value)}
                placeholder="common oder Directory (tenant) ID"
                autoComplete="off"
              />
            </label>
          ) : null}
          <label className={preset.showIssuer || preset.showTenant ? "" : "form-span-2"}>
            Client ID
            <input
              type="text"
              value={clientID}
              onChange={(e) => setClientID(e.target.value)}
              placeholder="Application / Client ID"
              autoComplete="off"
            />
          </label>
          <label className="form-span-2">
            Client Secret
            <input
              type="password"
              value={clientSecret}
              onChange={(e) => setClientSecret(e.target.value)}
              placeholder={
                secretPresent
                  ? "Gesetzt — leer lassen zum Behalten"
                  : "Upstream Client Secret"
              }
              autoComplete="new-password"
            />
          </label>
          <label className="form-span-2">
            Scopes
            <input
              type="text"
              value={scopes}
              onChange={(e) => setScopes(e.target.value)}
              placeholder={preset.scopes}
              autoComplete="off"
            />
          </label>
          {preset.showGetUserInfo ? (
            <label className="form-check">
              <input
                type="checkbox"
                checked={getUserInfo}
                onChange={(e) => setGetUserInfo(e.target.checked)}
              />
              getUserInfo
            </label>
          ) : null}
          <div className="form-span-2 settings-form-actions">
            <button type="submit" disabled={saving}>
              {saving ? "Speichern…" : "Host-Uplink speichern"}
            </button>
            {secretPresent ? (
              <span className="muted">Secret vorhanden</span>
            ) : (
              <span className="muted">Kein Secret gesetzt</span>
            )}
          </div>
          {error ? <p className="form-error form-span-2">{error}</p> : null}
          {status ? <p className="muted form-span-2">{status}</p> : null}
        </form>
      )}
    </div>
  );
}
