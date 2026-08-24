use std::borrow::Cow;

const BLOCKED_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "DROP", "CREATE", "ALTER", "TRUNCATE", "GRANT", "REVOKE", "COPY",
    "CALL", "EXEC", "EXECUTE", "MERGE", "REPLACE", "LOAD", "IMPORT", "EXPORT", "SET", "USE",
    "LOCK", "UNLOCK", "RENAME", "HANDLER", "PREPARE", "DEALLOCATE",
];

/// Returns Ok(normalized SQL) or an error message for non read-only statements.
pub(crate) fn validate_read_only_sql(sql: &str) -> Result<String, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL must not be empty".into());
    }
    if trimmed.contains('\\') {
        return Err("psql meta-commands are not allowed".into());
    }
    if contains_statement_separator(trimmed) {
        return Err("only a single SQL statement is allowed".into());
    }
    let upper = trimmed.to_ascii_uppercase();
    for keyword in BLOCKED_KEYWORDS {
        if contains_keyword(&upper, keyword) {
            return Err(format!("statement contains blocked keyword {keyword}"));
        }
    }
    let first = first_keyword(&upper).ok_or_else(|| "could not parse SQL statement".to_string())?;
    if !matches!(
        first.as_ref(),
        "SELECT" | "EXPLAIN" | "SHOW" | "DESCRIBE" | "DESC" | "WITH"
    ) {
        return Err(format!("only read-only statements are allowed (got {first})"));
    }
    Ok(trimmed.to_string())
}

fn first_keyword(sql: &str) -> Option<Cow<'_, str>> {
    let mut words = sql.split_whitespace();
    let first = words.next()?;
    if first == "WITH" {
        for word in words {
            let upper = word.to_ascii_uppercase();
            if upper == "SELECT" || upper == "EXPLAIN" {
                return Some(Cow::Owned(upper));
            }
        }
        return Some(Cow::Borrowed("WITH"));
    }
    Some(Cow::Owned(first.to_string()))
}

fn contains_keyword(haystack: &str, keyword: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word == keyword)
}

fn contains_statement_separator(sql: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                if in_single && chars.peek() == Some(&'\'') {
                    chars.next();
                } else {
                    in_single = !in_single;
                }
            }
            '"' if !in_single => in_double = !in_double,
            ';' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
}

pub(crate) fn build_psql_argv(host: &str, port: u16, user: &str, database: &str, sql: &str) -> Vec<String> {
    vec![
        "psql".into(),
        "-h".into(),
        host.into(),
        "-p".into(),
        port.to_string(),
        "-U".into(),
        user.into(),
        "-d".into(),
        database.into(),
        "-P".into(),
        "pager=off".into(),
        "-t".into(),
        "-A".into(),
        "-c".into(),
        sql.into(),
    ]
}

pub(crate) fn build_mysql_argv(
    host: &str,
    port: u16,
    user: &str,
    database: &str,
    sql: &str,
) -> Vec<String> {
    vec![
        "mysql".into(),
        "-h".into(),
        host.into(),
        "-P".into(),
        port.to_string(),
        "-u".into(),
        user.into(),
        "-D".into(),
        database.into(),
        "-N".into(),
        "-B".into(),
        "-e".into(),
        sql.into(),
    ]
}

pub(crate) fn parse_psql_tsv(stdout: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let columns: Vec<String> = lines[0].split('|').map(str::to_string).collect();
    let rows = if lines.len() == 1 && columns.len() == 1 && !stdout.contains('|') {
        vec![columns.clone()]
    } else {
        lines.iter().map(|line| line.split('|').map(str::to_string).collect()).collect()
    };
    (columns, rows)
}

pub(crate) fn parse_mysql_tsv(stdout: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut lines = stdout.lines().filter(|line| !line.is_empty()).peekable();
    let Some(first) = lines.next() else {
        return (Vec::new(), Vec::new());
    };
    let columns: Vec<String> = first.split('\t').map(str::to_string).collect();
    let rows = lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect();
    (columns, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_select() {
        assert!(validate_read_only_sql("select 1").is_ok());
    }

    #[test]
    fn rejects_insert() {
        assert!(validate_read_only_sql("INSERT INTO t VALUES (1)").is_err());
    }

    #[test]
    fn rejects_multi_statement() {
        assert!(validate_read_only_sql("SELECT 1; SELECT 2").is_err());
    }
}
