import { describe, expect, it } from "vitest";
import {
  normalizeOidcUplink,
  presetFor,
  scopesToInput,
  validateUplinkDraft,
} from "@/lib/oidcUplink";

describe("oidcUplink", () => {
  it("normalizes host uplink yaml shape", () => {
    const uplink = normalizeOidcUplink({
      type: "oidc",
      issuer: "https://login.example.com",
      clientID: "vzctl-dex",
      clientSecretFile: "client-secret",
      scopes: ["openid", "email"],
      getUserInfo: true,
    });
    expect(uplink).toEqual({
      type: "oidc",
      issuer: "https://login.example.com",
      tenant: undefined,
      clientID: "vzctl-dex",
      clientSecretFile: "client-secret",
      scopes: ["openid", "email"],
      getUserInfo: true,
    });
  });

  it("normalizes github / microsoft / discord types", () => {
    expect(normalizeOidcUplink({ type: "github", clientID: "x" })?.type).toBe(
      "github",
    );
    expect(
      normalizeOidcUplink({
        type: "microsoft",
        tenant: "common",
        clientID: "x",
      })?.tenant,
    ).toBe("common");
    expect(normalizeOidcUplink({ type: "discord", clientID: "x" })?.type).toBe(
      "discord",
    );
  });

  it("rejects unknown type as oidc fallback only when type missing", () => {
    expect(normalizeOidcUplink({ type: "ldap" })).toEqual({
      type: "oidc",
      issuer: undefined,
      tenant: undefined,
      clientID: undefined,
      clientSecretFile: undefined,
      scopes: undefined,
      getUserInfo: undefined,
    });
  });

  it("validates drafts per provider", () => {
    expect(
      validateUplinkDraft({
        type: "oidc",
        issuer: "http://bad.example",
        tenant: "",
        clientID: "x",
      }),
    ).toMatch(/https/);
    expect(
      validateUplinkDraft({
        type: "github",
        issuer: "",
        tenant: "",
        clientID: "gh",
      }),
    ).toBeNull();
    expect(
      validateUplinkDraft({
        type: "microsoft",
        issuer: "",
        tenant: "",
        clientID: "ms",
      }),
    ).toMatch(/Tenant/);
    expect(
      validateUplinkDraft({
        type: "microsoft",
        issuer: "",
        tenant: "common",
        clientID: "ms",
      }),
    ).toBeNull();
  });

  it("formats scopes and presets", () => {
    expect(scopesToInput(undefined)).toBe("openid, profile, email");
    expect(scopesToInput(undefined, "github")).toBe("read:user, user:email");
    expect(scopesToInput(["openid"])).toBe("openid");
    expect(presetFor("discord").help.createUrl).toContain("discord.com");
    expect(presetFor("microsoft").showTenant).toBe(true);
  });
});
