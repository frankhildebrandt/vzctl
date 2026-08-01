import { useEffect, useState, type FormEvent } from "react";
import { getT, useT, LOCALE_OPTIONS, type MessageKey } from "@/lib/i18n";
import {
  THEME_OPTION_IDS,
  THEME_LABEL_KEYS,
  THEME_DESCRIPTION_KEYS,
  type ThemeId,
} from "@/lib/settings";
import type { LocaleId } from "@/lib/i18n";
import { useSettingsStore } from "@/store/settingsStore";
import {
  loadHostOidcUplink,
  OidcMessageError,
  presetFor,
  PROVIDER_PRESETS,
  providerCreateLabelKey,
  providerHelpSteps,
  saveHostOidcUplink,
  scopesToInput,
  validateUplinkDraft,
  type UplinkType,
} from "@/lib/oidcUplink";

function formatError(error: unknown, t: ReturnType<typeof useT>): string {
  if (error instanceof OidcMessageError) return t(error.messageKey);
  if (error instanceof Error) return error.message;
  return String(error);
}

export function SettingsPage() {
  const t = useT();
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);
  const locale = useSettingsStore((s) => s.locale);
  const setLocale = useSettingsStore((s) => s.setLocale);

  return (
    <section>
      <h2 className="section-title">{t("settings.title")}</h2>
      <p className="muted">{t("settings.subtitle")}</p>

      <div className="card settings-section">
        <h2>{t("settings.appearance")}</h2>
        <p className="muted settings-section-hint">{t("settings.themeHint")}</p>
        <div
          className="theme-grid"
          role="radiogroup"
          aria-label={t("settings.themeAria")}
        >
          {THEME_OPTION_IDS.map((id) => (
            <ThemeCard
              key={id}
              id={id}
              label={t(THEME_LABEL_KEYS[id])}
              description={t(THEME_DESCRIPTION_KEYS[id])}
              selected={theme === id}
              onSelect={setTheme}
            />
          ))}
        </div>
      </div>

      <div className="card settings-section">
        <h2>{t("settings.locale")}</h2>
        <p className="muted settings-section-hint">{t("settings.localeHint")}</p>
        <div
          className="theme-grid"
          role="radiogroup"
          aria-label={t("settings.localeAria")}
        >
          {LOCALE_OPTIONS.map((option) => (
            <LocaleCard
              key={option.id}
              id={option.id}
              label={option.label}
              description={option.description}
              selected={locale === option.id}
              onSelect={setLocale}
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

function LocaleCard({
  id,
  label,
  description,
  selected,
  onSelect,
}: {
  id: LocaleId;
  label: string;
  description: string;
  selected: boolean;
  onSelect: (locale: LocaleId) => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={`theme-card locale-card${selected ? " selected" : ""}`}
      onClick={() => onSelect(id)}
    >
      <span className="theme-card-meta">
        <span className="theme-card-label">{label}</span>
        <span className="theme-card-desc muted">{description}</span>
      </span>
    </button>
  );
}

function HostOidcUplinkSection() {
  const t = useT();
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
  const createLabelKey = providerCreateLabelKey(type);
  const helpSteps = providerHelpSteps(type);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const state = await loadHostOidcUplink();
        if (cancelled) return;
        if (state.uplink) {
          const uplinkType = state.uplink.type ?? "oidc";
          setType(uplinkType);
          setIssuer(state.uplink.issuer ?? "");
          setTenant(state.uplink.tenant ?? "common");
          setClientID(state.uplink.clientID ?? "");
          setScopes(scopesToInput(state.uplink.scopes, uplinkType));
          setGetUserInfo(state.uplink.getUserInfo ?? true);
        }
        setSecretPresent(state.secretPresent);
      } catch (e) {
        if (!cancelled) {
          setError(formatError(e, getT()));
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
      setError(t(validation));
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
      setStatus(t("settings.uplinkSaved"));
    } catch (e) {
      setError(formatError(e, t));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="card settings-section">
      <h2>{t("settings.oidcHostTitle")}</h2>
      <p className="muted settings-section-hint">{t("settings.oidcHostHint")}</p>
      {loading ? (
        <p className="muted">{t("common.loading")}</p>
      ) : (
        <form className="form-grid" onSubmit={onSave}>
          <div className="form-span-2">
            <p
              className="muted settings-section-hint"
              style={{ marginBottom: "0.5rem" }}
            >
              {t("settings.oidcPresetHint")}
            </p>
            <div
              className="provider-preset-row"
              role="radiogroup"
              aria-label={t("settings.providerAria")}
            >
              {PROVIDER_PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  role="radio"
                  aria-checked={type === p.id}
                  className={`provider-preset${type === p.id ? " selected" : ""}`}
                  onClick={() => applyPreset(p.id)}
                >
                  {t(`oidc.preset.${p.id}` as MessageKey)}
                </button>
              ))}
            </div>
          </div>

          <div className="form-span-2 uplink-help">
            <p className="uplink-help-redirect">
              <strong>{t("settings.redirectUri")}</strong>{" "}
              <code>{preset.help.redirectHint}</code>
            </p>
            <ol className="uplink-help-steps">
              {helpSteps.map((key) => (
                <li key={key}>{t(key)}</li>
              ))}
            </ol>
            {preset.help.createUrl && createLabelKey ? (
              <p className="muted">
                <a href={preset.help.createUrl} target="_blank" rel="noreferrer">
                  {t(createLabelKey)} ↗
                </a>
              </p>
            ) : null}
          </div>

          {preset.showIssuer ? (
            <label>
              {t("settings.field.issuer")}
              <input
                type="url"
                value={issuer}
                onChange={(e) => setIssuer(e.target.value)}
                placeholder={t("settings.placeholder.issuer")}
                autoComplete="off"
              />
            </label>
          ) : null}
          {preset.showTenant ? (
            <label>
              {t("settings.field.tenant")}
              <input
                type="text"
                value={tenant}
                onChange={(e) => setTenant(e.target.value)}
                placeholder={t("settings.placeholder.tenant")}
                autoComplete="off"
              />
            </label>
          ) : null}
          <label
            className={preset.showIssuer || preset.showTenant ? "" : "form-span-2"}
          >
            {t("settings.field.clientId")}
            <input
              type="text"
              value={clientID}
              onChange={(e) => setClientID(e.target.value)}
              placeholder={t("settings.placeholder.clientId")}
              autoComplete="off"
            />
          </label>
          <label className="form-span-2">
            {t("settings.field.clientSecret")}
            <input
              type="password"
              value={clientSecret}
              onChange={(e) => setClientSecret(e.target.value)}
              placeholder={
                secretPresent
                  ? t("settings.placeholder.secretSet")
                  : t("settings.placeholder.secret")
              }
              autoComplete="new-password"
            />
          </label>
          <label className="form-span-2">
            {t("settings.field.scopes")}
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
              {t("settings.field.getUserInfo")}
            </label>
          ) : null}
          <div className="form-span-2 settings-form-actions">
            <button type="submit" disabled={saving}>
              {saving ? t("common.saving") : t("settings.saveUplink")}
            </button>
            {secretPresent ? (
              <span className="muted">{t("settings.secretPresent")}</span>
            ) : (
              <span className="muted">{t("settings.secretMissing")}</span>
            )}
          </div>
          {error ? <p className="form-error form-span-2">{error}</p> : null}
          {status ? <p className="muted form-span-2">{status}</p> : null}
        </form>
      )}
    </div>
  );
}
