import { describe, expect, it } from "vitest";
import { dedupeIngressRoutes, parseIngressInfo } from "@/lib/ingress";

describe("ingress", () => {
  it("dedupes routes by host", () => {
    const routes = dedupeIngressRoutes([
      { host: "a.example.test", url: "https://a.example.test" },
      { host: "b.example.test", url: "https://b.example.test" },
      { host: "a.example.test", url: "https://a.example.test" },
    ]);
    expect(routes.map((route) => route.host)).toEqual([
      "a.example.test",
      "b.example.test",
    ]);
  });

  it("ignores dataSources hosts in status bundle shape", () => {
    const raw = JSON.stringify({
      sections: {
        ingress: {
          data: {
            enabled: true,
            routes: [
              {
                host: "main-node.svc.neti.vz.test",
                url: "https://main-node.svc.neti.vz.test",
                to: "main-node:18080",
              },
              {
                host: "monitos.svc.neti.vz.test",
                url: "https://monitos.svc.neti.vz.test",
                to: "main-node:5555",
              },
            ],
          },
        },
      },
    });
    const info = parseIngressInfo(raw);
    expect(info?.routes.map((route) => route.host)).toEqual([
      "main-node.svc.neti.vz.test",
      "monitos.svc.neti.vz.test",
    ]);
  });
});
