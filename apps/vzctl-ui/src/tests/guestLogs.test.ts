import { describe, expect, it } from "vitest";
import { buildLogsQuery, guestServiceApiPath } from "@/lib/guestLogs";

describe("buildLogsQuery", () => {
  it("encodes iwatch web filters", () => {
    expect(
      buildLogsQuery({
        q: "level=error heartbeat",
        minLevel: "warn",
        groupField: "component",
        groupValue: "api",
        filters: { msg: "fail" },
        tail: 400,
      }),
    ).toBe(
      "?q=level%3Derror+heartbeat&minLevel=warn&groupField=component&groupValue=api&tail=400&filter.msg=fail",
    );
  });

  it("omits empty query", () => {
    expect(buildLogsQuery({})).toBe("");
  });
});

describe("guestServiceApiPath", () => {
  it("keeps encoded vm ids and /api prefix", () => {
    expect(guestServiceApiPath("edge/web", "app", "/api/logs/sse")).toBe(
      "/v1/vms/edge%2Fweb/guest-services/app/api/logs/sse",
    );
  });

  it("builds the iwatch restart path", () => {
    expect(guestServiceApiPath("edge/web", "app", "/api/restart")).toBe(
      "/v1/vms/edge%2Fweb/guest-services/app/api/restart",
    );
  });
});
