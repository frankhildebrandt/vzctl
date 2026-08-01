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

export const VmConfigSchema = z.object({
  from: z.string().min(1),
  clone: z.enum(["linked", "full"]).default("linked"),
  dataDisk: z.string().min(1),
  cpus: z.number().int().positive().optional(),
  memory: z.union([z.string(), z.number()]).optional(),
  networks: z.array(VmNetworkSchema).min(1),
  cloudInit: z.string().optional(),
  dependsOn: z.array(z.string()).default([]),
  roles: z.array(z.enum(["router", "docker"])).default([]),
  requires: z.array(z.string()).default([]),
  ports: z.array(z.string()).default([]),
  mounts: z.array(VmMountSchema).default([]),
});

export const ImageConfigSchema = z.object({
  from: z.string().min(1),
  role: z.literal("base"),
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
