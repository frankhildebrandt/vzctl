use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::BTreeMap;

use super::sources::{NatsResolvedSource, ResolvedVia};

pub(crate) struct NatsExecPlan {
    pub via: ResolvedVia,
    pub env: BTreeMap<String, String>,
    pub cmd: Vec<String>,
}

pub(crate) fn build_nats_env(source: &NatsResolvedSource) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    let mut url = format!("nats://{}", source.host);
    if source.port != 4222 {
        url = format!("nats://{}:{}", source.host, source.port);
    }
    if let (Some(user), Some(password)) = (&source.user, &source.password) {
        url = format!("nats://{user}:{password}@{}:{}", source.host, source.port);
    }
    env.insert("NATS_URL".into(), url);
    if let Some(token) = &source.token {
        env.insert("NATS_TOKEN".into(), token.clone());
    }
    Ok(env)
}

pub(crate) fn build_streams_argv() -> Vec<String> {
    vec!["nats".into(), "stream".into(), "ls".into(), "--json".into()]
}

pub(crate) fn build_sub_argv(subject: &str, count: u32, timeout_secs: u64) -> Vec<String> {
    vec![
        "nats".into(),
        "sub".into(),
        subject.into(),
        "--count".into(),
        count.to_string(),
        "--timeout".into(),
        format!("{timeout_secs}s"),
    ]
}

pub(crate) fn build_kv_get_argv(bucket: &str, key: &str) -> Vec<String> {
    vec![
        "nats".into(),
        "kv".into(),
        "get".into(),
        bucket.into(),
        key.into(),
    ]
}

pub(crate) fn build_kv_watch_argv(bucket: &str, key: Option<&str>, timeout_secs: u64) -> Vec<String> {
    let mut argv = vec!["nats".into(), "kv".into(), "watch".into(), bucket.into()];
    if let Some(key) = key {
        argv.push(key.into());
    }
    argv.push("--timeout".into());
    argv.push(format!("{timeout_secs}s"));
    argv
}

/// Wrap nats invocation when a creds file must exist inside the guest.
pub(crate) fn wrap_with_creds(
    source: &NatsResolvedSource,
    inner_argv: Vec<String>,
) -> Result<NatsExecPlan, String> {
    let creds = source
        .creds
        .as_ref()
        .ok_or_else(|| "internal error: creds wrapper without creds".to_string())?;
    let encoded = STANDARD.encode(creds.as_bytes());
    let creds_path = format!("/tmp/vzctl-debug-creds-{}", random_token());
    let inner = shell_join(&inner_argv);
    let script = format!(
        "echo {encoded} | base64 -d > {creds_path} && nats --creds {creds_path} {inner} ; rm -f {creds_path}"
    );
    let mut env = BTreeMap::new();
    env.insert(
        "NATS_URL".into(),
        format!("nats://{}:{}", source.host, source.port),
    );
    Ok(NatsExecPlan {
        via: source.via.clone(),
        env,
        cmd: vec!["sh".into(), "-c".into(), script],
    })
}

pub(crate) fn plan_nats_exec(
    source: &NatsResolvedSource,
    inner_argv: Vec<String>,
) -> Result<NatsExecPlan, String> {
    if source.creds.is_some() {
        return wrap_with_creds(source, inner_argv);
    }
    Ok(NatsExecPlan {
        via: source.via.clone(),
        env: build_nats_env(source)?,
        cmd: inner_argv,
    })
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|part| {
            if part.chars().any(|c| c.is_whitespace() || "\"'$\\".contains(c)) {
                format!("'{}'", part.replace('\'', "'\\''"))
            } else {
                part.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn random_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".into())
}
