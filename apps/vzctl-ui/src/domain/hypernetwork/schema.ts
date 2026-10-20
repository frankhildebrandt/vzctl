import { z } from "zod";

const nameId = z
  .string()
  .regex(/^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$/, "ungültiger Name");

export const NetworkModeSchema = z.enum(["shared", "host"]);

export const NetworkBackendSchema = z.enum(["vmnet", "docker"]);

export const NetworkConfigSchema = z.object({
  cidr: z.string().min(1),
  mode: NetworkModeSchema,
  dhcp: z.boolean().default(false),
  /** Host NAT / Internet; false = isolated (host-only). Default true. */
  natEgress: z.boolean().default(true),
  /** `vmnet` (default) or `docker` (logical docker0 bip = .2). */
  backend: NetworkBackendSchema.default("vmnet"),
});

export const RouteConfigSchema = z.object({
  name: nameId,
  from: z.string().min(1),
  to: z.string().min(1),
  via: z.string().min(1),
});

export const ProtocolSchema = z.enum(["tcp", "udp", "icmp"]);

export const AllowRuleSchema = z.object({
  to: z.string().min(1),
  proto: ProtocolSchema,
  ports: z.array(z.number().int().min(1).max(65535)).default([]),
});

export const PolicyConfigSchema = z.object({
  name: nameId,
  network: z.string().min(1),
  via: z.string().min(1).optional(),
  forward: z.literal("deny-all"),
  allow: z.array(AllowRuleSchema).default([]),
});

export const VmNetworkSchema = z.object({
  name: z.string().min(1),
  ip: z.string().min(1),
});

export const VmMountSchema = z.object({
  source: z.string().min(1),
  target: z.string().min(1),
  readOnly: z.boolean().default(false),
});

export const ContainerConfigSchema = z.object({
  image: z.string().min(1),
  ports: z.array(z.string()).optional(),
  env: z.record(z.string(), z.string()).optional(),
  volumes: z.array(z.string()).optional(),
  restart: z.string().optional(),
  command: z.array(z.string()).optional(),
  workdir: z.string().optional(),
  user: z.string().optional(),
}).strict();

export const VmConfigSchema = z.preprocess((value) => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return value;
  }
  const vm = value as Record<string, unknown>;
  if (vm.disk !== undefined || vm.dataDisk === undefined) return value;
  const { dataDisk, ...rest } = vm;
  return { ...rest, disk: dataDisk };
}, z.object({
  from: z.string().min(1),
  clone: z.literal("linked").default("linked"),
  disk: z.string().min(1),
  cpus: z.number().int().positive().optional(),
  memory: z.union([z.string(), z.number()]).optional(),
  networks: z.array(VmNetworkSchema).min(1),
  cloudInit: z.string().optional(),
  dependsOn: z.array(z.string()).default([]),
  roles: z.array(z.enum(["router", "docker"])).default([]),
  requires: z.array(z.string()).default([]),
  ports: z.array(z.string()).default([]),
  mounts: z.array(VmMountSchema).default([]),
  composeFiles: z.array(z.string()).optional(),
  containers: z.record(z.string(), ContainerConfigSchema).optional(),
  logs: z
    .object({
      journald: z.array(z.string().min(1)).default([]),
    })
    .strict()
    .optional(),
}));

export const ImageConfigSchema = z.object({
  from: z.string().min(1),
  role: z.literal("base"),
  tag: z
    .string()
    .min(1)
    .max(64)
    .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/),
});

export const DnsConfigSchema = z.object({
  enabled: z.boolean(),
  hostResolver: z.boolean().default(true),
  hostListen: z.string().default("127.0.0.1:15353"),
  forward: z
    .object({
      enabled: z.boolean(),
      upstream: z.string().default("system"),
    })
    .default({ enabled: true, upstream: "system" }),
});

export const EgressProbeSchema = z
  .object({
    enabled: z.boolean().default(true),
    url: z
      .url()
      .default("https://captive.apple.com/")
      .refine((value) => {
        const parsed = new URL(value);
        return (
          (parsed.protocol === "http:" || parsed.protocol === "https:") &&
          parsed.username === "" &&
          parsed.password === ""
        );
      }, "Probe-URL muss HTTP(S) sein und darf keine Zugangsdaten enthalten"),
  })
  .strict();

export const NetworkResilienceSchema = z
  .object({
    egressProbe: EgressProbeSchema.default({
      enabled: true,
      url: "https://captive.apple.com/",
    }),
    restartVMsOnStuckEgress: z.boolean().default(false),
  })
  .strict();

export const ResilienceSchema = z
  .object({
    network: NetworkResilienceSchema.default({
      egressProbe: { enabled: true, url: "https://captive.apple.com/" },
      restartVMsOnStuckEgress: false,
    }),
  })
  .strict();

// These connections execute inside the configured via VM, including loopback.
const DataSourceHostSchema = z.string().refine((host) => {
  if (host.endsWith(".vz.test")) return true;
  if (!z.ipv4().safeParse(host).success) return false;
  const [first, second] = host.split(".").map(Number);
  return first === 127 || first === 10 || (first === 172 && second >= 16 && second <= 31)
    || (first === 192 && second === 168);
}, "host must end with .vz.test or be a private RFC1918 or loopback IPv4 address");

const SqlDataSourceEntrySchema = z
  .object({
    host: DataSourceHostSchema,
    port: z.number().int().min(1).max(65535),
    database: z.string().min(1),
    user: z.string().min(1),
    passwordFile: z.string().optional(),
    via: z.string().optional(),
  })
  .strict();

const NatsDataSourceEntrySchema = z
  .object({
    host: DataSourceHostSchema,
    port: z.number().int().min(1).max(65535).default(4222),
    user: z.string().optional(),
    passwordFile: z.string().optional(),
    tokenFile: z.string().optional(),
    credsFile: z.string().optional(),
    via: z.string().optional(),
  })
  .strict();

const SqlDataSourceTypeSchema = z
  .object({
    default: nameId,
    via: z.string().min(1),
    entries: z.record(z.string(), SqlDataSourceEntrySchema),
  })
  .strict();

const NatsDataSourceTypeSchema = z
  .object({
    default: nameId,
    via: z.string().min(1),
    entries: z.record(z.string(), NatsDataSourceEntrySchema),
  })
  .strict();

export const ObservabilityProbeSchema = z
  .object({
    name: nameId,
    from: z.string().min(1),
    target: z.string().min(1),
    // Rust serializes Http2xx as http2xx; accept the documented legacy spelling.
    expect: z.enum(["tcp", "http2xx", "http_2xx", "dns"])
      .transform((value) => value === "http_2xx" ? "http2xx" as const : value),
  })
  .strict();

export const ObservabilitySchema = z
  .object({
    probes: z.array(ObservabilityProbeSchema).default([]),
    dataSources: z
      .object({
        postgres: SqlDataSourceTypeSchema.optional(),
        mysql: SqlDataSourceTypeSchema.optional(),
        nats: NatsDataSourceTypeSchema.optional(),
      })
      .strict()
      .optional(),
  })
  .strict()
  .optional();

export const OidcUplinkSchema = z
  .object({
    type: z.enum(["oidc", "github", "microsoft", "discord"]).optional(),
    issuer: z.string().optional(),
    tenant: z.string().optional(),
    clientID: z.string().optional(),
    clientSecretFile: z.string().optional(),
    scopes: z.array(z.string()).optional(),
    getUserInfo: z.boolean().optional(),
  })
  .strict();

export const OidcSimpleUserSchema = z
  .object({
    username: z.string().min(1),
    email: z.string().min(1),
  })
  .passthrough();

export const OidcConfigSchema = z
  .object({
    enabled: z.boolean().default(true),
    mode: z.enum(["embedded", "oidc-simple"]).default("embedded"),
    issuer: z.string().min(1),
    listen: z.string().default("127.0.0.1:5556"),
    clients: z.literal("auto").default("auto"),
    passwordFile: z.string().optional(),
    uplink: OidcUplinkSchema.optional(),
    users: z.array(OidcSimpleUserSchema).optional(),
  })
  .passthrough();

const kubernetesName = z.string().max(63).regex(/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/);
const kubernetesResourceName = z.string().max(253).refine(
  (value) => value.split(".").every((label) => kubernetesName.safeParse(label).success),
  "Ungültiger Kubernetes-Ressourcenname",
);
export const KubernetesClusterSchema = z.object({
  version: z.string().regex(/^v[0-9]+\.[0-9]+\.[0-9]+\+k3s[0-9]+$/),
  sha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  server: nameId,
  agents: z.array(nameId).default([]),
  network: nameId,
  podCIDR: z.string().min(1),
  serviceCIDR: z.string().min(1),
  defaultNamespace: kubernetesName.default("default"),
  manifests: z.array(z.string().min(1).refine((path) => !path.includes("://"), "Nur lokale Manifestpfade sind erlaubt")).max(32).default([]),
}).strict();
export const KubernetesSchema = z.object({
  clusters: z.record(kubernetesName, KubernetesClusterSchema).default({}),
}).strict();

const ServiceKindSchema = z.enum(["ingress", "oidc", "postgres", "s3"]);
export const ServiceProviderSchema = z.object({
  kind: ServiceKindSchema,
  via: nameId.optional(),
  endpoint: z.string().min(1).optional(),
  advertiseEndpoint: z.string().min(1).optional(),
  adminUser: z.string().min(1).optional(),
  passwordFile: z.string().min(1).optional(),
  accessKeyFile: z.string().min(1).optional(),
  secretKeyFile: z.string().min(1).optional(),
  implementation: z.string().min(1).optional(),
}).strict();
const KubernetesConsumerSchema = z.object({
  cluster: kubernetesName, namespace: kubernetesName, secret: kubernetesResourceName,
}).strict();
const ServiceConsumerSchema = z.union([
  z.object({ vm: nameId }).strict(),
  z.object({ container: z.string().min(1) }).strict(),
  z.object({ kubernetes: KubernetesConsumerSchema }).strict(),
]);
const ServiceBackendSchema = z.union([
  z.object({ vm: nameId, port: z.number().int().min(1).max(65535) }).strict(),
  z.object({ kubernetes: z.object({
    cluster: kubernetesName, namespace: kubernetesName, service: kubernetesResourceName,
    port: z.number().int().min(1).max(65535),
  }).strict() }).strict(),
]);
export const ServiceClaimSchema = z.preprocess((value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  const next = { ...value } as Record<string, unknown>;
  for (const [alias, canonical] of [["clientId", "clientID"], ["redirectUris", "redirectURIs"]]) {
    if (next[canonical] === undefined && next[alias] !== undefined) {
      next[canonical] = next[alias];
      delete next[alias];
    }
  }
  return next;
}, z.object({
  kind: ServiceKindSchema,
  provider: nameId.optional(),
  database: z.string().min(1).optional(), bucket: z.string().min(1).optional(),
  clientID: z.string().min(1).optional(), redirectURIs: z.array(z.string()).default([]),
  scopes: z.array(z.string()).default([]), host: z.string().min(1).optional(),
  backend: ServiceBackendSchema.optional(), consumers: z.array(ServiceConsumerSchema).default([]),
}).strict().superRefine((claim, ctx) => {
  const field = { postgres: "database", s3: "bucket", oidc: "clientID", ingress: "host" }[claim.kind] as "database" | "bucket" | "clientID" | "host";
  if (!claim[field]) ctx.addIssue({ code: "custom", path: [field], message: "Dienstname fehlt" });
  if (claim.kind === "ingress" && !claim.backend) ctx.addIssue({ code: "custom", path: ["backend"], message: "Ingress-Backend fehlt" });
}));

export const SpecSchema = z.object({
  project: nameId,
  domain: z.string().regex(/\.vz\.test$/, "domain muss auf .vz.test enden"),
  dns: DnsConfigSchema,
  images: z.record(z.string(), ImageConfigSchema),
  networks: z.record(z.string(), NetworkConfigSchema),
  routes: z.array(RouteConfigSchema).default([]),
  policies: z.array(PolicyConfigSchema).default([]),
  ports: z.array(z.string()).default([]),
  volumes: z.record(z.string(), z.string()).default({}),
  vms: z.record(z.string(), VmConfigSchema),
  certs: z.unknown().optional(),
  ingress: z.unknown().optional(),
  oidc: OidcConfigSchema.optional(),
  resilience: ResilienceSchema.optional(),
  observability: ObservabilitySchema,
  kubernetes: KubernetesSchema.optional(),
  serviceProviders: z.record(nameId, ServiceProviderSchema).optional(),
  serviceClaims: z.record(nameId, ServiceClaimSchema).optional(),
});

export const EnvironmentSchema = z.object({
  apiVersion: z.literal("hypernetwork/v1"),
  kind: z.literal("Environment"),
  metadata: z.object({
    name: nameId,
  }),
  spec: SpecSchema,
});

export type Environment = z.infer<typeof EnvironmentSchema>;
export type Spec = z.infer<typeof SpecSchema>;
export type NetworkConfig = z.infer<typeof NetworkConfigSchema>;
export type RouteConfig = z.infer<typeof RouteConfigSchema>;
export type PolicyConfig = z.infer<typeof PolicyConfigSchema>;
export type AllowRule = z.infer<typeof AllowRuleSchema>;
export type VmConfig = z.infer<typeof VmConfigSchema>;
export type VmNetwork = z.infer<typeof VmNetworkSchema>;
export type Protocol = z.infer<typeof ProtocolSchema>;
export type NetworkMode = z.infer<typeof NetworkModeSchema>;
export type NetworkBackend = z.infer<typeof NetworkBackendSchema>;
export type OidcConfig = z.infer<typeof OidcConfigSchema>;
export type OidcUplink = z.infer<typeof OidcUplinkSchema>;
export type OidcSimpleUser = z.infer<typeof OidcSimpleUserSchema>;
