import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { KubernetesSchema, ServiceClaimSchema, ServiceProviderSchema } from "@/domain/hypernetwork/schema";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import { parseEnvironmentYaml, serializeEnvironmentYaml } from "@/application/services/yaml";

const cluster = {
  version: "v1.34.1+k3s1", sha256: "a".repeat(64), server: "server", network: "lan",
  podCIDR: "10.42.0.0/16", serviceCIDR: "10.43.0.0/16",
  manifests: ["kubernetes/namespace.yaml", "/tmp/workloads.yaml"],
};
describe("Kubernetes UI contract", () => {
  it("round-trips the same K3s fixture validated by Rust", () => {
    const fixture = readFileSync(new URL("../../../../crates/vzctl/tests/fixtures/validate/valid-kubernetes.yaml", import.meta.url), "utf8");
    const env = parseEnvironmentYaml(fixture);
    expect(env.spec.kubernetes?.clusters.dev.agents).toEqual(["kube-worker"]);
    expect(env.spec.kubernetes?.clusters.dev.manifests).toEqual(["kubernetes/workloads.yaml"]);
    expect(parseEnvironmentYaml(serializeEnvironmentYaml(env)).spec.kubernetes).toEqual(env.spec.kubernetes);
  });
  it("preserves all service claim kinds, bindings and file references", () => {
    const env = scaffoldEnvironment({ name: "services" });
    env.spec.serviceProviders = {
      database: ServiceProviderSchema.parse({ kind: "postgres", via: "db", endpoint: "127.0.0.1:5432", advertiseEndpoint: "db.lan.services.vz.test:5432", adminUser: "postgres", passwordFile: "secrets/admin" }),
      objects: ServiceProviderSchema.parse({ kind: "s3", via: "objects", endpoint: "http://127.0.0.1:9000", implementation: "minio", accessKeyFile: "secrets/access", secretKeyFile: "secrets/key" }),
    };
    env.spec.serviceClaims = {
      db: ServiceClaimSchema.parse({ kind: "postgres", database: "app", provider: "database", consumers: [{ vm: "app" }, { container: "docker/app" }] }),
      files: ServiceClaimSchema.parse({ kind: "s3", bucket: "app-files", consumers: [{ kubernetes: { cluster: "dev", namespace: "app", secret: "app.files" } }] }),
      login: ServiceClaimSchema.parse({ kind: "oidc", clientId: "app", redirectUris: ["https://app.test/callback"] }),
      web: ServiceClaimSchema.parse({ kind: "ingress", host: "app.test", backend: { kubernetes: { cluster: "dev", namespace: "app", service: "web", port: 8080 } } }),
    };
    const loaded = parseEnvironmentYaml(serializeEnvironmentYaml(env));
    expect(loaded.spec.serviceProviders).toEqual(env.spec.serviceProviders);
    expect(loaded.spec.serviceClaims).toEqual(env.spec.serviceClaims);
    expect(loaded.spec.serviceClaims?.login.clientID).toBe("app");
    expect(serializeEnvironmentYaml(env)).toContain("redirectURIs:");
    expect(ServiceClaimSchema.safeParse({ kind: "postgres" }).success).toBe(false);
    expect(ServiceClaimSchema.safeParse({ kind: "ingress", host: "app.test" }).success).toBe(false);
  });
  it("preserves manifest references and pinned cluster fields through YAML", () => {
    const env = scaffoldEnvironment({ name: "kube" });
    env.spec.kubernetes = KubernetesSchema.parse({ clusters: { dev: cluster } });
    expect(env.spec.kubernetes.clusters.dev.agents).toEqual([]);
    expect(env.spec.kubernetes.clusters.dev.defaultNamespace).toBe("default");
    expect(parseEnvironmentYaml(serializeEnvironmentYaml(env)).spec.kubernetes).toEqual(env.spec.kubernetes);
  });
  it.each([
    { version: "latest" }, { sha256: "bad" }, { manifests: ["https://example.test/workload.yaml"] },
    { manifests: Array(33).fill("pod.yaml") }, { defaultNamespace: "INVALID" }, { unexpected: true },
  ])("rejects invalid cluster fields: %j", (override) => {
    expect(KubernetesSchema.safeParse({ clusters: { dev: { ...cluster, ...override } } }).success).toBe(false);
  });
  it("preserves Docker declarations and explicit policy router selection", () => {
    const env = scaffoldEnvironment({ name: "docker" });
    env.spec.vms.docker = {
      from: "ubuntu-base", clone: "linked", disk: "20G", roles: ["docker"], requires: [], dependsOn: [], ports: [], mounts: [],
      networks: [{ name: "lan", ip: "10.80.0.10" }], composeFiles: ["compose.yaml"],
      containers: { app: { image: "app:1", env: { MODE: "dev" }, command: ["serve"], volumes: ["./data:/data"] } },
    };
    env.spec.policies = [{ name: "lan-policy", network: "lan", via: "router", forward: "deny-all", allow: [] }];
    const loaded = parseEnvironmentYaml(serializeEnvironmentYaml(env));
    expect(loaded.spec.vms.docker.composeFiles).toEqual(["compose.yaml"]);
    expect(loaded.spec.vms.docker.containers).toEqual(env.spec.vms.docker.containers);
    expect(loaded.spec.policies[0].via).toBe("router");
  });
});
