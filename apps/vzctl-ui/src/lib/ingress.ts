export type IngressRouteLink = {
  host: string;
  url: string;
  to?: string;
  requires?: string[];
  alias?: { host: string; url: string } | null;
};

export type IngressInfo = {
  enabled: boolean;
  https_port?: number;
  host_aliases?: boolean;
  routes: IngressRouteLink[];
};

export function parseIngressInfo(
  statusRaw: string | null | undefined,
): IngressInfo | null {
  if (!statusRaw) return null;
  try {
    const parsed = JSON.parse(statusRaw) as {
      sections?: { ingress?: { data?: IngressInfo } };
    };
    const data = parsed.sections?.ingress?.data;
    if (!data || typeof data !== "object") return null;
    const routes = Array.isArray(data.routes) ? data.routes : [];
    return {
      enabled: Boolean(data.enabled),
      https_port: data.https_port,
      host_aliases: data.host_aliases,
      routes: routes
        .filter((route) => route && typeof route.host === "string" && route.host)
        .map((route) => ({
          host: String(route.host),
          url: String(route.url ?? `https://${route.host}`),
          to: route.to != null ? String(route.to) : undefined,
          requires: Array.isArray(route.requires)
            ? route.requires.map(String)
            : [],
          alias:
            route.alias && typeof route.alias === "object"
              ? {
                  host: String(route.alias.host ?? ""),
                  url: String(route.alias.url ?? ""),
                }
              : null,
        })),
    };
  } catch {
    return null;
  }
}

export async function openExternalUrl(url: string): Promise<void> {
  const { isDemoMode } = await import("@/lib/demo");
  if (isDemoMode() || !("__TAURI_INTERNALS__" in window)) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_url", { url });
}
