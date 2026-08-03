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
  SectionTitle,
  SelectableCard,
} from "@/components/ui";
import { getT, useT, LOCALE_OPTIONS, type MessageKey } from "@/lib/i18n";
import {
  THEME_OPTION_IDS,
  THEME_LABEL_KEYS,
  THEME_DESCRIPTION_KEYS,
} from "@/lib/settings";
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
      <SectionTitle>{t("settings.title")}</SectionTitle>
      <Muted>{t("settings.subtitle")}</Muted>

      <Card
        className="settings-section"
        title={t("settings.appearance")}
        subtitle={<span className="settings-section-hint">{t("settings.themeHint")}</span>}
      >
        <div
          className="theme-grid"
          role="radiogroup"
          aria-label={t("settings.themeAria")}
        >
          {THEME_OPTION_IDS.map((id) => (
            <SelectableCard
              key={id}
              label={t(THEME_LABEL_KEYS[id])}
              description={t(THEME_DESCRIPTION_KEYS[id])}
              selected={theme === id}
              previewKey={id}
              preview={<ThemePreview />}
              onClick={() => setTheme(id)}
            />
          ))}
        </div>
      </Card>

      <Card
        className="settings-section"
        title={t("settings.locale")}
        subtitle={<span className="settings-section-hint">{t("settings.localeHint")}</span>}
      >
        <div
          className="theme-grid"
          role="radiogroup"
          aria-label={t("settings.localeAria")}
        >
          {LOCALE_OPTIONS.map((option) => (
            <SelectableCard
              key={option.id}
              appearance="locale"
              label={option.label}
              description={option.description}
              selected={locale === option.id}
              onClick={() => setLocale(option.id)}
            />
          ))}
        </div>
      </Card>

      <HostOidcUplinkSection />
    </section>
  );
}

function ThemePreview() {
  return (
    <span className="theme-preview" aria-hidden>
      <span className="theme-preview-sidebar" />
      <span className="theme-preview-main">
        <span className="theme-preview-bar" />
        <span className="theme-preview-card" />
      </span>
    </span>
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
    <Card
      className="settings-section"
      title={t("settings.oidcHostTitle")}
      subtitle={<span className="settings-section-hint">{t("settings.oidcHostHint")}</span>}
    >
      {loading ? (
        <LoadingState message={t("common.loading")} />
      ) : (
        <form onSubmit={onSave}>
          <FormGrid>
          <div className="form-span-2">
            <Muted className="settings-section-hint" style={{ marginBottom: "0.5rem" }}>
              {t("settings.oidcPresetHint")}
            </Muted>
            <div
              className="provider-preset-row"
              role="radiogroup"
              aria-label={t("settings.providerAria")}
            >
              {PROVIDER_PRESETS.map((p) => (
                <SelectableCard
                  key={p.id}
                  appearance="preset"
                  selected={type === p.id}
                  label={t(`oidc.preset.${p.id}` as MessageKey)}
                  onClick={() => applyPreset(p.id)}
                />
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
            <FormField label={t("settings.field.issuer")}>
              <input
                type="url"
                value={issuer}
                onChange={(e) => setIssuer(e.target.value)}
                placeholder={t("settings.placeholder.issuer")}
                autoComplete="off"
              />
            </FormField>
          ) : null}
          {preset.showTenant ? (
            <FormField label={t("settings.field.tenant")}>
              <input
                type="text"
                value={tenant}
                onChange={(e) => setTenant(e.target.value)}
                placeholder={t("settings.placeholder.tenant")}
                autoComplete="off"
              />
            </FormField>
          ) : null}
          <FormField
            label={t("settings.field.clientId")}
            span={preset.showIssuer || preset.showTenant ? undefined : 2}
          >
            <input
              type="text"
              value={clientID}
              onChange={(e) => setClientID(e.target.value)}
              placeholder={t("settings.placeholder.clientId")}
              autoComplete="off"
            />
          </FormField>
          <FormField label={t("settings.field.clientSecret")} span={2}>
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
          </FormField>
          <FormField label={t("settings.field.scopes")} span={2}>
            <input
              type="text"
              value={scopes}
              onChange={(e) => setScopes(e.target.value)}
              placeholder={preset.scopes}
              autoComplete="off"
            />
          </FormField>
          {preset.showGetUserInfo ? (
            <FormCheck>
              <input
                type="checkbox"
                checked={getUserInfo}
                onChange={(e) => setGetUserInfo(e.target.checked)}
              />
              {t("settings.field.getUserInfo")}
            </FormCheck>
          ) : null}
          <FormActions
            className="form-span-2"
            busy={saving}
            submitLabel={saving ? t("common.saving") : t("settings.saveUplink")}
          >
            {secretPresent ? (
              <Muted as="span">{t("settings.secretPresent")}</Muted>
            ) : (
              <Muted as="span">{t("settings.secretMissing")}</Muted>
            )}
          </FormActions>
          <FieldError className="form-span-2" message={error} />
          {status ? <Muted className="form-span-2">{status}</Muted> : null}
          </FormGrid>
        </form>
      )}
    </Card>
  );
}
