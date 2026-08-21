import { api, encodeId } from "@/lib/api";
import {
  assertEnvelopeOk,
  parseEnvelope,
  waitForJob,
  type JobResponse,
  type VzctlEnvelope,
  type WaitForJobOptions,
} from "@/lib/vzctl";

export type ImageJobOptions = WaitForJobOptions;

export type ImageListItem = {
  alias: string;
  canonical_alias: string;
  aliases: string[];
  distribution: string;
  release: string;
  architecture: string;
  path: string;
  format: string;
  sha256: string;
  baked: boolean;
  sealed: boolean;
  agent_version: string | null;
};

export type ImageCatalogEntry = {
  alias: string;
  aliases: string[];
  distribution: string;
  release: string;
};

export type ImageListResult = {
  images: ImageListItem[];
  catalog: ImageCatalogEntry[];
  imagesDir: string;
  count: number;
};

export const IMAGE_ALIAS_HINTS = [
  "ubuntu-latest",
  "ubuntu-26.04",
  "ubuntu-24.04",
  "ubuntu-22.04",
  "ubuntu-20.04",
  "debian-latest",
  "debian-13",
  "debian-12",
  "debian-11",
  "alpine-latest",
  "arch-latest",
  "fedora-latest",
  "rocky-latest",
  "alma-latest",
  "opensuse-latest",
  "coreos-latest",
  "fedora-coreos-latest",
  "flatcar-latest",
  "photon-latest",
  "opensuse-microos-latest",
  "talos-latest",
] as const;

export type ImageOsVersion = {
  alias: string;
  label: string;
  isLatest: boolean;
};

export type ImageOsGroup = {
  distribution: string;
  versions: ImageOsVersion[];
};

function catalogEntryAliases(entry: ImageCatalogEntry): string[] {
  const aliases = new Set<string>([entry.alias, ...entry.aliases]);
  return [...aliases];
}

function isLatestAlias(alias: string): boolean {
  return alias.endsWith("-latest");
}

function versionLabel(entry: ImageCatalogEntry): string {
  return isLatestAlias(entry.alias)
    ? `Latest (${entry.release})`
    : entry.release;
}

/** Sort key: latest first, then descending numeric version segments. */
function versionSortKey(alias: string): [number, number, number, number] {
  if (isLatestAlias(alias)) return [0, Number.MAX_SAFE_INTEGER, 0, 0];
  const numbers = alias.match(/\d+(?:\.\d+)*/g)?.[0]?.split(".").map(Number) ?? [];
  const major = numbers[0] ?? 0;
  const minor = numbers[1] ?? 0;
  const patch = numbers[2] ?? 0;
  return [1, major, minor, patch];
}

function compareVersions(a: ImageOsVersion, b: ImageOsVersion): number {
  const left = versionSortKey(a.alias);
  const right = versionSortKey(b.alias);
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] === right[index]) continue;
    if (index === 0) return left[index]! - right[index]!;
    return right[index]! - left[index]!;
  }
  return a.alias.localeCompare(b.alias);
}

function syntheticCatalogFromHints(): ImageCatalogEntry[] {
  const distributionByPrefix: Record<string, string> = {
    ubuntu: "Ubuntu",
    debian: "Debian",
    alpine: "Alpine Linux",
    arch: "Arch Linux ARM",
    fedora: "Fedora Cloud",
    rocky: "Rocky Linux",
    alma: "AlmaLinux",
    opensuse: "openSUSE Leap",
    flatcar: "Flatcar Container Linux",
    photon: "VMware Photon OS",
    talos: "Talos Linux",
    coreos: "Fedora CoreOS",
  };
  return IMAGE_ALIAS_HINTS.map((alias) => {
    const prefix = alias.split("-")[0] ?? alias;
    const distribution =
      alias.startsWith("fedora-coreos") || alias === "coreos-latest"
        ? "Fedora CoreOS"
        : alias.startsWith("opensuse-microos")
          ? "openSUSE MicroOS"
          : (distributionByPrefix[prefix] ?? prefix);
    const release = isLatestAlias(alias) ? "latest" : alias.split("-").slice(1).join("-");
    return { alias, aliases: [alias], distribution, release };
  });
}

/** Group pull catalog entries by distribution for OS/version pickers. */
export function catalogOsGroups(catalog: ImageCatalogEntry[]): ImageOsGroup[] {
  const source = catalog.length > 0 ? catalog : syntheticCatalogFromHints();
  const byDistribution = new Map<string, Map<string, ImageOsVersion>>();

  for (const entry of source) {
    const distribution = entry.distribution.trim();
    if (!distribution) continue;
    const versions = byDistribution.get(distribution) ?? new Map<string, ImageOsVersion>();
    for (const alias of catalogEntryAliases(entry)) {
      if (versions.has(alias)) continue;
      versions.set(alias, {
        alias,
        label: versionLabel(entry),
        isLatest: isLatestAlias(alias),
      });
    }
    byDistribution.set(distribution, versions);
  }

  return [...byDistribution.entries()]
    .map(([distribution, versions]) => ({
      distribution,
      versions: [...versions.values()].sort(compareVersions),
    }))
    .sort((left, right) => left.distribution.localeCompare(right.distribution));
}

export function resolveCatalogSelection(
  groups: ImageOsGroup[],
  distribution: string,
  versionAlias: string,
): string {
  const group = groups.find((entry) => entry.distribution === distribution);
  const version = group?.versions.find((entry) => entry.alias === versionAlias);
  return version?.alias ?? versionAlias;
}

export function parseCatalogAlias(
  groups: ImageOsGroup[],
  alias: string,
): { distribution: string; versionAlias: string } | null {
  const trimmed = alias.trim();
  if (!trimmed) return null;
  for (const group of groups) {
    const version = group.versions.find((entry) => entry.alias === trimmed);
    if (version) {
      return { distribution: group.distribution, versionAlias: version.alias };
    }
  }
  return null;
}

export function defaultCatalogSelection(
  groups: ImageOsGroup[],
  preferredAlias = "ubuntu-latest",
): { distribution: string; versionAlias: string } | null {
  const parsed = parseCatalogAlias(groups, preferredAlias);
  if (parsed) return parsed;
  const first = groups[0];
  const version = first?.versions[0];
  if (!first || !version) return null;
  return { distribution: first.distribution, versionAlias: version.alias };
}

export const imageKeys = {
  all: ["images"] as const,
  list: () => [...imageKeys.all, "list"] as const,
};

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asBool(value: unknown): boolean {
  return value === true;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

function asImages(value: unknown): ImageListItem[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = row && typeof row === "object" ? (row as Record<string, unknown>) : {};
    const agent = obj.agent_version;
    return {
      alias: asString(obj.alias),
      canonical_alias: asString(obj.canonical_alias),
      aliases: asStringArray(obj.aliases),
      distribution: asString(obj.distribution),
      release: asString(obj.release),
      architecture: asString(obj.architecture, "arm64"),
      path: asString(obj.path),
      format: asString(obj.format, "raw"),
      sha256: asString(obj.sha256),
      baked: asBool(obj.baked),
      sealed: asBool(obj.sealed),
      agent_version: typeof agent === "string" ? agent : null,
    };
  });
}

function asCatalog(value: unknown): ImageCatalogEntry[] {
  if (!Array.isArray(value)) return [];
  return value.map((row) => {
    const obj = row && typeof row === "object" ? (row as Record<string, unknown>) : {};
    return {
      alias: asString(obj.alias),
      aliases: asStringArray(obj.aliases),
      distribution: asString(obj.distribution),
      release: asString(obj.release),
    };
  });
}

export function imageStateLabel(image: ImageListItem): "sealed" | "baked" | "pulled" {
  if (image.sealed) return "sealed";
  if (image.baked) return "baked";
  return "pulled";
}

/** Matches CLI/REST: 1–64 `[A-Za-z0-9][A-Za-z0-9._-]*`. */
export function validImageTag(tag: string): boolean {
  if (tag.length < 1 || tag.length > 64) return false;
  return /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(tag);
}

export const DEFAULT_IMAGE_TAG = "v1";

export async function listImages(): Promise<ImageListResult> {
  const envelope = parseEnvelope(await api.get("/v1/images"));
  assertEnvelopeOk(envelope, "image list failed");
  const summary =
    envelope.summary && typeof envelope.summary === "object"
      ? (envelope.summary as Record<string, unknown>)
      : {};
  return {
    images: asImages(envelope.images),
    catalog: asCatalog(envelope.catalog),
    imagesDir: asString(summary.images_dir),
    count: typeof summary.count === "number" ? summary.count : asImages(envelope.images).length,
  };
}

async function imageJob(
  alias: string,
  action: "pull" | "bake" | "seal",
  tag?: string,
  options: ImageJobOptions = {},
): Promise<VzctlEnvelope> {
  const body =
    action === "bake" || action === "seal"
      ? { tag: (tag ?? "").trim() }
      : undefined;
  if (body && !validImageTag(body.tag)) {
    throw new Error(
      `invalid image tag ${body.tag || "(empty)"}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*`,
    );
  }
  const accepted = await api.post<{ jobId: string }>(
    `/v1/images/${encodeId(alias)}/${action}`,
    body,
  );
  const envelope = await waitForJob(accepted.jobId, options);
  assertEnvelopeOk(envelope, `image ${action} ${alias} failed`);
  return envelope;
}

export async function pullImage(
  alias: string,
  options: ImageJobOptions = {},
): Promise<VzctlEnvelope> {
  return imageJob(alias, "pull", undefined, options);
}

export async function bakeImage(
  alias: string,
  tag: string,
  options: ImageJobOptions = {},
): Promise<VzctlEnvelope> {
  return imageJob(alias, "bake", tag, options);
}

export async function sealImage(
  target: string,
  tag: string,
  options: ImageJobOptions = {},
): Promise<VzctlEnvelope> {
  return imageJob(target, "seal", tag, options);
}

/** Re-export for callers that map job polls into UI. */
export type { JobResponse };

export function catalogAliasOptions(catalog: ImageCatalogEntry[]): string[] {
  if (catalog.length === 0) return [...IMAGE_ALIAS_HINTS];
  const aliases = new Set<string>();
  for (const entry of catalog) {
    aliases.add(entry.alias);
    for (const alias of entry.aliases) aliases.add(alias);
  }
  return [...aliases].sort();
}
