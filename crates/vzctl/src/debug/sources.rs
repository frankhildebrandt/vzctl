use crate::config::{
    config_path, validate_path, DataSourcesConfig, Environment, SqlDataSourceTypeConfig,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceType {
    Postgres,
    Mysql,
    Nats,
}

impl SourceType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Nats => "nats",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "postgres" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            "nats" => Ok(Self::Nats),
            other => Err(format!("unsupported data source type: {other}")),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedStack {
    pub _config_file: PathBuf,
    pub config_dir: PathBuf,
    pub environment: Environment,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedVia {
    pub vm_key: String,
    pub vm_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct SqlResolvedSource {
    pub source_type: SourceType,
    pub name: String,
    pub via: ResolvedVia,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct NatsResolvedSource {
    pub name: String,
    pub via: ResolvedVia,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub token: Option<String>,
    pub creds: Option<String>,
}

pub(crate) fn load_stack(config: &Path) -> Result<LoadedStack, String> {
    let config_file = config_path(config);
    let config_dir = config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let environment = validate_path(&config_file)
        .map_err(|issues| format_validation_issues(&issues))?;
    Ok(LoadedStack {
        _config_file: config_file,
        config_dir,
        environment,
    })
}

pub(crate) fn list_sources_json(environment: &Environment) -> Value {
    json!({
        "dataSources": build_type_list(&environment.spec.observability.data_sources),
    })
}

fn build_type_list(data_sources: &DataSourcesConfig) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(postgres) = &data_sources.postgres {
        out.insert(
            "postgres".into(),
            type_summary_generic(&postgres.default, &postgres.via, postgres.entries.keys()),
        );
    }
    if let Some(mysql) = &data_sources.mysql {
        out.insert(
            "mysql".into(),
            type_summary_generic(&mysql.default, &mysql.via, mysql.entries.keys()),
        );
    }
    if let Some(nats) = &data_sources.nats {
        out.insert(
            "nats".into(),
            type_summary_generic(&nats.default, &nats.via, nats.entries.keys()),
        );
    }
    Value::Object(out)
}

fn type_summary_generic(
    default: &str,
    via: &str,
    entries: impl Iterator<Item = impl AsRef<str>>,
) -> Value {
    json!({
        "default": default,
        "via": via,
        "entries": entries.map(|name| name.as_ref().to_string()).collect::<Vec<_>>(),
    })
}

pub(crate) fn resolve_sql_source(
    stack: &LoadedStack,
    source_type: SourceType,
    source_name: Option<&str>,
) -> Result<SqlResolvedSource, String> {
    let type_config = sql_type_config(&stack.environment, source_type)?;
    let name = source_name
        .map(str::to_string)
        .unwrap_or_else(|| type_config.default.clone());
    let entry = type_config
        .entries
        .get(&name)
        .ok_or_else(|| format!("unknown {} source {:?}", source_type.as_str(), name))?;
    let via_key = entry
        .via
        .as_deref()
        .unwrap_or(type_config.via.as_str())
        .to_string();
    let via = resolve_via(&stack.environment, &via_key)?;
    let password = read_optional_secret(&stack.config_dir, entry.password_file.as_deref())?;
    Ok(SqlResolvedSource {
        source_type,
        name,
        via,
        host: entry.host.clone(),
        port: entry.port,
        database: entry.database.clone(),
        user: entry.user.clone(),
        password,
    })
}

pub(crate) fn resolve_nats_source(
    stack: &LoadedStack,
    source_name: Option<&str>,
) -> Result<NatsResolvedSource, String> {
    let type_config = stack
        .environment
        .spec
        .observability
        .data_sources
        .nats
        .as_ref()
        .ok_or_else(|| "observability.dataSources.nats is not configured".to_string())?;
    let name = source_name
        .map(str::to_string)
        .unwrap_or_else(|| type_config.default.clone());
    let entry = type_config
        .entries
        .get(&name)
        .ok_or_else(|| format!("unknown nats source {:?}", name))?;
    let via_key = entry
        .via
        .as_deref()
        .unwrap_or(type_config.via.as_str())
        .to_string();
    let via = resolve_via(&stack.environment, &via_key)?;
    Ok(NatsResolvedSource {
        name,
        via,
        host: entry.host.clone(),
        port: entry.port,
        user: entry.user.clone(),
        password: read_optional_secret(&stack.config_dir, entry.password_file.as_deref())?,
        token: read_optional_secret(&stack.config_dir, entry.token_file.as_deref())?,
        creds: read_optional_secret(&stack.config_dir, entry.creds_file.as_deref())?,
    })
}

fn sql_type_config<'a>(
    environment: &'a Environment,
    source_type: SourceType,
) -> Result<&'a SqlDataSourceTypeConfig, String> {
    let data_sources = &environment.spec.observability.data_sources;
    match source_type {
        SourceType::Postgres => data_sources
            .postgres
            .as_ref()
            .ok_or_else(|| "observability.dataSources.postgres is not configured".to_string()),
        SourceType::Mysql => data_sources
            .mysql
            .as_ref()
            .ok_or_else(|| "observability.dataSources.mysql is not configured".to_string()),
        SourceType::Nats => Err("use resolve_nats_source for NATS".into()),
    }
}

fn resolve_via(environment: &Environment, vm_key: &str) -> Result<ResolvedVia, String> {
    if !environment.spec.vms.contains_key(vm_key) {
        return Err(format!("via references unknown VM {:?}", vm_key));
    }
    Ok(ResolvedVia {
        vm_id: crate::runtime_vm_id(&environment.spec.project, vm_key),
        vm_key: vm_key.to_string(),
    })
}

pub(crate) fn read_optional_secret(
    config_dir: &Path,
    path: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let resolved = resolve_secret_path(config_dir, path)?;
    let content = fs::read_to_string(&resolved).map_err(|error| {
        format!(
            "cannot read secret file {}: {error}",
            resolved.display()
        )
    })?;
    Ok(Some(content.trim_end_matches('\n').to_string()))
}

fn resolve_secret_path(config_dir: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        config_dir.join(candidate)
    };
    Ok(resolved)
}

pub(crate) fn sql_env(source: &SqlResolvedSource) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(password) = &source.password {
        match source.source_type {
            SourceType::Postgres => {
                env.insert("PGPASSWORD".into(), password.clone());
            }
            SourceType::Mysql => {
                env.insert("MYSQL_PWD".into(), password.clone());
            }
            SourceType::Nats => {}
        }
    }
    env
}

pub(crate) fn source_json(source_type: SourceType, name: &str, via: &ResolvedVia) -> Value {
    json!({
        "type": source_type.as_str(),
        "name": name,
        "via": via.vm_key,
        "vmId": via.vm_id,
    })
}

fn format_validation_issues(issues: &[crate::config::ValidationIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect::<Vec<_>>()
        .join("; ")
}

// Fix type_summary overload for nats - I used wrong function name above
impl SourceType {
    pub(crate) fn tool_binary(self) -> &'static str {
        match self {
            Self::Postgres => "psql",
            Self::Mysql => "mysql",
            Self::Nats => "nats",
        }
    }
}

pub(crate) fn configured_types(data_sources: &DataSourcesConfig) -> Vec<SourceType> {
    let mut out = Vec::new();
    if data_sources.postgres.is_some() {
        out.push(SourceType::Postgres);
    }
    if data_sources.mysql.is_some() {
        out.push(SourceType::Mysql);
    }
    if data_sources.nats.is_some() {
        out.push(SourceType::Nats);
    }
    out
}

pub(crate) fn list_sources_filtered(
    environment: &Environment,
    filter: Option<SourceType>,
) -> Value {
    let data_sources = &environment.spec.observability.data_sources;
    match filter {
        Some(SourceType::Postgres) => json!({
            "postgres": data_sources.postgres.as_ref().map(|cfg| type_summary_generic(&cfg.default, &cfg.via, cfg.entries.keys())),
        }),
        Some(SourceType::Mysql) => json!({
            "mysql": data_sources.mysql.as_ref().map(|cfg| type_summary_generic(&cfg.default, &cfg.via, cfg.entries.keys())),
        }),
        Some(SourceType::Nats) => json!({
            "nats": data_sources.nats.as_ref().map(|cfg| type_summary_generic(&cfg.default, &cfg.via, cfg.entries.keys())),
        }),
        None => list_sources_json(environment),
    }
}
