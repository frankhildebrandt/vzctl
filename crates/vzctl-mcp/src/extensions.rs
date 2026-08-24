//! Debug data source MCP helpers.

use crate::VzctlCli;

pub(crate) fn push_source(args: &mut Vec<String>, source: &Option<String>) {
    if let Some(name) = source {
        args.push("--source".into());
        args.push(name.clone());
    }
}

pub(crate) fn push_timeout_ms(args: &mut Vec<String>, timeout_ms: &Option<u64>) {
    if let Some(value) = timeout_ms {
        args.push("--timeout-ms".into());
        args.push(value.to_string());
    }
}

pub(crate) fn run_sql_query(
    cli: &VzctlCli,
    engine: &str,
    stack_path: &str,
    sql: &str,
    source: &Option<String>,
    max_rows: Option<u32>,
    timeout_ms: Option<u64>,
) -> Result<String, String> {
    let mut args = vec![
        "debug".to_string(),
        engine.to_string(),
        "query".to_string(),
        "-C".to_string(),
        stack_path.to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    push_source(&mut args, source);
    if let Some(rows) = max_rows {
        args.push("--max-rows".into());
        args.push(rows.to_string());
    }
    push_timeout_ms(&mut args, &timeout_ms);
    args.push("--".into());
    args.push(sql.to_string());
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(crate::util::json_pretty(&cli.run_json(&argv)?))
}
