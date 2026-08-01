import { openExternalUrl, type IngressInfo } from "@/lib/ingress";
import { useT } from "@/lib/i18n";

export function IngressLinksCard({
  ingress,
  loading,
}: {
  ingress: IngressInfo | null;
  loading?: boolean;
}) {
  const t = useT();

  if (loading && !ingress) {
    return (
      <div className="card ingress-card">
        <p className="stack-status-kicker">{t("ingress.kicker")}</p>
        <p className="muted">{t("ingress.loading")}</p>
      </div>
    );
  }

  if (!ingress?.enabled || ingress.routes.length === 0) {
    return null;
  }

  return (
    <div className="card ingress-card">
      <div className="ingress-head">
        <div>
          <p className="stack-status-kicker">{t("ingress.kicker")}</p>
          <h2 className="ingress-title">{t("ingress.title")}</h2>
        </div>
        <span className="muted">
          {t("ingress.urlCount", { n: ingress.routes.length })}
        </span>
      </div>

      <ul className="ingress-list">
        {ingress.routes.map((route) => (
          <li key={route.host} className="ingress-item">
            <button
              type="button"
              className="ingress-link"
              onClick={() => void openExternalUrl(route.url)}
              title={route.url}
            >
              <span className="ingress-host">{route.host}</span>
              <span className="ingress-url">{route.url}</span>
            </button>
            <div className="ingress-meta">
              {route.to ? <span className="ingress-chip">→ {route.to}</span> : null}
              {(route.requires ?? []).map((req) => (
                <span key={req} className="ingress-chip req">
                  {req}
                </span>
              ))}
              {route.alias?.url ? (
                <button
                  type="button"
                  className="ingress-alias"
                  onClick={() => void openExternalUrl(route.alias!.url)}
                  title={route.alias.url}
                >
                  {route.alias.host}
                </button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
