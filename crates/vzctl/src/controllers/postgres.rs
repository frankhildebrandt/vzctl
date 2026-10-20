//! PostgreSQL provisioning through psql in the explicitly selected provider VM.
//! Credentials travel in guest env/stdin, never CLI arguments or diagnostic output.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) struct PostgresController;

/// Explicit host:port only: libpq connection strings, URL credentials and Unix
/// sockets are intentionally not accepted as provider endpoints.
pub(crate) fn endpoint(value: &str) -> ControllerResult<(String, u16)> {
    let (host, port) = value.rsplit_once(':').ok_or_else(|| {
        ControllerError::new("InvalidProvider", "PostgreSQL endpoint requires host:port")
    })?;
    let host = if host.starts_with('[') && host.ends_with(']') {
        let ip = &host[1..host.len() - 1];
        if ip.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(ControllerError::new(
                "InvalidProvider",
                "invalid IPv6 endpoint",
            ));
        }
        ip
    } else {
        if host.is_empty()
            || host.len() > 253
            || (host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && host.parse::<std::net::Ipv4Addr>().is_err())
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || !label.as_bytes()[0].is_ascii_alphanumeric()
                    || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(ControllerError::new(
                "InvalidProvider",
                "invalid PostgreSQL host",
            ));
        }
        host
    };
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| ControllerError::new("InvalidProvider", "invalid PostgreSQL port"))?;
    Ok((host.into(), port))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn database_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !["postgres", "template0", "template1"].contains(&value.to_ascii_lowercase().as_str())
        && !value.to_ascii_lowercase().starts_with("pg_")
}

fn provision_sql(owner: &str, role: &str, db: &str, password: &str) -> String {
    let owner = literal(owner);
    let role_literal = literal(role);
    let db_literal = literal(db);
    let password = literal(password);
    // Dollar-quote delimiter must not be terminable by a project name or secret.
    let mut delimiter = "$vzctl$".to_string();
    while [&owner, &password]
        .iter()
        .any(|value| value.contains(&delimiter))
    {
        delimiter.insert(delimiter.len() - 1, 'x');
    }
    format!(
        r#"\set ON_ERROR_STOP on
SET standard_conforming_strings = on;
SET statement_timeout = '20s';
SET lock_timeout = '10s';
SELECT pg_advisory_lock(hashtextextended({owner}, 0));
CREATE TEMP TABLE vzctl_changed (value boolean NOT NULL);
INSERT INTO vzctl_changed VALUES (false);
DO {delimiter}
DECLARE r record; d record;
BEGIN
  SELECT oid, rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls,
         shobj_description(oid, 'pg_authid') AS owner INTO r
    FROM pg_roles WHERE rolname = {role_literal};
  IF FOUND AND r.owner IS DISTINCT FROM {owner} THEN
    RAISE EXCEPTION 'VZCTL_OWNERSHIP_CONFLICT';
  END IF;
  IF r.oid IS NOT NULL AND (r.rolsuper OR r.rolcreatedb OR r.rolcreaterole
      OR r.rolreplication OR r.rolbypassrls
      OR EXISTS (SELECT 1 FROM pg_auth_members WHERE member = r.oid)) THEN
    RAISE EXCEPTION 'VZCTL_PRIVILEGE_DRIFT';
  END IF;
  SELECT datdba, shobj_description(oid, 'pg_database') AS owner INTO d
    FROM pg_database WHERE datname = {db_literal};
  -- An existing database is ours only if owned by our verified dedicated role.
  -- NULL comment is recoverable after CREATE DATABASE succeeded before COMMENT.
  IF FOUND AND (r.oid IS NULL OR d.datdba <> r.oid
      OR (d.owner IS NOT NULL AND d.owner <> {owner})) THEN
    RAISE EXCEPTION 'VZCTL_OWNERSHIP_CONFLICT';
  END IF;
  IF r.oid IS NULL THEN
    CREATE ROLE "{role}" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD {password};
    COMMENT ON ROLE "{role}" IS {owner};
    UPDATE vzctl_changed SET value = true;
  END IF;
END
{delimiter};
SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = {db_literal}) AS create_database \gset
\if :create_database
CREATE DATABASE "{db}" OWNER "{role}";
UPDATE vzctl_changed SET value = true;
\endif
DO {delimiter}
BEGIN
  IF (SELECT shobj_description(oid, 'pg_database') FROM pg_database
      WHERE datname = {db_literal}) IS DISTINCT FROM {owner} THEN
    COMMENT ON DATABASE "{db}" IS {owner};
    UPDATE vzctl_changed SET value = true;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_database,
      LATERAL aclexplode(COALESCE(datacl, acldefault('d', datdba))) acl
      WHERE datname = {db_literal} AND acl.grantee = 0) THEN
    REVOKE ALL ON DATABASE "{db}" FROM PUBLIC;
    UPDATE vzctl_changed SET value = true;
  END IF;
END
{delimiter};
SELECT 'VZCTL_RESULT:' || json_build_object('changed', value)::text FROM vzctl_changed;
"#
    )
}

fn safe_failure(stderr: &str) -> ControllerError {
    if stderr.contains("VZCTL_OWNERSHIP_CONFLICT") {
        ControllerError::new(
            "OwnershipConflict",
            "database or application role belongs to another owner; choose another database or repair ownership explicitly",
        )
    } else if stderr.contains("VZCTL_PRIVILEGE_DRIFT") {
        ControllerError::new(
            "PrivilegeDrift",
            "managed database role has elevated privileges or memberships; repair those privileges explicitly before retrying",
        )
    } else {
        ControllerError::new(
            "ProviderUnavailable",
            "PostgreSQL provisioning failed; check provider connectivity, psql availability and administrator permissions (remote diagnostics redacted)",
        )
    }
}

impl ServiceController for PostgresController {
    fn reconcile(&self, ctx: &ControllerContext<'_>) -> ControllerResult<ReconcileResult> {
        if ctx.provider.kind != ServiceKind::Postgres || ctx.claim.kind != ServiceKind::Postgres {
            return Err(ControllerError::new(
                "InvalidProvider",
                "PostgreSQL controller requires PostgreSQL provider and claim",
            ));
        }
        let db = ctx
            .claim
            .database
            .as_deref()
            .filter(|v| database_name(v))
            .ok_or_else(|| {
                ControllerError::new(
                    "InvalidClaim",
                    "database must be a non-reserved ASCII identifier of at most 63 characters",
                )
            })?;
        let (host, port) = endpoint(ctx.provider.endpoint.as_deref().unwrap_or(""))?;
        let (advertised_host, advertised_port) = endpoint(ctx.advertised_endpoint)?;
        let admin = ctx
            .provider
            .admin_user
            .as_deref()
            .filter(|v| !v.is_empty() && v.len() <= 63 && !v.chars().any(char::is_control))
            .ok_or_else(|| {
                ControllerError::new("InvalidProvider", "PostgreSQL provider requires adminUser")
            })?;
        let admin_password =
            ctx.read_secret(ctx.provider.password_file.as_deref().ok_or_else(|| {
                ControllerError::new(
                    "InvalidProvider",
                    "PostgreSQL provider requires passwordFile",
                )
            })?)?;
        let owner = ctx.owner();
        let identity = format!("{:x}", Sha256::digest(owner.as_bytes()));
        let role = format!("vz_{}", &identity[..24]);
        let password = ctx
            .secrets
            .get_or_create(&format!("postgres/{identity}/password"))?;
        if password.is_empty() || password.contains('\0') || admin_password.contains('\0') {
            return Err(ControllerError::new(
                "InvalidSecret",
                "PostgreSQL credential is empty or contains NUL",
            ));
        }
        let vm = ctx.provider_vm()?;
        let argv = [
            "psql",
            "-X",
            "-qAt",
            "--no-password",
            "-h",
            &host,
            "-p",
            &port.to_string(),
            "-U",
            admin,
            "-d",
            "postgres",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let mut env = BTreeMap::from([
            ("PGPASSWORD".into(), admin_password),
            ("PGCONNECT_TIMEOUT".into(), "10".into()),
            ("PGAPPNAME".into(), "vzctl-service-controller".into()),
            (
                "PGOPTIONS".into(),
                "-c statement_timeout=20000 -c lock_timeout=10000".into(),
            ),
        ]);
        let result = ctx
            .guest
            .run(
                &vm,
                &argv,
                &env,
                provision_sql(&owner, &role, db, &password).as_bytes(),
                60_000,
            )
            .map_err(|_| safe_failure(""))?;
        if result.exit_code != 0 || result.truncated {
            return Err(safe_failure(&result.stderr));
        }
        let result: serde_json::Value = result
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("VZCTL_RESULT:"))
            .and_then(|line| serde_json::from_str(line).ok())
            .ok_or_else(|| {
                ControllerError::new(
                    "InvalidProviderResponse",
                    "PostgreSQL returned no complete provisioning result",
                )
            })?;
        let changed = result["changed"].as_bool().ok_or_else(|| {
            ControllerError::new(
                "InvalidProviderResponse",
                "PostgreSQL returned an invalid provisioning result",
            )
        })?;
        // A stored password is never silently rotated. Verify that the existing
        // account really authenticates before publishing a usable binding.
        env.insert("PGPASSWORD".into(), password.clone());
        let argv = [
            "psql",
            "-X",
            "-qAt",
            "--no-password",
            "-h",
            &host,
            "-p",
            &port.to_string(),
            "-U",
            &role,
            "-d",
            db,
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let ready = ctx.guest.run(&vm, &argv, &env, b"SELECT 'VZCTL_READY';\n", 20_000)
            .map_err(|_| ControllerError::new("CredentialCheckFailed", "application database authentication failed; verify provider authentication and the retained credential before explicit repair"))?;
        if ready.exit_code != 0 || ready.truncated || ready.stdout.trim() != "VZCTL_READY" {
            return Err(ControllerError::new(
                "CredentialCheckFailed",
                "application database authentication failed; credentials were not rotated",
            ));
        }
        Ok(ReconcileResult {
            changed,
            binding: Binding {
                public: BTreeMap::from([
                    ("host".into(), advertised_host),
                    ("port".into(), advertised_port.to_string()),
                    ("database".into(), db.into()),
                    ("user".into(), role.clone()),
                ]),
                secrets: BTreeMap::from([("password".into(), password)]),
            },
            resource: serde_json::json!({"kind":"postgres", "database":db, "user":role, "owner":owner}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Secrets;
    impl SecretStore for Secrets {
        fn get_or_create(&self, _key: &str) -> ControllerResult<String> {
            Ok("retained-app-secret".into())
        }
    }
    struct Guest {
        results: RefCell<VecDeque<ExecOutput>>,
        calls: RefCell<Vec<(Vec<String>, BTreeMap<String, String>, String)>>,
    }
    impl GuestExec for Guest {
        fn run(
            &self,
            vm: &str,
            argv: &[String],
            env: &BTreeMap<String, String>,
            stdin: &[u8],
            timeout_ms: u64,
        ) -> ControllerResult<ExecOutput> {
            assert_eq!(vm, "demo/database");
            assert!(timeout_ms <= 60_000);
            self.calls.borrow_mut().push((
                argv.to_vec(),
                env.clone(),
                String::from_utf8(stdin.to_vec()).unwrap(),
            ));
            self.results.borrow_mut().pop_front().ok_or_else(|| {
                ControllerError::new(
                    "ProviderUnavailable",
                    "guest failed with retained-app-secret",
                )
            })
        }
    }
    fn output(stdout: &str, stderr: &str, exit_code: i32) -> ExecOutput {
        ExecOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
            truncated: false,
        }
    }
    fn fixture(run: impl FnOnce(&ControllerContext<'_>, &Guest)) {
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vzctl-pg-controller-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("admin"), "administrator-secret\n").unwrap();
        let provider = serde_json::from_value(serde_json::json!({
            "kind":"postgres","via":"database","endpoint":"127.0.0.1:5432",
            "adminUser":"postgres","passwordFile":"admin"
        }))
        .unwrap();
        let claim = serde_json::from_value(serde_json::json!({"kind":"postgres","database":"app"}))
            .unwrap();
        let guest = Guest {
            results: RefCell::new(VecDeque::new()),
            calls: RefCell::new(Vec::new()),
        };
        let ctx = ControllerContext {
            project: "demo",
            claim_name: "app-db",
            provider: &provider,
            claim: &claim,
            advertised_endpoint: "db.lan.demo.vz.test:5432",
            config_dir: &path,
            guest: &guest,
            secrets: &Secrets,
        };
        run(&ctx, &guest);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn rejects_connection_string_injection_and_reserved_database_names() {
        for value in [
            "db:5432 user=root",
            "localhost/evil:5432",
            "postgres://user:pass@db:5432",
            "-h:5432",
            "db:0",
            "::1:5432",
        ] {
            assert!(endpoint(value).is_err(), "{value}");
        }
        assert_eq!(endpoint("[::1]:5432").unwrap(), ("::1".into(), 5432));
        assert_eq!(endpoint("db.lan.demo.vz.test:5432").unwrap().1, 5432);
        for value in [
            "postgres",
            "template1",
            "pg_catalog",
            "app\";DROP DATABASE x",
            "",
        ] {
            assert!(!database_name(value));
        }
    }

    #[test]
    fn secrets_cannot_escape_sql_and_raw_errors_are_redacted() {
        let sql = provision_sql(
            "vzctl:project/$vzctl$'claim",
            "vz_safe",
            "app",
            "secret'$vzctl$\\x",
        );
        assert!(sql.contains("DO $vzctlx$"));
        assert!(sql.contains("PASSWORD 'secret''$vzctl$\\x'"));
        let error = safe_failure("FATAL credential super-secret-value");
        assert!(!format!("{error:?}").contains("super-secret-value"));
        assert_eq!(
            safe_failure("ERROR VZCTL_OWNERSHIP_CONFLICT password=bad").code,
            "OwnershipConflict"
        );
        assert_eq!(
            safe_failure("ERROR VZCTL_PRIVILEGE_DRIFT password=bad").code,
            "PrivilegeDrift"
        );
    }

    #[test]
    fn ready_binding_uses_advertised_address_and_never_exposes_secrets_publicly() {
        fixture(|ctx, guest| {
            for changed in [true, false] {
                guest.results.borrow_mut().extend([
                    output(&format!("VZCTL_RESULT:{{\"changed\":{changed}}}\n"), "", 0),
                    output("VZCTL_READY\n", "", 0),
                ]);
                let result = PostgresController.reconcile(ctx).unwrap();
                assert_eq!(result.changed, changed);
                assert_eq!(result.binding.public["host"], "db.lan.demo.vz.test");
                assert_eq!(result.binding.secrets["password"], "retained-app-secret");
                assert!(!serde_json::to_string(&result.resource)
                    .unwrap()
                    .contains("secret"));
                assert!(!result.binding.public.values().any(|v| v.contains("secret")));
            }
            let calls = guest.calls.borrow();
            assert_eq!(calls.len(), 4);
            assert_eq!(
                calls[0].2, calls[2].2,
                "reconcile uses the same retained credential"
            );
            for (argv, _, _) in calls.iter() {
                assert!(
                    !argv.iter().any(|v| v.contains("secret")),
                    "credentials must not appear in argv"
                );
            }
            assert_eq!(calls[0].1["PGPASSWORD"], "administrator-secret");
            assert_eq!(calls[1].1["PGPASSWORD"], "retained-app-secret");
            assert_eq!(calls[1].2, "SELECT 'VZCTL_READY';\n");
        });
    }

    #[test]
    fn conflict_does_not_publish_binding_or_attempt_application_login() {
        fixture(|ctx, guest| {
            guest.results.borrow_mut().push_back(output(
                "",
                "VZCTL_OWNERSHIP_CONFLICT administrator-secret",
                1,
            ));
            let result = PostgresController.reconcile(ctx);
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("conflict must fail"),
            };
            assert_eq!(error.code, "OwnershipConflict");
            assert!(!format!("{error:?}").contains("administrator-secret"));
            assert_eq!(guest.calls.borrow().len(), 1);
        });
    }

    #[test]
    fn authentication_failure_does_not_rotate_or_publish_credentials() {
        fixture(|ctx, guest| {
            guest.results.borrow_mut().extend([
                output("VZCTL_RESULT:{\"changed\":false}", "", 0),
                output("", "FATAL retained-app-secret", 1),
            ]);
            let error = match PostgresController.reconcile(ctx) {
                Err(error) => error,
                Ok(_) => panic!("login must fail"),
            };
            assert_eq!(error.code, "CredentialCheckFailed");
            assert!(!format!("{error:?}").contains("retained-app-secret"));
            assert_eq!(guest.calls.borrow().len(), 2);
        });
    }

    #[test]
    fn transport_errors_are_redacted() {
        fixture(|ctx, _| {
            let error = match PostgresController.reconcile(ctx) {
                Err(error) => error,
                Ok(_) => panic!("transport must fail"),
            };
            assert_eq!(error.code, "ProviderUnavailable");
            assert!(!format!("{error:?}").contains("retained-app-secret"));
        });
    }
}
