import { describe, expect, it } from "vitest";
import {
  catalogAliasOptions,
  IMAGE_ALIAS_HINTS,
  imageStateLabel,
  type ImageCatalogEntry,
  type ImageListItem,
} from "@/lib/images";

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
});
