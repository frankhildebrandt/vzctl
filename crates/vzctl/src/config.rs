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
    #[schemars(length(min = 1))]
    pub(crate) images: BTreeMap<String, ImageConfig>,
    #[schemars(length(min = 1))]
    pub(crate) networks: BTreeMap<String, NetworkConfig>,
    pub(crate) routes: Vec<RouteConfig>,
    pub(crate) policies: Vec<PolicyConfig>,
    #[schemars(length(min = 1))]
    pub(crate) vms: BTreeMap<String, VmConfig>,
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
}

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
    validate_source(&source)
}

pub(crate) fn validate_source(source: &str) -> Result<Environment, Vec<ValidationIssue>> {
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
    let issues = validate_references(&environment);
    if issues.is_empty() {
        Ok(environment)
    } else {
        Err(issues)
    }
}

fn validate_references(environment: &Environment) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut networks = BTreeMap::new();

    validate_name_keys("$.spec.images", environment.spec.images.keys(), &mut issues);
    validate_name_keys(
        "$.spec.networks",
        environment.spec.networks.keys(),
        &mut issues,
    );
    validate_name_keys("$.spec.vms", environment.spec.vms.keys(), &mut issues);

    for (name, network) in &environment.spec.networks {
        let path = format!("{}.cidr", json_path_key("$.spec.networks", name));
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
        for (allow_index, allow) in policy.allow.iter().enumerate() {
            let allow_base = format!("{base}.allow[{allow_index}]");
            require_network(
                &allow.to,
                &format!("{allow_base}.to"),
                &networks,
                &mut issues,
            );
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
    for (vm_name, vm) in &environment.spec.vms {
        let vm_base = json_path_key("$.spec.vms", vm_name);
        if !environment.spec.images.contains_key(&vm.from) {
            issues.push(ValidationIssue::new(
                format!("{vm_base}.from"),
                format!("unknown image {:?}", vm.from),
                "semantic",
            ));
        }
        let mut attachments = BTreeSet::new();
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
    detect_dependency_cycles(&environment.spec.vms, &mut issues);
    sort_and_deduplicate(&mut issues);
    issues
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

fn config_path(path: &Path) -> PathBuf {
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
        assert_eq!(environment.spec.vms["router"].networks.len(), 2);
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
}
