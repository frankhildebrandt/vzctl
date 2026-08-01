import { invoke } from "@tauri-apps/api/core";
import { parse, stringify } from "yaml";

export const UPLINK_TYPES = ["oidc", "github", "microsoft", "discord"] as const;
export type UplinkType = (typeof UPLINK_TYPES)[number];

export type OidcUplinkConfig = {
  type?: UplinkType;
  issuer?: string;
  tenant?: string;
  clientID?: string;
  clientSecretFile?: string;
  scopes?: string[];
  getUserInfo?: boolean;
};

export type HostOidcUplinkFile = {
  uplink?: OidcUplinkConfig;
};

export type HostOidcUplinkState = {
  uplink: OidcUplinkConfig | null;
  secretPresent: boolean;
  stateDir: string;
};

export type ProviderPreset = {
  id: UplinkType;
  label: string;
  issuer?: string;
  tenant?: string;
  scopes: string;
  getUserInfo: boolean;
  showIssuer: boolean;
  showTenant: boolean;
  showGetUserInfo: boolean;
  help: {
    createUrl: string;
    createLabel: string;
    redirectHint: string;
    steps: string[];
  };
};

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "oidc",
    label: "Generic OIDC",
    scopes: "openid, profile, email",
    getUserInfo: true,
    showIssuer: true,
    showTenant: false,
    showGetUserInfo: true,
    help: {
      createUrl: "",
      createLabel: "",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
      steps: [
        "Beliebiger OIDC-IdP mit Discovery (/.well-known/openid-configuration).",
        "Redirect URI in der IdP-App: https://auth.svc.<project>.vz.test/callback",
        "Client ID + Secret hier eintragen.",
      ],
    },
  },
  {
    id: "github",
    label: "GitHub",
    scopes: "read:user, user:email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: false,
    showGetUserInfo: false,
    help: {
      createUrl: "https://github.com/settings/developers",
      createLabel: "GitHub OAuth App anlegen",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
      steps: [
        "GitHub → Settings → Developer settings → OAuth Apps → New OAuth App.",
        "Homepage URL beliebig (z. B. https://vzctl.local).",
        "Authorization callback URL: https://auth.svc.<project>.vz.test/callback",
        "Client ID übernehmen; Client Secret generieren und hier speichern.",
        "Dex nutzt den nativen github-Connector.",
      ],
    },
  },
  {
    id: "discord",
    label: "Discord",
    scopes: "identify, email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: false,
    showGetUserInfo: false,
    help: {
      createUrl: "https://discord.com/developers/applications",
      createLabel: "Discord Application anlegen",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
      steps: [
        "Discord Developer Portal → New Application → OAuth2.",
        "Redirects: https://auth.svc.<project>.vz.test/callback hinzufügen.",
        "Client ID + Client Secret (Reset Secret) hier eintragen.",
        "Scopes identify + email (E-Mail muss in Discord verifiziert sein).",
        "Dex mappt Discord via oauth2-Connector.",
      ],
    },
  },
  {
    id: "microsoft",
    label: "Microsoft Entra ID",
    tenant: "common",
    scopes: "openid, profile, email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: true,
    showGetUserInfo: false,
    help: {
      createUrl:
        "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
      createLabel: "Entra App-Registrierung",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
      steps: [
        "Azure Portal → Microsoft Entra ID → App registrations → New registration.",
        "Supported account types passend wählen (Single tenant / Multitenant).",
        "Redirect URI (Web): https://auth.svc.<project>.vz.test/callback",
        "Certificates & secrets → New client secret.",
        "Application (client) ID + Secret hier; Tenant = Directory (tenant) ID oder common / organizations.",
        "Dex nutzt den nativen microsoft-Connector.",
      ],
    },
  },
];

async function isTauri(): Promise<boolean> {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function getVzctlStateDir(): Promise<string> {
  if (await isTauri()) {
    return invoke<string>("vzctl_state_dir");
  }
  return "/Users/demo/Library/Application Support/vzctl";
}

export function hostUplinkPath(stateDir: string): string {
  return `${stateDir}/config/oidc-uplink.yaml`;
}

export function hostSecretPath(stateDir: string): string {
  return `${stateDir}/config/oidc/client-secret`;
}

export function projectUplinkSecretPath(stateDir: string, project: string): string {
  return `${stateDir}/projects/${project}/oidc/uplink-secret`;
}

export function isUplinkType(value: unknown): value is UplinkType {
  return typeof value === "string" && (UPLINK_TYPES as readonly string[]).includes(value);
}

export function presetFor(type: UplinkType): ProviderPreset {
  return PROVIDER_PRESETS.find((p) => p.id === type) ?? PROVIDER_PRESETS[0];
}

export function normalizeOidcUplink(value: unknown): OidcUplinkConfig | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const type: UplinkType = isUplinkType(record.type) ? record.type : "oidc";
  const scopes = Array.isArray(record.scopes)
    ? record.scopes.filter((s): s is string => typeof s === "string" && s.length > 0)
    : undefined;
  return {
    type,
    issuer: typeof record.issuer === "string" ? record.issuer : undefined,
    tenant: typeof record.tenant === "string" ? record.tenant : undefined,
    clientID: typeof record.clientID === "string" ? record.clientID : undefined,
    clientSecretFile:
      typeof record.clientSecretFile === "string" ? record.clientSecretFile : undefined,
    scopes: scopes && scopes.length > 0 ? scopes : undefined,
    getUserInfo:
      typeof record.getUserInfo === "boolean" ? record.getUserInfo : undefined,
  };
}

export function validateUplinkDraft(draft: {
  type: UplinkType;
  issuer: string;
  tenant: string;
  clientID: string;
}): string | null {
  if (!draft.clientID.trim()) return "Client ID ist erforderlich.";
  if (draft.type === "oidc") {
    if (!draft.issuer.trim()) return "Issuer ist erforderlich.";
    if (!draft.issuer.trim().startsWith("https://")) {
      return "Issuer muss mit https:// beginnen.";
    }
  }
  if (draft.type === "microsoft" && !draft.tenant.trim()) {
    return "Tenant ist erforderlich (z. B. common oder Directory-ID).";
  }
  return null;
}

export async function loadHostOidcUplink(): Promise<HostOidcUplinkState> {
  const { isDemoMode } = await import("@/lib/demo");
  if (isDemoMode()) {
    return { uplink: null, secretPresent: false, stateDir: await getVzctlStateDir() };
  }
  const stateDir = await getVzctlStateDir();
  let uplink: OidcUplinkConfig | null = null;
  let secretPresent = false;
  try {
    const { api } = await import("@/lib/api");
    const yaml = await api.getText("/v1/oidc/uplink");
    if (yaml.trim()) {
      const parsed = parse(yaml) as unknown;
      if (parsed && typeof parsed === "object") {
        uplink = normalizeOidcUplink((parsed as HostOidcUplinkFile).uplink);
      }
    }
    // Secret presence: probe via path when Tauri available.
    if (await isTauri()) {
      secretPresent = await invoke<boolean>("path_exists", {
        path: hostSecretPath(stateDir),
      });
    }
  } catch {
    // fall through
  }
  return { uplink, secretPresent, stateDir };
}

export async function saveHostOidcUplink(input: {
  type: UplinkType;
  issuer: string;
  tenant: string;
  clientID: string;
  scopes: string;
  getUserInfo: boolean;
  clientSecret: string;
}): Promise<void> {
  const error = validateUplinkDraft(input);
  if (error) throw new Error(error);
  if (!(await isTauri())) {
    throw new Error("OIDC-Uplink speichern nur in der Tauri-App");
  }

  const stateDir = await getVzctlStateDir();
  const scopes = input.scopes
    .split(/[,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const preset = presetFor(input.type);
  const uplink: OidcUplinkConfig = {
    type: input.type,
    clientID: input.clientID.trim(),
    clientSecretFile: "client-secret",
    scopes: scopes.length > 0 ? scopes : preset.scopes.split(/,\s*/),
  };
  if (preset.showIssuer && input.issuer.trim()) {
    uplink.issuer = input.issuer.trim();
  }
  if (preset.showTenant && input.tenant.trim()) {
    uplink.tenant = input.tenant.trim();
  }
  if (preset.showGetUserInfo) {
    uplink.getUserInfo = input.getUserInfo;
  }

  const file: HostOidcUplinkFile = { uplink };
  const yaml = stringify(file, { lineWidth: 100 });
  const { api } = await import("@/lib/api");
  await api.putText("/v1/oidc/uplink", yaml, "text/yaml; charset=utf-8");
  if (input.clientSecret.trim()) {
    await invoke("write_secret_file", {
      path: hostSecretPath(stateDir),
      contents: `${input.clientSecret.trim()}\n`,
    });
  } else {
    const present = await invoke<boolean>("path_exists", {
      path: hostSecretPath(stateDir),
    });
    if (!present) {
      throw new Error("Client Secret ist erforderlich (noch nicht gesetzt).");
    }
  }
}

export async function saveProjectUplinkSecret(
  project: string,
  secret: string,
): Promise<void> {
  if (!(await isTauri())) {
    throw new Error("Secret speichern nur in der Tauri-App");
  }
  if (!secret.trim()) throw new Error("Project Secret darf nicht leer sein.");
  const { apiRequest, encodeId } = await import("@/lib/api");
  await apiRequest(`/v1/projects/${encodeId(project)}/oidc/secret`, {
    method: "PUT",
    rawBody: `${secret.trim()}\n`,
    contentType: "text/plain; charset=utf-8",
  });
}

export function scopesToInput(scopes: string[] | undefined, type: UplinkType = "oidc"): string {
  if (scopes && scopes.length > 0) return scopes.join(", ");
  return presetFor(type).scopes;
}
