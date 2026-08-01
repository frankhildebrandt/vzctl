import { invoke } from "@tauri-apps/api/core";
import { parse, stringify } from "yaml";
import type { MessageKey } from "@/lib/i18n";

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
  issuer?: string;
  tenant?: string;
  scopes: string;
  getUserInfo: boolean;
  showIssuer: boolean;
  showTenant: boolean;
  showGetUserInfo: boolean;
  help: {
    createUrl: string;
    redirectHint: string;
  };
};

const HELP_STEP_KEYS: Record<UplinkType, MessageKey[]> = {
  oidc: [
    "oidc.help.oidc.step1",
    "oidc.help.oidc.step2",
    "oidc.help.oidc.step3",
  ],
  github: [
    "oidc.help.github.step1",
    "oidc.help.github.step2",
    "oidc.help.github.step3",
    "oidc.help.github.step4",
    "oidc.help.github.step5",
  ],
  discord: [
    "oidc.help.discord.step1",
    "oidc.help.discord.step2",
    "oidc.help.discord.step3",
    "oidc.help.discord.step4",
    "oidc.help.discord.step5",
  ],
  microsoft: [
    "oidc.help.microsoft.step1",
    "oidc.help.microsoft.step2",
    "oidc.help.microsoft.step3",
    "oidc.help.microsoft.step4",
    "oidc.help.microsoft.step5",
    "oidc.help.microsoft.step6",
  ],
};

const CREATE_LABEL_KEYS: Partial<Record<UplinkType, MessageKey>> = {
  github: "oidc.help.github.create",
  discord: "oidc.help.discord.create",
  microsoft: "oidc.help.microsoft.create",
};

export class OidcMessageError extends Error {
  readonly messageKey: MessageKey;

  constructor(messageKey: MessageKey) {
    super(messageKey);
    this.name = "OidcMessageError";
    this.messageKey = messageKey;
  }
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "oidc",
    scopes: "openid, profile, email",
    getUserInfo: true,
    showIssuer: true,
    showTenant: false,
    showGetUserInfo: true,
    help: {
      createUrl: "",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
    },
  },
  {
    id: "github",
    scopes: "read:user, user:email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: false,
    showGetUserInfo: false,
    help: {
      createUrl: "https://github.com/settings/developers",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
    },
  },
  {
    id: "discord",
    scopes: "identify, email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: false,
    showGetUserInfo: false,
    help: {
      createUrl: "https://discord.com/developers/applications",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
    },
  },
  {
    id: "microsoft",
    tenant: "common",
    scopes: "openid, profile, email",
    getUserInfo: true,
    showIssuer: false,
    showTenant: true,
    showGetUserInfo: false,
    help: {
      createUrl:
        "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
      redirectHint: "https://auth.svc.<project>.vz.test/callback",
    },
  },
];

export function providerHelpSteps(type: UplinkType): MessageKey[] {
  return HELP_STEP_KEYS[type];
}

export function providerCreateLabelKey(type: UplinkType): MessageKey | null {
  return CREATE_LABEL_KEYS[type] ?? null;
}

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
}): MessageKey | null {
  if (!draft.clientID.trim()) return "oidc.error.clientIdRequired";
  if (draft.type === "oidc") {
    if (!draft.issuer.trim()) return "oidc.error.issuerRequired";
    if (!draft.issuer.trim().startsWith("https://")) {
      return "oidc.error.issuerHttps";
    }
  }
  if (draft.type === "microsoft" && !draft.tenant.trim()) {
    return "oidc.error.tenantRequired";
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
  const validation = validateUplinkDraft(input);
  if (validation) throw new OidcMessageError(validation);
  if (!(await isTauri())) {
    throw new OidcMessageError("oidc.error.tauriOnly");
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
      throw new OidcMessageError("oidc.error.secretRequired");
    }
  }
}

export async function saveProjectUplinkSecret(
  project: string,
  secret: string,
): Promise<void> {
  if (!(await isTauri())) {
    throw new OidcMessageError("oidc.error.secretTauriOnly");
  }
  if (!secret.trim()) throw new OidcMessageError("oidc.error.projectSecretEmpty");
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
