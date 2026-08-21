import { describe, expect, it } from "vitest";
import {
  catalogAliasOptions,
  catalogOsGroups,
  defaultCatalogSelection,
  IMAGE_ALIAS_HINTS,
  imageStateLabel,
  parseCatalogAlias,
  resolveCatalogSelection,
  validImageTag,
  type ImageCatalogEntry,
  type ImageListItem,
} from "@/lib/images";
import {
  mergeProgressLog,
  parseProgressLine,
  progressFromJob,
} from "@/lib/jobLog";

function sampleImage(
  overrides: Partial<ImageListItem> = {},
): ImageListItem {
  return {
    alias: "ubuntu-latest",
    canonical_alias: "ubuntu-latest",
    aliases: ["ubuntu-latest"],
    distribution: "Ubuntu",
    release: "26.04 LTS",
    architecture: "arm64",
    path: "/images/objects/a.raw",
    format: "raw",
    sha256: "a".repeat(64),
    baked: false,
    sealed: false,
    agent_version: null,
    ...overrides,
  };
}

describe("images helpers", () => {
  it("labels lifecycle state", () => {
    expect(imageStateLabel(sampleImage())).toBe("pulled");
    expect(imageStateLabel(sampleImage({ baked: true }))).toBe("baked");
    expect(imageStateLabel(sampleImage({ baked: true, sealed: true }))).toBe(
      "sealed",
    );
  });

  it("validates image tags like CLI", () => {
    expect(validImageTag("v1")).toBe(true);
    expect(validImageTag("release.1_0-rc")).toBe(true);
    expect(validImageTag("")).toBe(false);
    expect(validImageTag(".v1")).toBe(false);
    expect(validImageTag("a".repeat(65))).toBe(false);
  });

  it("flattens catalog aliases for pull options", () => {
    const catalog: ImageCatalogEntry[] = [
      {
        alias: "ubuntu-latest",
        aliases: ["ubuntu-latest"],
        distribution: "Ubuntu",
        release: "26.04",
      },
      {
        alias: "fedora-coreos-latest",
        aliases: ["fedora-coreos-latest", "coreos-latest"],
        distribution: "Fedora CoreOS",
        release: "stable",
      },
    ];
    expect(catalogAliasOptions(catalog)).toEqual([
      "coreos-latest",
      "fedora-coreos-latest",
      "ubuntu-latest",
    ]);
  });

  it("falls back to IMAGE_ALIAS_HINTS when catalog empty", () => {
    expect(catalogAliasOptions([])).toEqual([...IMAGE_ALIAS_HINTS]);
  });

  it("groups catalog entries by distribution with latest first", () => {
    const catalog: ImageCatalogEntry[] = [
      {
        alias: "ubuntu-22.04",
        aliases: ["ubuntu-22.04"],
        distribution: "Ubuntu",
        release: "22.04 LTS",
      },
      {
        alias: "ubuntu-latest",
        aliases: ["ubuntu-latest"],
        distribution: "Ubuntu",
        release: "26.04 LTS",
      },
      {
        alias: "ubuntu-24.04",
        aliases: ["ubuntu-24.04"],
        distribution: "Ubuntu",
        release: "24.04 LTS",
      },
      {
        alias: "debian-12",
        aliases: ["debian-12"],
        distribution: "Debian",
        release: "12 (Bookworm)",
      },
    ];
    const groups = catalogOsGroups(catalog);
    expect(groups.map((group) => group.distribution)).toEqual(["Debian", "Ubuntu"]);
    const ubuntu = groups.find((group) => group.distribution === "Ubuntu");
    expect(ubuntu?.versions.map((entry) => entry.alias)).toEqual([
      "ubuntu-latest",
      "ubuntu-24.04",
      "ubuntu-22.04",
    ]);
    expect(ubuntu?.versions[0]?.label).toBe("Latest (26.04 LTS)");
  });

  it("resolves and parses catalog selections", () => {
    const groups = catalogOsGroups([
      {
        alias: "ubuntu-24.04",
        aliases: ["ubuntu-24.04"],
        distribution: "Ubuntu",
        release: "24.04 LTS",
      },
    ]);
    expect(
      resolveCatalogSelection(groups, "Ubuntu", "ubuntu-24.04"),
    ).toBe("ubuntu-24.04");
    expect(parseCatalogAlias(groups, "ubuntu-24.04")).toEqual({
      distribution: "Ubuntu",
      versionAlias: "ubuntu-24.04",
    });
    expect(defaultCatalogSelection(groups, "ubuntu-24.04")).toEqual({
      distribution: "Ubuntu",
      versionAlias: "ubuntu-24.04",
    });
  });
});

describe("job log progress", () => {
  it("parses trailing percent meters", () => {
    expect(parseProgressLine("Downloading image… 12%")).toEqual({
      percent: 12,
      label: "Downloading image…",
    });
    expect(parseProgressLine("no meter")).toBeNull();
  });

  it("replaces a trailing progress line instead of appending", () => {
    const first = mergeProgressLog([], ["Downloading…", "Downloading… 10%"], (text) => ({
      id: 1,
      ts: "00:00:00",
      level: "info",
      text,
    }));
    expect(first.map((line) => line.text)).toEqual([
      "Downloading…",
      "Downloading… 10%",
    ]);
    const next = mergeProgressLog(first, ["Downloading… 40%"], (text) => ({
      id: 2,
      ts: "00:00:01",
      level: "info",
      text,
    }));
    expect(next.map((line) => line.text)).toEqual([
      "Downloading…",
      "Downloading… 40%",
    ]);
    expect(next).toHaveLength(2);
  });

  it("reads progress from job payload", () => {
    expect(
      progressFromJob({ progress: { percent: 42, label: "Normalizing" } }),
    ).toEqual({ percent: 42, label: "Normalizing" });
  });
});
