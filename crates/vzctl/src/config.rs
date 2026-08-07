use ipnet::Ipv4Net;
use jsonschema::error::ValidationErrorKind;
use jsonschema::JSONSchema;
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const API_VERSION: &str = "vzctl.dev/v1";
const CONFIG_FILE: &str = "hypernetwork.config.yaml";
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Human,
    Json,
}

#[derive(Debug, Eq, PartialEq)]
struct Options {
    directory: PathBuf,
    format: Format,
    export_schema: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ValidationIssue {
    pub(crate) path: String,
    pub(crate) message: String,
    pub(crate) kind: &'static str,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Environment {
    #[serde(rename = "apiVersion")]
    pub(crate) api_version: ConfigApiVersion,
    pub(crate) kind: ConfigKind,
    pub(crate) metadata: Metadata,
    pub(crate) spec: Spec,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
pub(crate) enum ConfigApiVersion {
    #[serde(rename = "hypernetwork/v1")]
    HypernetworkV1,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
pub(crate) enum ConfigKind {
    Environment,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$"))]
    pub(crate) name: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$"))]
    pub(crate) project: String,
    #[schemars(regex(
        pattern = r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*\.vz\.test$"
    ))]
    pub(crate) domain: String,
    pub(crate) dns: DnsConfig,
    /// Host network/sleep recovery policy. Persisted by apply in the supervisor.
    #[serde(default)]
    pub(crate) resilience: ResilienceConfig,
    #[schemars(length(min = 1))]
    pub(crate) images: BTreeMap<String, ImageConfig>,
    #[schemars(length(min = 1))]
    pub(crate) networks: BTreeMap<String, NetworkConfig>,
    pub(crate) routes: Vec<RouteConfig>,
    pub(crate) policies: Vec<PolicyConfig>,
    /// Stack-level host port forwards, e.g. `"8080:web:80"` or `"127.0.0.1:5432:db:5432"`.
    #[serde(default)]
    pub(crate) ports: Vec<String>,
    /// Named host directories for virtiofs mounts (`name → path`, relative to config dir).
    #[serde(default)]
    pub(crate) volumes: BTreeMap<String, String>,
    /// Local CA / trust rollout (v0.2).
    #[serde(default)]
    pub(crate) certs: Option<CertsConfig>,
    /// Caddy ingress on loopback (v0.2).
    #[serde(default)]
    pub(crate) ingress: Option<IngressConfig>,
    /// Embedded Dex OIDC (v0.2).
    #[serde(default)]
    pub(crate) oidc: Option<OidcConfig>,
    #[schemars(length(min = 1))]
    pub(crate) vms: BTreeMap<String, VmConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResilienceConfig {
    #[serde(default)]
    pub(crate) network: NetworkResilienceConfig,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkResilienceConfig {
    #[serde(default, rename = "egressProbe")]
    pub(crate) egress_probe: EgressProbeConfig,
    #[serde(default, rename = "restartVMsOnStuckEgress")]
    pub(crate) restart_vms_on_stuck_egress: bool,
}

impl Default for NetworkResilienceConfig {
    fn default() -> Self {
        Self {
            egress_probe: EgressProbeConfig::default(),
            restart_vms_on_stuck_egress: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EgressProbeConfig {
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default = "default_egress_probe_url")]
    pub(crate) url: String,
}

impl Default for EgressProbeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: default_egress_probe_url(),
        }
    }
}

fn default_egress_probe_url() -> String {
    "https://captive.apple.com/".to_string()
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CertsConfig {
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// After CA rotate: reinject into running guests, or reboot them.
    #[serde(default, rename = "onRotate")]
    pub(crate) on_rotate: CertsOnRotate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CertsOnRotate {
    #[default]
    Reinject,
    Reboot,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IngressConfig {
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Alpha/v0.2: only `127.0.0.1`.
    #[serde(default = "default_loopback")]
    pub(crate) bind: String,
    #[serde(default = "default_http_port", rename = "httpPort")]
    pub(crate) http_port: u16,
    #[serde(default = "default_https_port", rename = "httpsPort")]
    pub(crate) https_port: u16,
    /// Publish `{short}.localhost` host aliases for the same upstreams.
    #[serde(default = "default_true", rename = "hostAliases")]
    pub(crate) host_aliases: bool,
    #[serde(default = "default_true", rename = "redirectHttp")]
    pub(crate) redirect_http: bool,
    #[serde(default)]
    pub(crate) routes: Vec<IngressRoute>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IngressRoute {
    pub(crate) host: String,
    /// `vm:port` or `oidc:<port>` (Dex upstream on loopback).
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) requires: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OidcConfig {
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) mode: OidcMode,
    /// Canonical issuer URL — must be `https://auth.svc.{project}.vz.test` style (never `*.localhost`).
    pub(crate) issuer: String,
    #[serde(default = "default_oidc_listen")]
    pub(crate) listen: String,
    #[serde(default)]
    pub(crate) clients: OidcClients,
    /// Relative to config dir; bcrypt htpasswd-style file for Dex static passwords.
    #[serde(default, rename = "passwordFile")]
    pub(crate) password_file: Option<String>,
    /// Optional Dex OIDC connector (federator uplink). Partial fields override host defaults.
    #[serde(default)]
    pub(crate) uplink: Option<OidcUplink>,
    /// Dev users for `mode: oidc-simple` (username + email + optional custom claims).
    #[serde(default)]
    pub(crate) users: Vec<OidcSimpleUser>,
}

/// Picker user for `oidc-simple`. Extra YAML keys become OIDC claims.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub(crate) struct OidcSimpleUser {
    pub(crate) username: String,
    pub(crate) email: String,
    #[serde(flatten)]
    #[schemars(skip)]
    pub(crate) claims: std::collections::BTreeMap<String, Value>,
}

/// Dex upstream IdP connector. Secrets live in files, never inline.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OidcUplink {
    /// Connector kind. Omit in project overrides to inherit the host type.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) uplink_type: Option<OidcUplinkType>,
    /// Required for `type: oidc` (generic OIDC issuer URL).
    #[serde(default)]
    pub(crate) issuer: Option<String>,
    /// Microsoft Entra tenant id, or `common` / `organizations` / `consumers`.
    #[serde(default)]
    pub(crate) tenant: Option<String>,
    #[serde(default, rename = "clientID")]
    pub(crate) client_id: Option<String>,
    /// Path to secret file, or `"host"` to use the host-default secret.
    #[serde(default, rename = "clientSecretFile")]
    pub(crate) client_secret_file: Option<String>,
    #[serde(default)]
    pub(crate) scopes: Option<Vec<String>>,
    #[serde(default, rename = "getUserInfo")]
    pub(crate) get_user_info: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OidcUplinkType {
    #[default]
    Oidc,
    Github,
    Microsoft,
    Discord,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OidcMode {
    #[default]
    Embedded,
    /// Dev-only IdP: pick a user from a list, no passwords (see `users`).
    OidcSimple,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OidcClients {
    #[default]
    Auto,
}

fn default_true() -> bool {
    true
}

fn default_loopback() -> String {
    "127.0.0.1".to_string()
}

fn default_http_port() -> u16 {
    80
}

fn default_https_port() -> u16 {
    443
}

fn default_oidc_listen() -> String {
    "127.0.0.1:5556".to_string()
}

/// Parsed ingress upstream target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IngressUpstream {
    Vm { name: String, port: u16 },
    Oidc { port: u16 },
}

impl IngressUpstream {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let Some((left, right)) = raw.split_once(':') else {
            return Err("ingress route to must be vm:port or oidc:<port>".to_string());
        };
        if left.is_empty() || right.is_empty() {
            return Err("ingress route to must be vm:port or oidc:<port>".to_string());
        }
        let port: u16 = right
            .parse()
            .map_err(|_| format!("invalid port in ingress route to {raw:?}"))?;
        if port == 0 {
            return Err("ingress route port must be in 1...65535".to_string());
        }
        if left.eq_ignore_ascii_case("oidc") {
            Ok(Self::Oidc { port })
        } else {
            Ok(Self::Vm {
                name: left.to_string(),
                port,
            })
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DnsConfig {
    pub(crate) enabled: bool,
    #[serde(rename = "hostResolver")]
    pub(crate) host_resolver: bool,
    #[serde(rename = "hostListen")]
    pub(crate) host_listen: String,
    pub(crate) forward: DnsForward,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DnsForward {
    pub(crate) enabled: bool,
    pub(crate) upstream: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageConfig {
    pub(crate) from: String,
    pub(crate) role: ImageRole,
    /// Artifact pin for the sealed bake/seal product (`sealed/<alias>@<tag>.raw`).
    #[schemars(
        length(min = 1, max = 64),
        regex(pattern = "^[A-Za-z0-9][A-Za-z0-9._-]*$")
    )]
    pub(crate) tag: String,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ImageRole {
    Base,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkConfig {
    pub(crate) cidr: String,
    pub(crate) mode: NetworkMode,
    #[serde(default)]
    pub(crate) dhcp: bool,
    /// Host NAT / Internet egress via vmnet shared mode. When false, the
    /// network is host-only (`VMNET_HOST_MODE`); guests reach the Internet
    /// only via a router + `policies.allow` with `to: internet`.
    #[serde(default = "default_true", rename = "natEgress")]
    pub(crate) nat_egress: bool,
    /// `vmnet` (default): real custom-vmnet. `docker`: logical subnet on a
    /// Docker+Router VM (`docker0` bip = `.2`); no vmnet handle.
    #[serde(default, rename = "backend")]
    pub(crate) backend: NetworkBackend,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NetworkBackend {
    #[default]
    Vmnet,
    Docker,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NetworkMode {
    Shared,
    Host,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteConfig {
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$"))]
    pub(crate) name: String,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) via: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyConfig {
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$"))]
    pub(crate) name: String,
    pub(crate) network: String,
    /// Optional router VM (config key) that must apply this policy when multiple
    /// routers attach to `network`. Same semantics as `routes.*.via`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) via: Option<String>,
    pub(crate) forward: ForwardPolicy,
    #[serde(default)]
    pub(crate) allow: Vec<AllowRule>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ForwardPolicy {
    DenyAll,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowRule {
    pub(crate) to: String,
    pub(crate) proto: Protocol,
    #[serde(default)]
    pub(crate) ports: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Protocol {
    Tcp,
    Udp,
    Icmp,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmConfig {
    pub(crate) from: String,
    #[serde(default = "default_clone")]
    pub(crate) clone: CloneMode,
    #[serde(rename = "dataDisk")]
    #[schemars(regex(pattern = r"^[1-9][0-9]*(?:[KMGTP]i?B?|[kmgpt]i?b?)$"))]
    pub(crate) data_disk: String,
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub(crate) cpus: Option<u32>,
    #[serde(default)]
    #[schemars(regex(pattern = r"^[1-9][0-9]*(?:[KMGTP]i?B?|[kmgpt]i?b?)?$"))]
    pub(crate) memory: Option<String>,
    #[schemars(length(min = 1))]
    pub(crate) networks: Vec<VmNetwork>,
    #[serde(default, rename = "cloudInit")]
    pub(crate) cloud_init: Option<String>,
    #[serde(default, rename = "dependsOn")]
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) roles: Vec<String>,
    #[serde(default)]
    pub(crate) requires: Vec<String>,
    /// VM-level host port forwards, e.g. `"8080:80"` or `"127.0.0.1:8080:80"`.
    #[serde(default)]
    pub(crate) ports: Vec<String>,
    /// virtiofs mounts; `source` references `spec.volumes` name.
    #[serde(default)]
    pub(crate) mounts: Vec<VmMount>,
    /// Docker Compose files relative to the stack config (docker-role VMs only).
    #[serde(default, rename = "composeFiles")]
    pub(crate) compose_files: Vec<String>,
    /// Declarative default containers (docker-role VMs only); key = container name.
    #[serde(default)]
    pub(crate) containers: BTreeMap<String, ContainerConfig>,
}

/// Declared container under a docker-role VM (`ensure_containers`, ensure-only).
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainerConfig {
    pub(crate) image: String,
    /// Docker publish specs, e.g. `"8080:80"` or `"127.0.0.1:8080:80"`.
    #[serde(default)]
    pub(crate) ports: Vec<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    /// Bind mounts `host:container` or `host:container:ro` (host path relative to config).
    #[serde(default)]
    pub(crate) volumes: Vec<String>,
    #[serde(default)]
    pub(crate) restart: Option<String>,
    #[serde(default)]
    pub(crate) command: Vec<String>,
    #[serde(default)]
    pub(crate) workdir: Option<String>,
    #[serde(default)]
    pub(crate) user: Option<String>,
}

/// Declared host→guest virtiofs mount (`source` = volume name).
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmMount {
    pub(crate) source: String,
    pub(crate) target: String,
    #[serde(default, rename = "readOnly")]
    pub(crate) read_only: bool,
}

/// Parsed host→guest TCP port forward (Alpha: bind loopback only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PortForward {
    pub(crate) bind: String,
    pub(crate) host_port: u16,
    pub(crate) vm: String,
    pub(crate) guest_port: u16,
    pub(crate) source: String,
}

const ALLOWED_VM_ROLES: &[&str] = &["router", "docker"];
/// VirtioFS MultipleDirectoryShare device tag (reserved; not a volume name).
pub(crate) const VIRTIOFS_DEVICE_TAG: &str = "vzctl";

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CloneMode {
    Linked,
    Full,
}

fn default_clone() -> CloneMode {
    CloneMode::Linked
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmNetwork {
    pub(crate) name: String,
    pub(crate) ip: String,
}

pub(crate) fn command(args: impl Iterator<Item = String>) -> ExitCode {
    let args = args.collect::<Vec<_>>();
    let requested_format = requested_format(&args);
    let options = match parse_options(args.into_iter()) {
        Ok(options) => options,
        Err(message) => {
            emit_usage_failure(requested_format, &message);
            return ExitCode::from(EXIT_USAGE);
        }
    };

    if options.export_schema {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_schema())
                .expect("hypernetwork schema always serializes")
        );
        return ExitCode::SUCCESS;
    }

    let path = config_path(&options.directory);
    match validate_path(&path) {
        Ok(environment) => {
            match options.format {
                Format::Human => println!("valid: {}", path.display()),
                Format::Json => println!(
                    "{}",
                    success_envelope(&path, &environment.metadata.name, &environment.spec.project)
                ),
            }
            ExitCode::SUCCESS
        }
        Err(issues) => {
            emit_validation_failure(options.format, &path, &issues);
            ExitCode::from(EXIT_INVALID)
        }
    }
}

pub(crate) fn json_schema() -> Value {
    let mut schema = serde_json::to_value(schema_for!(Environment))
        .expect("hypernetwork schema always serializes");
    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "$id".to_string(),
            json!("https://vzctl.dev/schemas/hypernetwork-v1.schema.json"),
        );
        object.insert(
            "title".to_string(),
            json!("vzctl hypernetwork/v1 Environment"),
        );
    }
    schema
}

pub(crate) fn validate_path(path: &Path) -> Result<Environment, Vec<ValidationIssue>> {
    let source = fs::read_to_string(path).map_err(|error| {
        vec![ValidationIssue::new(
            "$",
            format!("cannot read {}: {error}", path.display()),
            "io",
        )]
    })?;
    let base = path.parent().map(Path::to_path_buf);
    validate_source_with_base(&source, base.as_deref())
}

#[cfg(test)]
pub(crate) fn validate_source(source: &str) -> Result<Environment, Vec<ValidationIssue>> {
    validate_source_with_base(source, None)
}

pub(crate) fn validate_source_with_base(
    source: &str,
    config_dir: Option<&Path>,
) -> Result<Environment, Vec<ValidationIssue>> {
    let document: Value = serde_yaml::from_str(source).map_err(|error| {
        let location = error
            .location()
            .map(|location| format!(" at line {}, column {}", location.line(), location.column()))
            .unwrap_or_default();
        vec![ValidationIssue::new(
            "$",
            format!("invalid YAML{location}: {error}"),
            "syntax",
        )]
    })?;

    let schema = json_schema();
    let compiled = JSONSchema::compile(&schema).map_err(|error| {
        vec![ValidationIssue::new(
            "$",
            format!("internal hypernetwork schema error: {error}"),
            "schema",
        )]
    })?;
    if let Err(errors) = compiled.validate(&document) {
        let mut issues = errors
            .map(|error| {
                let mut path = pointer_to_json_path(&error.instance_path.to_string());
                if let ValidationErrorKind::Required { property } = &error.kind {
                    if let Some(property) = property.as_str() {
                        path.push('.');
                        path.push_str(property);
                    }
                }
                ValidationIssue::new(path, error.to_string(), "schema")
            })
            .collect::<Vec<_>>();
        sort_and_deduplicate(&mut issues);
        return Err(issues);
    }

    let environment: Environment = serde_json::from_value(document).map_err(|error| {
        vec![ValidationIssue::new(
            "$",
            format!("cannot deserialize validated config: {error}"),
            "schema",
        )]
    })?;
    let issues = validate_references(&environment, config_dir);
    if issues.is_empty() {
        Ok(environment)
    } else {
        Err(issues)
    }
}

fn validate_references(
    environment: &Environment,
    config_dir: Option<&Path>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut networks = BTreeMap::new();

    let probe = &environment.spec.resilience.network.egress_probe;
    if probe.enabled && !valid_probe_url(&probe.url) {
        issues.push(ValidationIssue::new(
            "$.spec.resilience.network.egressProbe.url",
            "egress probe URL must be http(s), include a host, and contain no credentials",
            "semantic",
        ));
    }

    validate_name_keys("$.spec.images", environment.spec.images.keys(), &mut issues);
    for (name, image) in &environment.spec.images {
        if !valid_image_tag(&image.tag) {
            issues.push(ValidationIssue::new(
                format!("{}.tag", json_path_key("$.spec.images", name)),
                "image tag must be 1-64 ASCII characters: alphanumeric, dot, dash, or underscore",
                "semantic",
            ));
        }
    }
    validate_name_keys(
        "$.spec.networks",
        environment.spec.networks.keys(),
        &mut issues,
    );
    validate_name_keys("$.spec.vms", environment.spec.vms.keys(), &mut issues);
    validate_dns_keys(
        "$.spec.networks",
        environment.spec.networks.keys(),
        &mut issues,
    );
    validate_dns_keys("$.spec.vms", environment.spec.vms.keys(), &mut issues);
    validate_volume_keys(&environment.spec.volumes, config_dir, &mut issues);

    for (name, network) in &environment.spec.networks {
        let base = json_path_key("$.spec.networks", name);
        let path = format!("{base}.cidr");
        match network.cidr.parse::<Ipv4Net>() {
            Ok(cidr) => {
                let canonical = cidr.trunc();
                if cidr != canonical {
                    issues.push(ValidationIssue::new(
                        path,
                        format!("CIDR must use its network address; expected {}", canonical),
                        "semantic",
                    ));
                }
                networks.insert(name.as_str(), canonical);
            }
            Err(error) => issues.push(ValidationIssue::new(
                path,
                format!("invalid IPv4 CIDR: {error}"),
                "semantic",
            )),
        }
        if network.backend == NetworkBackend::Docker {
            if network.dhcp {
                issues.push(ValidationIssue::new(
                    format!("{base}.dhcp"),
                    "backend docker networks must not enable DHCP".to_string(),
                    "semantic",
                ));
            }
            if network.nat_egress {
                issues.push(ValidationIssue::new(
                    format!("{base}.natEgress"),
                    "backend docker networks must set natEgress: false".to_string(),
                    "semantic",
                ));
            }
        }
    }

    let mut route_names = BTreeSet::new();
    for (index, route) in environment.spec.routes.iter().enumerate() {
        let base = format!("$.spec.routes[{index}]");
        if !route_names.insert(route.name.as_str()) {
            issues.push(ValidationIssue::new(
                format!("{base}.name"),
                format!("duplicate route name {:?}", route.name),
                "semantic",
            ));
        }
        require_network(&route.from, &format!("{base}.from"), &networks, &mut issues);
        require_network(&route.to, &format!("{base}.to"), &networks, &mut issues);
        match environment.spec.vms.get(&route.via) {
            None => issues.push(ValidationIssue::new(
                format!("{base}.via"),
                format!("route via references unknown VM {:?}", route.via),
                "semantic",
            )),
            Some(vm) => {
                if !vm.roles.iter().any(|role| role == "router") {
                    issues.push(ValidationIssue::new(
                        format!("{base}.via"),
                        format!("route via VM {:?} does not have role router", route.via),
                        "semantic",
                    ));
                }
                let attached = vm
                    .networks
                    .iter()
                    .map(|network| network.name.as_str())
                    .collect::<BTreeSet<_>>();
                for network in [&route.from, &route.to] {
                    if networks.contains_key(network.as_str())
                        && !attached.contains(network.as_str())
                    {
                        issues.push(ValidationIssue::new(
                            format!("{base}.via"),
                            format!(
                                "route via VM {:?} is not attached to network {:?}",
                                route.via, network
                            ),
                            "semantic",
                        ));
                    }
                }
            }
        }
    }

    let mut policy_names = BTreeSet::new();
    for (index, policy) in environment.spec.policies.iter().enumerate() {
        let base = format!("$.spec.policies[{index}]");
        if !policy_names.insert(policy.name.as_str()) {
            issues.push(ValidationIssue::new(
                format!("{base}.name"),
                format!("duplicate policy name {:?}", policy.name),
                "semantic",
            ));
        }
        require_network(
            &policy.network,
            &format!("{base}.network"),
            &networks,
            &mut issues,
        );
        if let Some(via) = policy.via.as_ref() {
            match environment.spec.vms.get(via) {
                None => issues.push(ValidationIssue::new(
                    format!("{base}.via"),
                    format!("policy via references unknown VM {via:?}"),
                    "semantic",
                )),
                Some(vm) => {
                    if !vm.roles.iter().any(|role| role == "router") {
                        issues.push(ValidationIssue::new(
                            format!("{base}.via"),
                            format!("policy via VM {via:?} does not have role router"),
                            "semantic",
                        ));
                    }
                    let attached = vm
                        .networks
                        .iter()
                        .map(|network| network.name.as_str())
                        .collect::<BTreeSet<_>>();
                    if networks.contains_key(policy.network.as_str())
                        && !attached.contains(policy.network.as_str())
                    {
                        issues.push(ValidationIssue::new(
                            format!("{base}.via"),
                            format!(
                                "policy via VM {via:?} is not attached to network {:?}",
                                policy.network
                            ),
                            "semantic",
                        ));
                    }
                    for allow in &policy.allow {
                        if allow.to == "internet" {
                            continue;
                        }
                        if networks.contains_key(allow.to.as_str())
                            && !attached.contains(allow.to.as_str())
                        {
                            issues.push(ValidationIssue::new(
                                format!("{base}.via"),
                                format!(
                                    "policy via VM {via:?} is not attached to network {:?}",
                                    allow.to
                                ),
                                "semantic",
                            ));
                        }
                    }
                }
            }
        }
        for (allow_index, allow) in policy.allow.iter().enumerate() {
            let allow_base = format!("{base}.allow[{allow_index}]");
            if allow.to == "internet" {
                let source = &policy.network;
                let source_is_docker = environment
                    .spec
                    .networks
                    .get(source)
                    .is_some_and(|network| network.backend == NetworkBackend::Docker);
                let has_router = environment.spec.vms.values().any(|vm| {
                    vm.roles.iter().any(|r| r == "router")
                        && vm.networks.iter().any(|n| n.name == *source)
                        && vm.networks.iter().any(|n| {
                            environment
                                .spec
                                .networks
                                .get(&n.name)
                                .is_some_and(|net| net.nat_egress)
                        })
                });
                // Docker-backend sources may forward to internet without local
                // MASQUERADE; peer routers on parent nets provide NAT.
                if !has_router && !source_is_docker {
                    issues.push(ValidationIssue::new(
                        format!("{allow_base}.to"),
                        format!(
                            "policy {:?} allow to internet requires a router on {:?} that also attaches a natEgress network",
                            policy.name, source
                        ),
                        "semantic",
                    ));
                }
            } else {
                require_network(
                    &allow.to,
                    &format!("{allow_base}.to"),
                    &networks,
                    &mut issues,
                );
            }
            match allow.proto {
                Protocol::Icmp if !allow.ports.is_empty() => {
                    issues.push(ValidationIssue::new(
                        format!("{allow_base}.ports"),
                        "ICMP rules must not declare ports",
                        "semantic",
                    ));
                }
                Protocol::Tcp | Protocol::Udp if allow.ports.is_empty() => {
                    issues.push(ValidationIssue::new(
                        format!("{allow_base}.ports"),
                        "TCP/UDP rules require at least one port",
                        "semantic",
                    ));
                }
                _ => {}
            }
            for (port_index, port) in allow.ports.iter().enumerate() {
                if *port == 0 {
                    issues.push(ValidationIssue::new(
                        format!("{allow_base}.ports[{port_index}]"),
                        "port must be in 1...65535",
                        "semantic",
                    ));
                }
            }
        }
    }

    let mut assigned_ips: BTreeMap<(&str, Ipv4Addr), (&str, usize)> = BTreeMap::new();
    let mut host_binds: BTreeMap<(String, u16), String> = BTreeMap::new();
    validate_port_list(
        &environment.spec.ports,
        "$.spec.ports",
        None,
        &environment.spec.vms,
        &mut host_binds,
        &mut issues,
    );

    for (vm_name, vm) in &environment.spec.vms {
        let vm_base = json_path_key("$.spec.vms", vm_name);
        if !environment.spec.images.contains_key(&vm.from) {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.from"),
                format!("unknown image {:?}", vm.from),
                "semantic",
            ));
        }
        for (role_index, role) in vm.roles.iter().enumerate() {
            if !ALLOWED_VM_ROLES.contains(&role.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.roles[{role_index}]"),
                    format!("unsupported VM role {role:?}; allowed: router, docker"),
                    "semantic",
                ));
            }
        }
        validate_port_list(
            &vm.ports,
            &format!("{vm_base}.ports"),
            Some(vm_name.as_str()),
            &environment.spec.vms,
            &mut host_binds,
            &mut issues,
        );
        validate_vm_mounts(
            vm_name,
            &vm.mounts,
            &environment.spec.volumes,
            &vm_base,
            &mut issues,
        );
        validate_vm_containers(vm_name, vm, &vm_base, config_dir, &mut issues);
        if (!vm.compose_files.is_empty() || !vm.containers.is_empty())
            && !vm.networks.iter().any(|attachment| {
                environment
                    .spec
                    .networks
                    .get(&attachment.name)
                    .is_some_and(|network| network.backend == NetworkBackend::Docker)
            })
        {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.networks"),
                "container DNS requires an attached backend: docker network",
                "semantic",
            ));
        }
        if let Some(0) = vm.cpus {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.cpus"),
                "cpus must be greater than zero".to_string(),
                "semantic",
            ));
        }
        if let Some(memory) = &vm.memory {
            if let Err(message) = crate::parse_memory_mib(memory) {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.memory"),
                    message,
                    "semantic",
                ));
            }
        }
        let mut attachments = BTreeSet::new();
        let mut has_vmnet_attachment = false;
        for (index, attachment) in vm.networks.iter().enumerate() {
            let base = format!("{vm_base}.networks[{index}]");
            if !attachments.insert(attachment.name.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{base}.name"),
                    format!("VM has duplicate attachment to {:?}", attachment.name),
                    "semantic",
                ));
            }
            let Some(cidr) = networks.get(attachment.name.as_str()) else {
                issues.push(ValidationIssue::new(
                    format!("{base}.name"),
                    format!("unknown network {:?}", attachment.name),
                    "semantic",
                ));
                continue;
            };
            let is_docker_backend = environment
                .spec
                .networks
                .get(&attachment.name)
                .is_some_and(|network| network.backend == NetworkBackend::Docker);
            if !is_docker_backend {
                has_vmnet_attachment = true;
            }
            match attachment.ip.parse::<Ipv4Addr>() {
                Err(error) => issues.push(ValidationIssue::new(
                    format!("{base}.ip"),
                    format!("invalid IPv4 address: {error}"),
                    "semantic",
                )),
                Ok(ip) if !cidr.contains(&ip) => issues.push(ValidationIssue::new(
                    format!("{base}.ip"),
                    format!("IP {ip} is not in CIDR {cidr}"),
                    "semantic",
                )),
                Ok(ip) if ip == cidr.network() || ip == cidr.broadcast() => {
                    issues.push(ValidationIssue::new(
                        format!("{base}.ip"),
                        format!("IP {ip} is a reserved network/broadcast address in {cidr}"),
                        "semantic",
                    ));
                }
                Ok(ip) if host_offset(*cidr, ip) <= 1 => issues.push(ValidationIssue::new(
                    format!("{base}.ip"),
                    format!("IP {ip} is reserved for gateway/DNS (.0) or future use (.1)"),
                    "semantic",
                )),
                Ok(ip) => {
                    if is_docker_backend && host_offset(*cidr, ip) != 2 {
                        issues.push(ValidationIssue::new(
                            format!("{base}.ip"),
                            format!(
                                "backend docker network {:?} requires router IP .2, got {ip}",
                                attachment.name
                            ),
                            "semantic",
                        ));
                    }
                    if let Some((other_vm, other_index)) =
                        assigned_ips.insert((attachment.name.as_str(), ip), (vm_name, index))
                    {
                        issues.push(ValidationIssue::new(
                            format!("{base}.ip"),
                            format!(
                                "IP {ip} collides with {}.networks[{other_index}].ip",
                                json_path_key("$.spec.vms", other_vm)
                            ),
                            "semantic",
                        ));
                    }
                    if environment
                        .spec
                        .networks
                        .get(&attachment.name)
                        .is_some_and(|network| network.dhcp)
                    {
                        issues.push(ValidationIssue::new(
                            format!("{base}.ip"),
                            format!(
                                "static IP {ip} collides with DHCP enabled on network {:?}",
                                attachment.name
                            ),
                            "semantic",
                        ));
                    }
                }
            }
        }
        let attaches_docker_backend = vm.networks.iter().any(|attachment| {
            environment
                .spec
                .networks
                .get(&attachment.name)
                .is_some_and(|network| network.backend == NetworkBackend::Docker)
        });
        if attaches_docker_backend {
            if !has_vmnet_attachment {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.networks"),
                    "VM attached to a backend docker network also needs a vmnet attachment"
                        .to_string(),
                    "semantic",
                ));
            }
            let has_docker = vm.roles.iter().any(|role| role == "docker");
            let has_router = vm.roles.iter().any(|role| role == "router");
            if !has_docker || !has_router {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.roles"),
                    "VM attached to a backend docker network requires roles [docker, router]"
                        .to_string(),
                    "semantic",
                ));
            }
        }
        let mut dependencies = BTreeSet::new();
        for (index, dependency) in vm.depends_on.iter().enumerate() {
            let path = format!("{vm_base}.dependsOn[{index}]");
            if dependency == vm_name {
                issues.push(ValidationIssue::new(
                    path,
                    "VM cannot depend on itself",
                    "semantic",
                ));
            } else if !environment.spec.vms.contains_key(dependency) {
                issues.push(ValidationIssue::new(
                    path,
                    format!("unknown VM dependency {dependency:?}"),
                    "semantic",
                ));
            } else if !dependencies.insert(dependency.as_str()) {
                issues.push(ValidationIssue::new(
                    path,
                    format!("duplicate VM dependency {dependency:?}"),
                    "semantic",
                ));
            }
        }
    }

    for (name, network) in &environment.spec.networks {
        if network.backend != NetworkBackend::Docker {
            continue;
        }
        let owners = environment
            .spec
            .vms
            .iter()
            .filter(|(_, vm)| {
                vm.networks
                    .iter()
                    .any(|attachment| attachment.name == *name)
            })
            .map(|(vm_name, _)| vm_name.as_str())
            .collect::<Vec<_>>();
        if owners.len() != 1 {
            issues.push(ValidationIssue::new(
                format!("{}.backend", json_path_key("$.spec.networks", name)),
                format!(
                    "backend docker network {:?} requires exactly one attached VM, found {}",
                    name,
                    owners.len()
                ),
                "semantic",
            ));
        }
    }

    detect_dependency_cycles(&environment.spec.vms, &mut issues);
    validate_certs_ingress_oidc(environment, &mut issues);
    sort_and_deduplicate(&mut issues);
    issues
}

fn valid_probe_url(value: &str) -> bool {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    let Some(rest) = rest else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty() && !authority.contains('@') && !authority.chars().any(char::is_whitespace)
}

fn validate_certs_ingress_oidc(environment: &Environment, issues: &mut Vec<ValidationIssue>) {
    let oidc_enabled = environment
        .spec
        .oidc
        .as_ref()
        .is_some_and(|oidc| oidc.enabled);
    let ingress_enabled = environment
        .spec
        .ingress
        .as_ref()
        .is_some_and(|ingress| ingress.enabled);

    if let Some(certs) = &environment.spec.certs {
        let _ = certs; // enabled + onRotate validated by schema/serde
    }

    let mut oidc_route_hosts = BTreeSet::new();
    if let Some(ingress) = &environment.spec.ingress {
        let base = "$.spec.ingress";
        if ingress.enabled {
            if ingress.bind != "127.0.0.1" {
                issues.push(ValidationIssue::new(
                    format!("{base}.bind"),
                    format!(
                        "unsupported bind address {:?}; v0.2 only allows 127.0.0.1",
                        ingress.bind
                    ),
                    "semantic",
                ));
            }
            if ingress.http_port == 0 {
                issues.push(ValidationIssue::new(
                    format!("{base}.httpPort"),
                    "httpPort must be in 1...65535",
                    "semantic",
                ));
            }
            if ingress.https_port == 0 {
                issues.push(ValidationIssue::new(
                    format!("{base}.httpsPort"),
                    "httpsPort must be in 1...65535",
                    "semantic",
                ));
            }
            if ingress.routes.is_empty() {
                issues.push(ValidationIssue::new(
                    format!("{base}.routes"),
                    "ingress.enabled requires at least one route",
                    "semantic",
                ));
            }
            let mut hosts = BTreeSet::new();
            for (index, route) in ingress.routes.iter().enumerate() {
                let route_base = format!("{base}.routes[{index}]");
                if route.host.is_empty() {
                    issues.push(ValidationIssue::new(
                        format!("{route_base}.host"),
                        "ingress route host must not be empty",
                        "semantic",
                    ));
                } else if route.host.ends_with(".localhost") {
                    issues.push(ValidationIssue::new(
                        format!("{route_base}.host"),
                        "ingress route host must not use *.localhost; use *.svc.{project}.vz.test (hostAliases publishes localhost separately)",
                        "semantic",
                    ));
                } else if !hosts.insert(route.host.as_str()) {
                    issues.push(ValidationIssue::new(
                        format!("{route_base}.host"),
                        format!("duplicate ingress host {:?}", route.host),
                        "semantic",
                    ));
                }
                match IngressUpstream::parse(&route.to) {
                    Ok(IngressUpstream::Vm { name, .. }) => {
                        if !environment.spec.vms.contains_key(&name) {
                            issues.push(ValidationIssue::new(
                                format!("{route_base}.to"),
                                format!("ingress route references unknown VM {name:?}"),
                                "semantic",
                            ));
                        }
                    }
                    Ok(IngressUpstream::Oidc { .. }) => {
                        oidc_route_hosts.insert(route.host.clone());
                        if !oidc_enabled {
                            issues.push(ValidationIssue::new(
                                format!("{route_base}.to"),
                                "ingress oidc upstream requires spec.oidc.enabled",
                                "semantic",
                            ));
                        }
                    }
                    Err(message) => {
                        issues.push(ValidationIssue::new(
                            format!("{route_base}.to"),
                            message,
                            "semantic",
                        ));
                    }
                }
                for (req_index, req) in route.requires.iter().enumerate() {
                    if req == "oidc" && !oidc_enabled {
                        issues.push(ValidationIssue::new(
                            format!("{route_base}.requires[{req_index}]"),
                            "requires: [oidc] needs spec.oidc.enabled",
                            "semantic",
                        ));
                    } else if req != "oidc" && req != "guest-agent-v1" {
                        issues.push(ValidationIssue::new(
                            format!("{route_base}.requires[{req_index}]"),
                            format!("unsupported require {req:?}; allowed: oidc, guest-agent-v1"),
                            "semantic",
                        ));
                    }
                }
            }
        }
    }

    if let Some(oidc) = &environment.spec.oidc {
        let base = "$.spec.oidc";
        if oidc.enabled {
            if oidc.issuer.is_empty() {
                issues.push(ValidationIssue::new(
                    format!("{base}.issuer"),
                    "oidc.issuer must not be empty",
                    "semantic",
                ));
            } else {
                if oidc.issuer.contains(".localhost") {
                    issues.push(ValidationIssue::new(
                        format!("{base}.issuer"),
                        "oidc.issuer must never use *.localhost; use https://auth.svc.{project}.vz.test",
                        "semantic",
                    ));
                }
                if let Some(host) = oidc_issuer_host(&oidc.issuer) {
                    if ingress_enabled && !oidc_route_hosts.contains(host) {
                        issues.push(ValidationIssue::new(
                            format!("{base}.issuer"),
                            format!(
                                "oidc.issuer host {host:?} must match an ingress route with to: oidc:<port>"
                            ),
                            "semantic",
                        ));
                    }
                    let expected = format!("auth.svc.{}", environment.spec.domain);
                    if host != expected {
                        issues.push(ValidationIssue::new(
                            format!("{base}.issuer"),
                            format!("oidc.issuer host must be {expected:?} (got {host:?})"),
                            "semantic",
                        ));
                    }
                } else {
                    issues.push(ValidationIssue::new(
                        format!("{base}.issuer"),
                        "oidc.issuer must be an https:// URL",
                        "semantic",
                    ));
                }
            }
            if !oidc.listen.starts_with("127.0.0.1:") {
                issues.push(ValidationIssue::new(
                    format!("{base}.listen"),
                    format!("oidc.listen must bind 127.0.0.1 (got {:?})", oidc.listen),
                    "semantic",
                ));
            }
            match oidc.mode {
                OidcMode::OidcSimple => {
                    if oidc.password_file.is_some() {
                        issues.push(ValidationIssue::new(
                            format!("{base}.passwordFile"),
                            "oidc.passwordFile is not allowed with mode: oidc-simple",
                            "semantic",
                        ));
                    }
                    if oidc.uplink.is_some() {
                        issues.push(ValidationIssue::new(
                            format!("{base}.uplink"),
                            "oidc.uplink is not allowed with mode: oidc-simple",
                            "semantic",
                        ));
                    }
                    if oidc.users.is_empty() {
                        issues.push(ValidationIssue::new(
                            format!("{base}.users"),
                            "oidc.users must list at least one user when mode: oidc-simple",
                            "semantic",
                        ));
                    } else {
                        let mut seen = BTreeSet::new();
                        for (index, user) in oidc.users.iter().enumerate() {
                            let ubase = format!("{base}.users[{index}]");
                            if user.username.trim().is_empty() {
                                issues.push(ValidationIssue::new(
                                    format!("{ubase}.username"),
                                    "oidc user username must not be empty",
                                    "semantic",
                                ));
                            } else if !seen.insert(user.username.clone()) {
                                issues.push(ValidationIssue::new(
                                    format!("{ubase}.username"),
                                    format!("duplicate oidc user username {:?}", user.username),
                                    "semantic",
                                ));
                            }
                            if user.email.trim().is_empty() {
                                issues.push(ValidationIssue::new(
                                    format!("{ubase}.email"),
                                    "oidc user email must not be empty",
                                    "semantic",
                                ));
                            }
                        }
                    }
                }
                OidcMode::Embedded => {
                    if !oidc.users.is_empty() {
                        issues.push(ValidationIssue::new(
                            format!("{base}.users"),
                            "oidc.users is only valid with mode: oidc-simple",
                            "semantic",
                        ));
                    }
                    if let Some(uplink) = &oidc.uplink {
                        validate_oidc_uplink(uplink, &format!("{base}.uplink"), issues);
                    }
                }
            }
        }
    }

    for (vm_name, vm) in &environment.spec.vms {
        let vm_base = json_path_key("$.spec.vms", vm_name);
        for (index, req) in vm.requires.iter().enumerate() {
            if req == "oidc" && !oidc_enabled {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.requires[{index}]"),
                    "requires: [oidc] needs spec.oidc.enabled",
                    "semantic",
                ));
            } else if req != "oidc" && req != "guest-agent-v1" {
                issues.push(ValidationIssue::new(
                    format!("{vm_base}.requires[{index}]"),
                    format!("unsupported require {req:?}; allowed: oidc, guest-agent-v1"),
                    "semantic",
                ));
            }
        }
    }
}

fn oidc_issuer_host(issuer: &str) -> Option<&str> {
    let rest = issuer.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn validate_oidc_uplink(uplink: &OidcUplink, base: &str, issues: &mut Vec<ValidationIssue>) {
    // deny_unknown_fields rejects inline clientSecret.
    if let Some(issuer) = &uplink.issuer {
        if issuer.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{base}.issuer"),
                "oidc.uplink.issuer must not be empty when set",
                "semantic",
            ));
        } else if !issuer.starts_with("https://") {
            issues.push(ValidationIssue::new(
                format!("{base}.issuer"),
                "oidc.uplink.issuer must be an https:// URL",
                "semantic",
            ));
        }
    }
    if let Some(tenant) = &uplink.tenant {
        if tenant.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{base}.tenant"),
                "oidc.uplink.tenant must not be empty when set",
                "semantic",
            ));
        }
    }
    if let Some(client_id) = &uplink.client_id {
        if client_id.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{base}.clientID"),
                "oidc.uplink.clientID must not be empty when set",
                "semantic",
            ));
        }
    }
    if let Some(secret_file) = &uplink.client_secret_file {
        if secret_file.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{base}.clientSecretFile"),
                "oidc.uplink.clientSecretFile must not be empty when set",
                "semantic",
            ));
        }
    }
}

/// Collect all declared port forwards from an environment (stack + VM level).
pub(crate) fn collect_port_forwards(
    environment: &Environment,
) -> Result<Vec<PortForward>, Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let mut host_binds = BTreeMap::new();
    let mut forwards = Vec::new();
    collect_port_list(
        &environment.spec.ports,
        "$.spec.ports",
        None,
        &environment.spec.vms,
        &mut host_binds,
        &mut forwards,
        &mut issues,
    );
    for (vm_name, vm) in &environment.spec.vms {
        collect_port_list(
            &vm.ports,
            &format!("{}.ports", json_path_key("$.spec.vms", vm_name)),
            Some(vm_name.as_str()),
            &environment.spec.vms,
            &mut host_binds,
            &mut forwards,
            &mut issues,
        );
    }
    if issues.is_empty() {
        Ok(forwards)
    } else {
        Err(issues)
    }
}

fn validate_port_list(
    entries: &[String],
    base: &str,
    default_vm: Option<&str>,
    vms: &BTreeMap<String, VmConfig>,
    host_binds: &mut BTreeMap<(String, u16), String>,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut forwards = Vec::new();
    collect_port_list(
        entries,
        base,
        default_vm,
        vms,
        host_binds,
        &mut forwards,
        issues,
    );
}

fn collect_port_list(
    entries: &[String],
    base: &str,
    default_vm: Option<&str>,
    vms: &BTreeMap<String, VmConfig>,
    host_binds: &mut BTreeMap<(String, u16), String>,
    forwards: &mut Vec<PortForward>,
    issues: &mut Vec<ValidationIssue>,
) {
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("{base}[{index}]");
        match parse_port_forward(entry, default_vm) {
            Ok(forward) => {
                if forward.bind == "0.0.0.0" {
                    issues.push(ValidationIssue::new(
                        path.clone(),
                        "Alpha rejects bind 0.0.0.0; use 127.0.0.1 (Ingress is loopback-only until v0.2)",
                        "semantic",
                    ));
                } else if forward.bind != "127.0.0.1" {
                    issues.push(ValidationIssue::new(
                        path.clone(),
                        format!(
                            "unsupported bind address {:?}; Alpha only allows 127.0.0.1",
                            forward.bind
                        ),
                        "semantic",
                    ));
                }
                if !vms.contains_key(&forward.vm) {
                    issues.push(ValidationIssue::new(
                        path.clone(),
                        format!("port forward references unknown VM {:?}", forward.vm),
                        "semantic",
                    ));
                }
                let key = (forward.bind.clone(), forward.host_port);
                if let Some(other) = host_binds.insert(key.clone(), path.clone()) {
                    issues.push(ValidationIssue::new(
                        path.clone(),
                        format!(
                            "host bind {}:{} collides with {other}",
                            forward.bind, forward.host_port
                        ),
                        "semantic",
                    ));
                }
                forwards.push(forward);
            }
            Err(message) => issues.push(ValidationIssue::new(path, message, "semantic")),
        }
    }
}

/// Parse `"8080:80"`, `"127.0.0.1:8080:80"`, `"8080:web:80"`, or `"127.0.0.1:8080:web:80"`.
pub(crate) fn parse_port_forward(
    raw: &str,
    default_vm: Option<&str>,
) -> Result<PortForward, String> {
    let parts = raw.split(':').collect::<Vec<_>>();
    let (bind, host_port, vm, guest_port) = match parts.as_slice() {
        [host, guest] => {
            let vm = default_vm.ok_or_else(|| {
                "stack-level port requires VM name (hostPort:vm:guestPort)".to_string()
            })?;
            (
                "127.0.0.1".to_string(),
                parse_port_number(host, "host port")?,
                vm.to_string(),
                parse_port_number(guest, "guest port")?,
            )
        }
        [left, middle, right] => {
            if middle.chars().all(|c| c.is_ascii_digit()) {
                // bind:hostPort:guestPort (VM-level)
                let vm = default_vm.ok_or_else(|| {
                    "stack-level port with bind requires VM name (bind:hostPort:vm:guestPort)"
                        .to_string()
                })?;
                (
                    (*left).to_string(),
                    parse_port_number(middle, "host port")?,
                    vm.to_string(),
                    parse_port_number(right, "guest port")?,
                )
            } else {
                // hostPort:vm:guestPort
                (
                    "127.0.0.1".to_string(),
                    parse_port_number(left, "host port")?,
                    (*middle).to_string(),
                    parse_port_number(right, "guest port")?,
                )
            }
        }
        [bind, host, vm, guest] => (
            (*bind).to_string(),
            parse_port_number(host, "host port")?,
            (*vm).to_string(),
            parse_port_number(guest, "guest port")?,
        ),
        _ => {
            return Err(
                "invalid port forward; expected hostPort:guestPort, bind:hostPort:guestPort, hostPort:vm:guestPort, or bind:hostPort:vm:guestPort"
                    .to_string(),
            )
        }
    };
    if vm.is_empty() {
        return Err("port forward VM name must not be empty".to_string());
    }
    Ok(PortForward {
        bind,
        host_port,
        vm,
        guest_port,
        source: raw.to_string(),
    })
}

fn parse_port_number(value: &str, label: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("invalid {label}: {value}"))?;
    if port == 0 {
        return Err(format!("{label} must be in 1...65535"));
    }
    Ok(port)
}

fn detect_dependency_cycles(vms: &BTreeMap<String, VmConfig>, issues: &mut Vec<ValidationIssue>) {
    fn visit<'a>(
        vm: &'a str,
        vms: &'a BTreeMap<String, VmConfig>,
        visiting: &mut Vec<&'a str>,
        visited: &mut BTreeSet<&'a str>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        if visited.contains(vm) {
            return;
        }
        if let Some(position) = visiting.iter().position(|candidate| *candidate == vm) {
            let mut cycle = visiting[position..].to_vec();
            cycle.push(vm);
            issues.push(ValidationIssue::new(
                format!("{}.dependsOn", json_path_key("$.spec.vms", vm)),
                format!("dependency cycle: {}", cycle.join(" -> ")),
                "semantic",
            ));
            return;
        }
        visiting.push(vm);
        if let Some(config) = vms.get(vm) {
            for dependency in &config.depends_on {
                if vms.contains_key(dependency) {
                    visit(dependency, vms, visiting, visited, issues);
                }
            }
        }
        visiting.pop();
        visited.insert(vm);
    }

    let mut visited = BTreeSet::new();
    for vm in vms.keys() {
        visit(vm, vms, &mut Vec::new(), &mut visited, issues);
    }
}

fn validate_name_keys<'a>(
    base: &str,
    names: impl Iterator<Item = &'a String>,
    issues: &mut Vec<ValidationIssue>,
) {
    for name in names {
        if !valid_name(name) {
            issues.push(ValidationIssue::new(
                json_path_key(base, name),
                "name must be 1-63 ASCII characters: alphanumeric, dot, dash, or underscore",
                "semantic",
            ));
        }
    }
}

fn valid_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'-' | b'_'))
        })
}

fn validate_dns_keys<'a>(
    base: &str,
    names: impl Iterator<Item = &'a String>,
    issues: &mut Vec<ValidationIssue>,
) {
    for name in names {
        if !valid_dns_label(name) {
            issues.push(ValidationIssue::new(
                json_path_key(base, name),
                "DNS-visible name must be lowercase a-z, 0-9, hyphen; never svc",
                "semantic",
            ));
        }
    }
}

fn valid_dns_label(name: &str) -> bool {
    name != "svc"
        && (1..=63).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

pub(crate) fn valid_image_tag(tag: &str) -> bool {
    (1..=64).contains(&tag.len())
        && tag.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'-' | b'_'))
        })
}

pub(crate) fn valid_volume_name(name: &str) -> bool {
    if name == VIRTIOFS_DEVICE_TAG {
        return false;
    }
    (1..=36).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_'))
        })
}

fn validate_volume_keys(
    volumes: &BTreeMap<String, String>,
    config_dir: Option<&Path>,
    issues: &mut Vec<ValidationIssue>,
) {
    for (name, path) in volumes {
        let base = json_path_key("$.spec.volumes", name);
        if !valid_volume_name(name) {
            issues.push(ValidationIssue::new(
                base.clone(),
                format!(
                    "volume name must be 1-36 chars [A-Za-z0-9][A-Za-z0-9_-]* and must not be reserved tag {VIRTIOFS_DEVICE_TAG:?}"
                ),
                "semantic",
            ));
        }
        if path.trim().is_empty() {
            issues.push(ValidationIssue::new(
                base.clone(),
                "volume path must not be empty",
                "semantic",
            ));
            continue;
        }
        let resolved = resolve_volume_path(path, config_dir);
        match resolved {
            None => {
                // Relative path without config dir (string-only validate): skip existence.
            }
            Some(candidate) => {
                if !candidate.is_dir() {
                    issues.push(ValidationIssue::new(
                        base,
                        format!(
                            "volume path {:?} is not an existing directory",
                            candidate.display()
                        ),
                        "semantic",
                    ));
                }
            }
        }
    }
}

pub(crate) fn resolve_volume_path(path: &str, config_dir: Option<&Path>) -> Option<PathBuf> {
    let raw = PathBuf::from(path);
    if raw.is_absolute() {
        return Some(raw);
    }
    config_dir.map(|dir| dir.join(raw))
}

fn validate_vm_containers(
    vm_name: &str,
    vm: &VmConfig,
    vm_base: &str,
    config_dir: Option<&Path>,
    issues: &mut Vec<ValidationIssue>,
) {
    let has_docker = vm.roles.iter().any(|role| role == "docker");
    if !has_docker {
        if !vm.compose_files.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.composeFiles"),
                format!("composeFiles requires roles to include docker (VM {vm_name})"),
                "semantic",
            ));
        }
        if !vm.containers.is_empty() {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.containers"),
                format!("containers requires roles to include docker (VM {vm_name})"),
                "semantic",
            ));
        }
        return;
    }

    for (index, compose_file) in vm.compose_files.iter().enumerate() {
        let path = format!("{vm_base}.composeFiles[{index}]");
        if compose_file.trim().is_empty() {
            issues.push(ValidationIssue::new(
                path,
                "compose file path must not be empty",
                "semantic",
            ));
            continue;
        }
        let Some(resolved) = resolve_volume_path(compose_file, config_dir) else {
            // Without config_dir (inline tests) skip existence checks.
            continue;
        };
        if !resolved.is_file() {
            issues.push(ValidationIssue::new(
                path,
                format!("compose file not found: {}", resolved.display()),
                "semantic",
            ));
        }
    }

    for (name, container) in &vm.containers {
        let base = json_path_key(&format!("{vm_base}.containers"), name);
        if !valid_container_name(name) {
            issues.push(ValidationIssue::new(
                base.clone(),
                "container name must be 1-63 ASCII characters: alphanumeric, underscore, dot, or dash",
                "semantic",
            ));
        }
        if !valid_dns_label(name) {
            issues.push(ValidationIssue::new(
                base.clone(),
                "container DNS name must be lowercase a-z, 0-9, hyphen; never svc",
                "semantic",
            ));
        }
        if container.image.trim().is_empty() {
            issues.push(ValidationIssue::new(
                format!("{base}.image"),
                "container image must not be empty",
                "semantic",
            ));
        }
        for (index, publish) in container.ports.iter().enumerate() {
            if let Err(message) = validate_container_publish(publish) {
                issues.push(ValidationIssue::new(
                    format!("{base}.ports[{index}]"),
                    message,
                    "semantic",
                ));
            }
        }
        for (index, volume) in container.volumes.iter().enumerate() {
            if let Err(message) = validate_container_volume(volume, config_dir) {
                issues.push(ValidationIssue::new(
                    format!("{base}.volumes[{index}]"),
                    message,
                    "semantic",
                ));
            }
        }
        if let Some(restart) = &container.restart {
            if !valid_restart_policy(restart) {
                issues.push(ValidationIssue::new(
                    format!("{base}.restart"),
                    format!(
                        "unsupported restart policy {restart:?}; allowed: no, always, on-failure, unless-stopped"
                    ),
                    "semantic",
                ));
            }
        }
    }
}

fn valid_container_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'.' | b'-'))
        })
}

fn valid_restart_policy(policy: &str) -> bool {
    matches!(policy, "no" | "always" | "on-failure" | "unless-stopped")
        || policy.starts_with("on-failure:")
}

/// Validate a Docker `-p` publish string (`80`, `8080:80`, `127.0.0.1:8080:80`, optional `/tcp`).
fn validate_container_publish(raw: &str) -> Result<(), String> {
    let without_proto = raw.split('/').next().unwrap_or(raw);
    if without_proto.is_empty() {
        return Err("publish spec must not be empty".into());
    }
    let parts = without_proto.split(':').collect::<Vec<_>>();
    if !(1..=3).contains(&parts.len()) {
        return Err(format!(
            "invalid publish {raw:?}; expected PORT, HOST:CONTAINER, or IP:HOST:CONTAINER"
        ));
    }
    let container_port = parts.last().copied().unwrap_or("");
    if container_port.parse::<u16>().is_err() {
        return Err(format!("invalid container port in publish {raw:?}"));
    }
    if parts.len() >= 2 {
        let host_port = parts[parts.len() - 2];
        if host_port.parse::<u16>().is_err() {
            return Err(format!("invalid host port in publish {raw:?}"));
        }
    }
    if parts.len() == 3 {
        let bind = parts[0];
        if bind.parse::<Ipv4Addr>().is_err() && bind != "localhost" {
            return Err(format!(
                "invalid bind address in publish {raw:?}; use IPv4 or localhost"
            ));
        }
    }
    Ok(())
}

fn validate_container_volume(raw: &str, config_dir: Option<&Path>) -> Result<(), String> {
    let parts = raw.split(':').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!(
            "invalid volume {raw:?}; expected HOST:CONTAINER or HOST:CONTAINER:ro|rw"
        ));
    }
    if parts[0].is_empty() || parts[1].is_empty() {
        return Err(format!(
            "invalid volume {raw:?}; host and container paths required"
        ));
    }
    if !parts[1].starts_with('/') {
        return Err(format!(
            "container volume target must be absolute: {:?}",
            parts[1]
        ));
    }
    if parts.len() == 3 && !matches!(parts[2], "ro" | "rw") {
        return Err(format!(
            "invalid volume mode {:?}; expected ro or rw",
            parts[2]
        ));
    }
    let host = parts[0];
    if let Some(resolved) = resolve_volume_path(host, config_dir) {
        // Parent may not exist yet; only reject empty host path after resolve.
        if resolved.as_os_str().is_empty() {
            return Err(format!("cannot resolve volume host path {host:?}"));
        }
    }
    Ok(())
}

fn validate_vm_mounts(
    vm_name: &str,
    mounts: &[VmMount],
    volumes: &BTreeMap<String, String>,
    vm_base: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut targets = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for (index, mount) in mounts.iter().enumerate() {
        let base = format!("{vm_base}.mounts[{index}]");
        if !volumes.contains_key(&mount.source) {
            issues.push(ValidationIssue::new(
                format!("{base}.source"),
                format!(
                    "mount source {:?} references unknown volume (VM {vm_name})",
                    mount.source
                ),
                "semantic",
            ));
        } else if !sources.insert(mount.source.as_str()) {
            issues.push(ValidationIssue::new(
                format!("{base}.source"),
                format!("duplicate mount source {:?}", mount.source),
                "semantic",
            ));
        }
        if !mount.target.starts_with('/') || mount.target.len() < 2 {
            issues.push(ValidationIssue::new(
                format!("{base}.target"),
                "mount target must be an absolute path (not /)",
                "semantic",
            ));
        } else if !targets.insert(mount.target.as_str()) {
            issues.push(ValidationIssue::new(
                format!("{base}.target"),
                format!("duplicate mount target {:?}", mount.target),
                "semantic",
            ));
        }
    }
}

fn require_network(
    name: &str,
    path: &str,
    networks: &BTreeMap<&str, Ipv4Net>,
    issues: &mut Vec<ValidationIssue>,
) {
    if !networks.contains_key(name) {
        issues.push(ValidationIssue::new(
            path,
            format!("unknown network {name:?}"),
            "semantic",
        ));
    }
}

fn host_offset(cidr: Ipv4Net, ip: Ipv4Addr) -> u32 {
    u32::from(ip) - u32::from(cidr.network())
}

fn sort_and_deduplicate(issues: &mut Vec<ValidationIssue>) {
    issues.sort_by(|left, right| {
        (&left.path, &left.message, left.kind).cmp(&(&right.path, &right.message, right.kind))
    });
    issues.dedup_by(|left, right| {
        left.path == right.path && left.message == right.message && left.kind == right.kind
    });
}

fn pointer_to_json_path(pointer: &str) -> String {
    let mut path = "$".to_string();
    for segment in pointer.split('/').skip(1) {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        if segment.chars().all(|character| character.is_ascii_digit()) {
            path.push('[');
            path.push_str(&segment);
            path.push(']');
        } else if segment
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        {
            path.push('.');
            path.push_str(&segment);
        } else {
            path.push_str(&format!("[{}]", json!(segment)));
        }
    }
    path
}

fn json_path_key(base: &str, key: &str) -> String {
    if key.chars().enumerate().all(|(index, character)| {
        character.is_ascii_alphanumeric() || (index > 0 && character == '_')
    }) {
        format!("{base}.{key}")
    } else {
        format!("{base}[{}]", json!(key))
    }
}

fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut directory = PathBuf::from(".");
    let mut format = Format::Human;
    let mut export_schema = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-C" => {
                let value = args
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "-C requires a directory or config path".to_string())?;
                directory = PathBuf::from(value);
            }
            "--format" => {
                format = match args.next().as_deref() {
                    Some("human") => Format::Human,
                    Some("json") => Format::Json,
                    Some(value) => return Err(format!("unsupported validate format: {value}")),
                    None => return Err("--format requires human or json".to_string()),
                };
            }
            "--schema" => export_schema = true,
            "-h" | "--help" => return Err(usage().to_string()),
            _ => return Err(format!("unknown validate option: {argument}")),
        }
    }
    if export_schema && directory != Path::new(".") {
        return Err("--schema cannot be combined with -C".to_string());
    }
    if export_schema && format != Format::Human {
        return Err("--schema cannot be combined with --format".to_string());
    }
    Ok(Options {
        directory,
        format,
        export_schema,
    })
}

fn requested_format(args: &[String]) -> Format {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| Format::Json)
        .unwrap_or(Format::Human)
}

pub(crate) fn config_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(CONFIG_FILE)
    } else {
        path.to_path_buf()
    }
}

fn success_envelope(path: &Path, name: &str, project: &str) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "validate",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": "hypernetwork/v1 config is valid",
            "errors": 0,
        },
        "config": {
            "path": path,
            "apiVersion": "hypernetwork/v1",
            "name": name,
            "project": project,
        },
        "errors": [],
    })
}

fn emit_usage_failure(format: Format, message: &str) {
    match format {
        Format::Human => eprintln!("{message}"),
        Format::Json => println!(
            "{}",
            failure_envelope(
                EXIT_USAGE,
                "invalid validate command",
                &[ValidationIssue::new("$", message, "usage")]
            )
        ),
    }
}

fn emit_validation_failure(format: Format, path: &Path, issues: &[ValidationIssue]) {
    match format {
        Format::Human => {
            eprintln!("invalid: {}", path.display());
            for issue in issues {
                eprintln!("  {}: {}", issue.path, issue.message);
            }
        }
        Format::Json => println!(
            "{}",
            failure_envelope(
                EXIT_INVALID,
                &format!(
                    "hypernetwork/v1 config has {} validation error(s)",
                    issues.len()
                ),
                issues
            )
        ),
    }
}

fn failure_envelope(exit_code: u8, message: &str, issues: &[ValidationIssue]) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "command": "validate",
        "status": "fail",
        "exit_code": exit_code,
        "summary": {
            "message": message,
            "errors": issues.len(),
        },
        "errors": issues,
    })
}

fn usage() -> &'static str {
    "usage: vzctl validate [-C <directory|config>] [--format human|json]\n       vzctl validate --schema"
}

impl ValidationIssue {
    fn new(path: impl Into<String>, message: impl Into<String>, kind: &'static str) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            kind,
        }
    }
}

/// Normalize a project name like the UI scaffold (lowercase, safe chars, max 63).
pub(crate) fn normalize_project_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = trimmed.to_ascii_lowercase();
    normalized = normalized
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    while normalized.starts_with('-') {
        normalized.remove(0);
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    if normalized.len() > 63 {
        normalized.truncate(63);
        while normalized.ends_with('-') {
            normalized.pop();
        }
    }
    if normalized.is_empty() {
        return None;
    }
    let first = normalized.chars().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !normalized
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return None;
    }
    Some(normalized)
}

pub(crate) fn scaffold_environment(name: &str, cidr: &str) -> Result<Environment, String> {
    let name = normalize_project_name(name).ok_or("invalid or empty project name")?;
    if cidr.parse::<Ipv4Net>().is_err() {
        return Err(format!("invalid CIDR: {cidr}"));
    }
    Ok(Environment {
        api_version: ConfigApiVersion::HypernetworkV1,
        kind: ConfigKind::Environment,
        metadata: Metadata { name: name.clone() },
        spec: Spec {
            project: name.clone(),
            domain: format!("{name}.vz.test"),
            dns: DnsConfig {
                enabled: true,
                host_resolver: true,
                host_listen: "127.0.0.1:15353".to_string(),
                forward: DnsForward {
                    enabled: true,
                    upstream: "system".to_string(),
                },
            },
            resilience: ResilienceConfig {
                network: NetworkResilienceConfig::default(),
            },
            images: BTreeMap::from([(
                "ubuntu-base".to_string(),
                ImageConfig {
                    from: "ubuntu-latest".to_string(),
                    role: ImageRole::Base,
                    tag: "v1".to_string(),
                },
            )]),
            networks: BTreeMap::from([(
                "lan".to_string(),
                NetworkConfig {
                    cidr: cidr.to_string(),
                    mode: NetworkMode::Shared,
                    dhcp: false,
                    nat_egress: true,
                    backend: NetworkBackend::Vmnet,
                },
            )]),
            routes: Vec::new(),
            policies: Vec::new(),
            ports: Vec::new(),
            volumes: BTreeMap::new(),
            certs: None,
            ingress: None,
            oidc: None,
            vms: BTreeMap::new(),
        },
    })
}

pub(crate) fn next_free_ip(
    environment: &Environment,
    network_name: &str,
) -> Result<String, String> {
    let network = environment
        .spec
        .networks
        .get(network_name)
        .ok_or_else(|| format!("unknown network {network_name:?}"))?;
    let cidr = network
        .cidr
        .parse::<Ipv4Net>()
        .map_err(|error| format!("invalid CIDR {}: {error}", network.cidr))?;
    let mut used = BTreeSet::new();
    for vm in environment.spec.vms.values() {
        for attachment in &vm.networks {
            if attachment.name == network_name {
                if let Ok(ip) = attachment.ip.parse::<Ipv4Addr>() {
                    used.insert(ip);
                }
            }
        }
    }
    if network.backend == NetworkBackend::Docker {
        let ip = offset_ip(cidr, 2)?;
        if used.contains(&ip) {
            return Err(format!(
                "docker network {network_name:?} already has owner at {ip}"
            ));
        }
        return Ok(ip.to_string());
    }
    for offset in 10..254 {
        let ip = offset_ip(cidr, offset)?;
        if cidr.contains(&ip) && !used.contains(&ip) {
            return Ok(ip.to_string());
        }
    }
    Err(format!("no free IP on network {network_name:?}"))
}

fn offset_ip(cidr: Ipv4Net, offset: u32) -> Result<Ipv4Addr, String> {
    let network = u32::from(cidr.network());
    let broadcast = u32::from(cidr.broadcast());
    let candidate = network + offset;
    if candidate > broadcast {
        return Err(format!("host offset {offset} out of range for {cidr}"));
    }
    Ok(Ipv4Addr::from(candidate))
}

pub(crate) fn serialize_environment_yaml(environment: &Environment) -> String {
    let spec = &environment.spec;
    let mut spec_map = serde_json::Map::new();
    spec_map.insert("project".to_string(), json!(spec.project));
    spec_map.insert("domain".to_string(), json!(spec.domain));
    spec_map.insert("dns".to_string(), serde_json::to_value(&spec.dns).unwrap());
    spec_map.insert(
        "images".to_string(),
        serde_json::to_value(&spec.images).unwrap(),
    );
    spec_map.insert(
        "networks".to_string(),
        serde_json::to_value(&spec.networks).unwrap(),
    );
    spec_map.insert("routes".to_string(), json!(spec.routes));
    spec_map.insert("policies".to_string(), json!(spec.policies));
    if !spec.volumes.is_empty() {
        spec_map.insert(
            "volumes".to_string(),
            serde_json::to_value(&spec.volumes).unwrap(),
        );
    }
    if !spec.ports.is_empty() {
        spec_map.insert("ports".to_string(), json!(spec.ports));
    }
    if spec.certs.is_some() {
        spec_map.insert(
            "certs".to_string(),
            serde_json::to_value(&spec.certs).unwrap(),
        );
    }
    if spec.ingress.is_some() {
        spec_map.insert(
            "ingress".to_string(),
            serde_json::to_value(&spec.ingress).unwrap(),
        );
    }
    if spec.oidc.is_some() {
        spec_map.insert(
            "oidc".to_string(),
            serde_json::to_value(&spec.oidc).unwrap(),
        );
    }
    spec_map.insert(
        "resilience".to_string(),
        serde_json::to_value(&spec.resilience).unwrap(),
    );
    spec_map.insert("vms".to_string(), serde_json::to_value(&spec.vms).unwrap());

    let document = json!({
        "apiVersion": "hypernetwork/v1",
        "kind": "Environment",
        "metadata": { "name": environment.metadata.name },
        "spec": Value::Object(spec_map),
    });
    serde_yaml::to_string(&document).expect("hypernetwork environment always serializes to YAML")
}

pub(crate) fn write_environment_atomic(
    path: &Path,
    environment: &Environment,
) -> Result<Environment, Vec<ValidationIssue>> {
    let config_dir = path.parent();
    let yaml = serialize_environment_yaml(environment);
    let validated = validate_source_with_base(&yaml, config_dir.as_deref())?;
    let tmp_path = path.with_extension("yaml.tmp");
    fs::write(&tmp_path, yaml).map_err(|error| {
        vec![ValidationIssue::new(
            "$",
            format!("cannot write {}: {error}", tmp_path.display()),
            "io",
        )]
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        vec![ValidationIssue::new(
            "$",
            format!(
                "cannot rename {} to {}: {error}",
                tmp_path.display(),
                path.display()
            ),
            "io",
        )]
    })?;
    Ok(validated)
}

pub(crate) struct AddVmOptions {
    pub(crate) from_image: String,
    pub(crate) network: Option<String>,
    pub(crate) ip: Option<String>,
    pub(crate) data_disk: String,
    pub(crate) cpus: Option<u32>,
    pub(crate) memory: Option<String>,
    pub(crate) roles: Vec<String>,
    pub(crate) cloud_init: Option<String>,
}

/// Resolve a VM `from` reference: image config key or pull alias (`spec.images.*.from`).
pub(crate) fn resolve_image_key(
    environment: &Environment,
    reference: &str,
) -> Result<String, String> {
    if environment.spec.images.contains_key(reference) {
        return Ok(reference.to_string());
    }
    let matches = environment
        .spec
        .images
        .iter()
        .filter(|(_, image)| image.from == reference)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    match matches.len() {
        0 => {
            let available = environment
                .spec
                .images
                .iter()
                .map(|(key, image)| format!("{key} (from {alias})", alias = image.from))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "unknown image {reference:?}; available: {available}"
            ))
        }
        1 => Ok(matches[0].clone()),
        _ => Err(format!(
            "image alias {reference:?} is ambiguous; use one of: {}",
            matches.join(", ")
        )),
    }
}

pub(crate) fn add_vm(
    environment: &mut Environment,
    name: &str,
    options: &AddVmOptions,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.vms.contains_key(name) {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.vms", name),
            format!("VM {name:?} already exists"),
            "semantic",
        )]);
    }
    let from_image = resolve_image_key(environment, &options.from_image)
        .map_err(|message| vec![ValidationIssue::new("$.spec.images", message, "semantic")])?;
    let network_name = if let Some(name) = options.network.clone() {
        name
    } else {
        environment
            .spec
            .networks
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| {
                vec![ValidationIssue::new(
                    "$.spec.networks",
                    "no networks defined; add a network first",
                    "semantic",
                )]
            })?
    };
    if !environment.spec.networks.contains_key(&network_name) {
        return Err(vec![ValidationIssue::new(
            "$.spec.networks",
            format!("unknown network {network_name:?}"),
            "semantic",
        )]);
    }
    let ip = match &options.ip {
        Some(ip) => ip.clone(),
        None => next_free_ip(environment, &network_name).map_err(|message| {
            vec![ValidationIssue::new(
                json_path_key("$.spec.vms", name),
                message,
                "semantic",
            )]
        })?,
    };
    environment.spec.vms.insert(
        name.to_string(),
        VmConfig {
            from: from_image,
            clone: CloneMode::Linked,
            data_disk: options.data_disk.clone(),
            cpus: options.cpus,
            memory: options.memory.clone(),
            networks: vec![VmNetwork {
                name: network_name,
                ip,
            }],
            cloud_init: options.cloud_init.clone(),
            depends_on: Vec::new(),
            roles: options.roles.clone(),
            requires: Vec::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
            compose_files: Vec::new(),
            containers: BTreeMap::new(),
        },
    );
    Ok(())
}

pub(crate) fn remove_vm(
    environment: &mut Environment,
    name: &str,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.vms.remove(name).is_none() {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.vms", name),
            format!("VM {name:?} does not exist"),
            "semantic",
        )]);
    }
    Ok(())
}

pub(crate) struct AddNetworkOptions {
    pub(crate) cidr: String,
    pub(crate) mode: NetworkMode,
    pub(crate) backend: NetworkBackend,
    pub(crate) nat_egress: bool,
}

pub(crate) fn add_network(
    environment: &mut Environment,
    name: &str,
    options: &AddNetworkOptions,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.networks.contains_key(name) {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.networks", name),
            format!("network {name:?} already exists"),
            "semantic",
        )]);
    }
    environment.spec.networks.insert(
        name.to_string(),
        NetworkConfig {
            cidr: options.cidr.clone(),
            mode: options.mode,
            dhcp: false,
            nat_egress: options.nat_egress,
            backend: options.backend,
        },
    );
    Ok(())
}

pub(crate) fn remove_network(
    environment: &mut Environment,
    name: &str,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.networks.remove(name).is_none() {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.networks", name),
            format!("network {name:?} does not exist"),
            "semantic",
        )]);
    }
    Ok(())
}

pub(crate) fn add_volume(
    environment: &mut Environment,
    name: &str,
    path: &str,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.volumes.contains_key(name) {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.volumes", name),
            format!("volume {name:?} already exists"),
            "semantic",
        )]);
    }
    environment
        .spec
        .volumes
        .insert(name.to_string(), path.to_string());
    Ok(())
}

pub(crate) fn remove_volume(
    environment: &mut Environment,
    name: &str,
) -> Result<(), Vec<ValidationIssue>> {
    if environment.spec.volumes.remove(name).is_none() {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.volumes", name),
            format!("volume {name:?} does not exist"),
            "semantic",
        )]);
    }
    Ok(())
}

pub(crate) fn add_mount(
    environment: &mut Environment,
    vm_name: &str,
    source: &str,
    target: &str,
    read_only: bool,
) -> Result<(), Vec<ValidationIssue>> {
    let vm = environment.spec.vms.get_mut(vm_name).ok_or_else(|| {
        vec![ValidationIssue::new(
            json_path_key("$.spec.vms", vm_name),
            format!("VM {vm_name:?} does not exist"),
            "semantic",
        )]
    })?;
    if !environment.spec.volumes.contains_key(source) {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.volumes", source),
            format!("unknown volume {source:?}"),
            "semantic",
        )]);
    }
    if vm.mounts.iter().any(|mount| mount.target == target) {
        return Err(vec![ValidationIssue::new(
            json_path_key("$.spec.vms", vm_name),
            format!("mount target {target:?} already exists on VM {vm_name:?}"),
            "semantic",
        )]);
    }
    vm.mounts.push(VmMount {
        source: source.to_string(),
        target: target.to_string(),
        read_only,
    });
    Ok(())
}

pub(crate) fn remove_mount(
    environment: &mut Environment,
    vm_name: &str,
    target: &str,
) -> Result<(), Vec<ValidationIssue>> {
    let vm = environment.spec.vms.get_mut(vm_name).ok_or_else(|| {
        vec![ValidationIssue::new(
            json_path_key("$.spec.vms", vm_name),
            format!("VM {vm_name:?} does not exist"),
            "semantic",
        )]
    })?;
    let index = vm
        .mounts
        .iter()
        .position(|mount| mount.target == target)
        .ok_or_else(|| {
            vec![ValidationIssue::new(
                json_path_key("$.spec.vms", vm_name),
                format!("mount target {target:?} not found on VM {vm_name:?}"),
                "semantic",
            )]
        })?;
    vm.mounts.remove(index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_schema_rejects_missing_required_sections() {
        let schema = json_schema();
        let compiled = JSONSchema::compile(&schema).unwrap();
        let document = json!({
            "apiVersion": "hypernetwork/v1",
            "kind": "Environment",
            "metadata": { "name": "edge-dmz" },
            "spec": { "project": "edge-dmz" }
        });
        let paths = compiled
            .validate(&document)
            .unwrap_err()
            .map(|error| {
                let mut path = pointer_to_json_path(&error.instance_path.to_string());
                if let ValidationErrorKind::Required { property } = error.kind {
                    path.push('.');
                    path.push_str(property.as_str().unwrap());
                }
                path
            })
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("$.spec.domain"));
        assert!(paths.contains("$.spec.vms"));
    }

    #[test]
    fn positive_fixture_deserializes_to_typed_config() {
        let environment =
            validate_source(include_str!("../tests/fixtures/validate/valid-full.yaml")).unwrap();
        assert_eq!(environment.metadata.name, "edge-dmz");
        assert_eq!(environment.spec.images["ubuntu-base"].tag, "v1");
        assert_eq!(environment.spec.vms["router"].networks.len(), 2);
        assert_eq!(environment.spec.vms["web"].cpus, Some(2));
        assert_eq!(environment.spec.vms["web"].memory.as_deref(), Some("2Gi"));
        assert_eq!(environment.spec.ports.len(), 1);
        assert_eq!(
            environment.spec.vms["web"].ports,
            vec!["8080:80".to_string()]
        );
        assert!(environment.spec.vms["docker"]
            .roles
            .iter()
            .any(|role| role == "docker"));
    }

    #[test]
    fn docker_backend_fixture_validates() {
        let environment = validate_source(include_str!(
            "../tests/fixtures/validate/valid-docker-backend.yaml"
        ))
        .unwrap();
        assert_eq!(
            environment.spec.networks["containers"].backend,
            NetworkBackend::Docker
        );
        assert!(environment.spec.vms["docker"]
            .roles
            .iter()
            .any(|role| role == "router"));
    }

    #[test]
    fn docker_containers_fixture_validates_with_base() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/validate/valid-docker-containers.yaml");
        let environment = validate_path(&path).unwrap();
        let docker = &environment.spec.vms["docker"];
        assert_eq!(docker.compose_files, vec!["compose.yaml".to_string()]);
        assert_eq!(docker.containers["redis"].image, "redis:7-alpine");
        assert_eq!(
            docker.containers["redis"].restart.as_deref(),
            Some("unless-stopped")
        );
    }

    #[test]
    fn dns_visible_vm_network_and_container_names_reserve_svc() {
        let source = include_str!("../tests/fixtures/validate/valid-docker-containers.yaml");
        for changed in [
            source
                .replacen("    lan:", "    svc:", 1)
                .replace("name: lan", "name: svc"),
            source.replacen("    docker:", "    svc:", 1),
            source.replacen("        redis:", "        svc:", 1),
        ] {
            let issues = validate_source(&changed).unwrap_err();
            assert!(issues
                .iter()
                .any(|issue| issue.message.contains("never svc")));
        }
    }

    #[test]
    fn containers_require_docker_role() {
        let issues = validate_source(include_str!(
            "../tests/fixtures/validate/invalid-containers-no-docker-role.yaml"
        ))
        .unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue.path.contains(".containers") && issue.message.contains("docker")
        }));
    }

    #[test]
    fn missing_compose_file_is_rejected() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: bad }
spec:
  project: bad
  domain: bad.vz.test
  dns: { enabled: true, hostResolver: true, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
  images: { ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 } }
  networks:
    lan: { cidr: 10.90.0.0/24, mode: shared }
  routes: []
  policies: []
  vms:
    docker:
      from: ubuntu-base
      dataDisk: 40G
      networks:
        - { name: lan, ip: 10.90.0.10 }
      roles: [docker]
      composeFiles: [missing-compose.yaml]
"#;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate");
        let issues = validate_source_with_base(source, Some(&dir)).unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue.path.contains("composeFiles") && issue.message.contains("not found")
        }));
    }

    #[test]
    fn docker_backend_rejects_missing_router_role() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: bad }
spec:
  project: bad
  domain: bad.vz.test
  dns: { enabled: true, hostResolver: true, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
  images: { ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 } }
  networks:
    lan: { cidr: 10.90.0.0/24, mode: shared, natEgress: false }
    containers: { cidr: 10.95.0.0/24, mode: shared, natEgress: false, backend: docker }
  routes: []
  policies: []
  vms:
    docker:
      from: ubuntu-base
      dataDisk: 40G
      networks:
        - { name: lan, ip: 10.90.0.10 }
        - { name: containers, ip: 10.95.0.2 }
      roles: [docker]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("docker, router")));
    }

    #[test]
    fn rejects_invalid_image_tag() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: bad }
spec:
  project: bad
  domain: bad.vz.test
  dns: { enabled: true, hostResolver: true, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
  images: { ubuntu-base: { from: ubuntu-latest, role: base, tag: "-bad" } }
  networks:
    lan: { cidr: 10.90.0.0/24, mode: shared }
  routes: []
  policies: []
  vms:
    web:
      from: ubuntu-base
      dataDisk: 8G
      networks:
        - { name: lan, ip: 10.90.0.10 }
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue.path.contains(".tag")
                && (issue.message.contains("image tag") || issue.message.contains("does not match"))
        }));
    }

    #[test]
    fn port_forward_parser_accepts_vm_and_stack_forms() {
        let vm = parse_port_forward("8080:80", Some("web")).unwrap();
        assert_eq!(vm.bind, "127.0.0.1");
        assert_eq!(vm.host_port, 8080);
        assert_eq!(vm.vm, "web");
        assert_eq!(vm.guest_port, 80);

        let bound = parse_port_forward("127.0.0.1:8080:80", Some("web")).unwrap();
        assert_eq!(bound.host_port, 8080);

        let stack = parse_port_forward("5432:db:5432", None).unwrap();
        assert_eq!(stack.vm, "db");
        assert_eq!(stack.guest_port, 5432);

        let full = parse_port_forward("127.0.0.1:2222:router:22", None).unwrap();
        assert_eq!(full.vm, "router");
        assert_eq!(full.host_port, 2222);
    }

    #[test]
    fn rejects_unknown_role_and_port_collision() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  ports: ["8080:web:80"]
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      roles: [builder]
      ports: ["8080:80"]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("unsupported VM role")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("collides")));
    }

    #[test]
    fn rejects_unknown_volume_and_reserved_tag() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  volumes:
    vzctl: /tmp
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      mounts:
        - { source: missing, target: /srv/app }
        - { source: missing, target: /srv/app }
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("reserved tag")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("unknown volume")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("duplicate mount target")));
    }

    #[test]
    fn policy_via_requires_router_attached_to_networks() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns: { enabled: true, hostResolver: true, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
  images: { ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 } }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared, natEgress: true }
    lan: { cidr: 10.90.0.0/24, mode: shared, natEgress: false }
  routes: []
  policies:
    - name: lan-to-internet
      network: lan
      via: web
      forward: deny-all
      allow:
        - { to: internet, proto: tcp, ports: [443] }
  vms:
    router:
      from: ubuntu-base
      dataDisk: 4G
      roles: [router]
      networks:
        - { name: dmz, ip: 10.80.0.2 }
        - { name: lan, ip: 10.90.0.2 }
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue.path.contains("policies[0].via")
                && issue.message.contains("does not have role router")
        }));
    }

    #[test]
    fn policy_via_pins_valid_router() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns: { enabled: true, hostResolver: true, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
  images: { ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 } }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared, natEgress: true }
    lan: { cidr: 10.90.0.0/24, mode: shared, natEgress: false }
  routes: []
  policies:
    - name: lan-to-internet
      network: lan
      via: router
      forward: deny-all
      allow:
        - { to: internet, proto: tcp, ports: [443] }
  vms:
    router:
      from: ubuntu-base
      dataDisk: 4G
      roles: [router]
      networks:
        - { name: dmz, ip: 10.80.0.2 }
        - { name: lan, ip: 10.90.0.2 }
"#;
        let environment = validate_source(source).unwrap();
        assert_eq!(environment.spec.policies[0].via.as_deref(), Some("router"));
    }

    #[test]
    fn accepts_volumes_and_mounts_without_path_base() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  volumes:
    web-src: ../app
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      mounts:
        - { source: web-src, target: /srv/app }
"#;
        let environment = validate_source(source).unwrap();
        assert_eq!(environment.spec.volumes["web-src"], "../app");
        assert_eq!(environment.spec.vms["web"].mounts.len(), 1);
        assert_eq!(environment.spec.vms["web"].mounts[0].target, "/srv/app");
    }

    #[test]
    fn negative_fixture_reports_referential_paths_and_cycle() {
        let issues = validate_source(include_str!(
            "../tests/fixtures/validate/invalid-references.yaml"
        ))
        .unwrap_err();
        let paths = issues
            .iter()
            .map(|issue| issue.path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("$.spec.routes[0].via"));
        assert!(paths.contains("$.spec.vms.web.networks[0].ip"));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("dependency cycle")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("collides with DHCP")));
    }

    #[test]
    fn accepts_ingress_oidc_and_certs() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    bind: "127.0.0.1"
    hostAliases: true
    routes:
      - { host: web.svc.edge-dmz.vz.test, to: "web:80", requires: [oidc] }
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    mode: embedded
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    clients: auto
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      requires: [oidc]
"#;
        let environment = validate_source(source).unwrap();
        assert!(environment.spec.ingress.as_ref().unwrap().enabled);
        assert_eq!(
            environment.spec.oidc.as_ref().unwrap().issuer,
            "https://auth.svc.edge-dmz.vz.test"
        );
    }

    #[test]
    fn rejects_localhost_oidc_issuer_and_unknown_ingress_vm() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  ingress:
    enabled: true
    routes:
      - { host: web.svc.edge-dmz.vz.test, to: "missing:80" }
      - { host: auth.localhost, to: "oidc:5556" }
  oidc:
    enabled: true
    issuer: https://auth.localhost
    listen: "127.0.0.1:5556"
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("unknown VM")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("*.localhost")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("never use *.localhost")));
    }

    #[test]
    fn accepts_partial_oidc_uplink_override() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    bind: "127.0.0.1"
    hostAliases: true
    routes:
      - { host: web.svc.edge-dmz.vz.test, to: "web:80", requires: [oidc] }
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    mode: embedded
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    clients: auto
    uplink:
      type: oidc
      clientID: edge-dmz-dex
      clientSecretFile: host
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      requires: [oidc]
"#;
        let environment = validate_source(source).unwrap();
        let uplink = environment
            .spec
            .oidc
            .as_ref()
            .unwrap()
            .uplink
            .as_ref()
            .unwrap();
        assert_eq!(uplink.client_id.as_deref(), Some("edge-dmz-dex"));
        assert_eq!(uplink.client_secret_file.as_deref(), Some("host"));
        assert!(uplink.issuer.is_none());
    }

    #[test]
    fn rejects_oidc_uplink_http_issuer() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  ingress:
    enabled: true
    routes:
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    uplink:
      type: oidc
      issuer: http://login.example.com
      clientID: vzctl-dex
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues.iter().any(|issue| issue
            .message
            .contains("oidc.uplink.issuer must be an https://")));
    }

    #[test]
    fn rejects_inline_oidc_client_secret() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  oidc:
    enabled: true
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    uplink:
      type: oidc
      issuer: https://login.example.com
      clientID: vzctl-dex
      clientSecret: inline-not-allowed
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue.message.contains("clientSecret")
                || issue.message.contains("unknown field")
                || issue.kind == "schema"
        }));
    }

    #[test]
    fn accepts_oidc_simple_with_users_and_custom_claims() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    bind: "127.0.0.1"
    hostAliases: true
    routes:
      - { host: web.svc.edge-dmz.vz.test, to: "web:80", requires: [oidc] }
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    mode: oidc-simple
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    clients: auto
    users:
      - username: alice
        email: alice@dev.local
        role: admin
        teams: [platform]
      - username: bob
        email: bob@dev.local
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
      requires: [oidc]
"#;
        let environment = validate_source(source).unwrap();
        let oidc = environment.spec.oidc.as_ref().unwrap();
        assert_eq!(oidc.mode, OidcMode::OidcSimple);
        assert_eq!(oidc.users.len(), 2);
        assert_eq!(oidc.users[0].username, "alice");
        assert_eq!(
            oidc.users[0].claims.get("role").and_then(|v| v.as_str()),
            Some("admin")
        );
    }

    #[test]
    fn rejects_oidc_simple_without_users_or_with_uplink() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    bind: "127.0.0.1"
    routes:
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    mode: oidc-simple
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    clients: auto
    uplink:
      type: oidc
      issuer: https://login.example
      clientID: x
      clientSecretFile: host
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("users must list")));
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("uplink is not allowed")));
    }

    #[test]
    fn rejects_users_on_embedded_oidc() {
        let source = r#"
apiVersion: hypernetwork/v1
kind: Environment
metadata: { name: edge-dmz }
spec:
  project: edge-dmz
  domain: edge-dmz.vz.test
  dns:
    enabled: true
    hostResolver: true
    hostListen: "127.0.0.1:15353"
    forward: { enabled: true, upstream: system }
  images:
    ubuntu-base: { from: ubuntu-latest, role: base, tag: v1 }
  networks:
    dmz: { cidr: 10.80.0.0/24, mode: shared }
  routes: []
  policies: []
  certs: { enabled: true, onRotate: reinject }
  ingress:
    enabled: true
    bind: "127.0.0.1"
    routes:
      - { host: auth.svc.edge-dmz.vz.test, to: "oidc:5556" }
  oidc:
    enabled: true
    mode: embedded
    issuer: https://auth.svc.edge-dmz.vz.test
    listen: "127.0.0.1:5556"
    clients: auto
    users:
      - username: alice
        email: alice@dev.local
  vms:
    web:
      from: ubuntu-base
      dataDisk: 4G
      networks: [{ name: dmz, ip: 10.80.0.10 }]
"#;
        let issues = validate_source(source).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("only valid with mode: oidc-simple")));
    }

    #[test]
    fn json_pointer_is_rendered_as_readable_json_path() {
        assert_eq!(
            pointer_to_json_path("/spec/vms/web/networks/0/ip"),
            "$.spec.vms.web.networks[0].ip"
        );
        assert_eq!(
            pointer_to_json_path("/spec/vms/weird.name/from"),
            "$.spec.vms[\"weird.name\"].from"
        );
    }

    #[test]
    fn resilience_defaults_safe_and_validates_probe_url() {
        let source = include_str!("../tests/fixtures/validate/valid-full.yaml");
        let environment = validate_source(source).unwrap();
        assert!(environment.spec.resilience.network.egress_probe.enabled);
        assert_eq!(
            environment.spec.resilience.network.egress_probe.url,
            "https://captive.apple.com/"
        );
        assert!(
            !environment
                .spec
                .resilience
                .network
                .restart_vms_on_stuck_egress
        );

        let invalid = source.replacen(
            "\n  vms:",
            "\n  resilience:\n    network:\n      egressProbe:\n        enabled: true\n        url: https://user:secret@example.com/\n      restartVMsOnStuckEgress: false\n  vms:",
            1,
        );
        let issues = validate_source(&invalid).unwrap_err();
        assert!(issues
            .iter()
            .any(|issue| issue.path.contains("resilience.network.egressProbe.url")));
    }

    #[test]
    fn scaffold_environment_matches_ui_defaults() {
        let environment = scaffold_environment("My Lab", "10.80.0.0/24").unwrap();
        assert_eq!(environment.metadata.name, "my-lab");
        assert_eq!(environment.spec.project, "my-lab");
        assert_eq!(environment.spec.domain, "my-lab.vz.test");
        assert_eq!(environment.spec.networks["lan"].cidr, "10.80.0.0/24");
        assert!(environment.spec.images.contains_key("ubuntu-base"));
        assert!(environment.spec.vms.is_empty());
        validate_source(&serialize_environment_yaml(&environment)).unwrap();
    }

    #[test]
    fn resolve_image_key_accepts_config_key_or_pull_alias() {
        let environment = scaffold_environment("lab", "10.80.0.0/24").unwrap();
        assert_eq!(
            resolve_image_key(&environment, "ubuntu-base").unwrap(),
            "ubuntu-base"
        );
        assert_eq!(
            resolve_image_key(&environment, "ubuntu-latest").unwrap(),
            "ubuntu-base"
        );
        assert!(resolve_image_key(&environment, "missing").is_err());
    }

    #[test]
    fn next_free_ip_skips_used_addresses() {
        let mut environment = scaffold_environment("lab", "10.80.0.0/24").unwrap();
        add_vm(
            &mut environment,
            "web",
            &AddVmOptions {
                from_image: "ubuntu-base".to_string(),
                network: Some("lan".to_string()),
                ip: Some("10.80.0.10".to_string()),
                data_disk: "4G".to_string(),
                cpus: None,
                memory: None,
                roles: Vec::new(),
                cloud_init: None,
            },
        )
        .unwrap();
        assert_eq!(next_free_ip(&environment, "lan").unwrap(), "10.80.0.11");
    }

    #[test]
    fn serialize_round_trips_through_validator() {
        let environment = scaffold_environment("lab", "10.80.0.0/24").unwrap();
        let yaml = serialize_environment_yaml(&environment);
        assert!(yaml.contains("apiVersion: hypernetwork/v1"));
        assert!(yaml.contains("resilience:"));
        validate_source(&yaml).unwrap();
    }
}
