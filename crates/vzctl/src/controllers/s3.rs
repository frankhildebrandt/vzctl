//! MinIO-specific S3 IAM adapter. Generic S3 has no portable IAM provisioning API.
//! Requires Python 3 and mc in the provider guest, never on the macOS host.
use super::*;
use base64::Engine;
use sha2::{Digest, Sha256};

pub(crate) struct S3Controller;

// MinIO's service-account creation path enforces 8..40 bytes, even though
// root credentials accept longer values. Keep the existing persisted random
// seed untouched and derive a versioned 240-bit credential for its exact legacy
// format. Already valid secrets remain byte-identical; unknown formats fail closed.
fn service_account_secret(stored: &str) -> ControllerResult<String> {
    if (8..=40).contains(&stored.len()) && !stored.contains('\0') {
        return Ok(stored.into());
    }
    if stored.len() == 64
        && stored
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        let mut hash = Sha256::new();
        hash.update(b"vzctl.s3.service-account-secret.v1\0");
        hash.update(stored.as_bytes());
        return Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&hash.finalize()[..30]));
    }
    Err(ControllerError::new(
        "InvalidSecret",
        "S3 stored credential format is unsupported; no credential was changed",
    ))
}

pub(crate) fn valid_endpoint(value: &str) -> bool {
    let (scheme, authority) = match value.split_once("://") {
        Some(parts) => parts,
        None => return false,
    };
    if !matches!(scheme, "http" | "https") {
        return false;
    }
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    let with_port = if authority.starts_with('[') && authority.ends_with(']') {
        format!("{authority}:{}", if scheme == "https" { 443 } else { 80 })
    } else if authority.contains(':') {
        authority.into()
    } else {
        format!("{authority}:{}", if scheme == "https" { 443 } else { 80 })
    };
    super::postgres::endpoint(&with_port).is_ok()
}

pub(crate) fn valid_bucket(value: &str) -> bool {
    (3..=63).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
        && !value.starts_with("xn--")
        && !value.starts_with("sthree-")
        && !value.starts_with("amzn-s3-demo-")
        && !["-s3alias", "--ol-s3", "--x-s3", "--table-s3"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
}

// The implementation uses the documented mc v10 config and svcacct add/info/edit
// contracts (github.com/minio/mc, cmd/config-v10.go, admin-user-svcacct-{add,info,set}.go).
// mc requires --secret-key for explicit service-account creation: the credential
// is briefly visible to privileged processes INSIDE the trusted provider guest.
// Host argv, logs and returned errors never contain it. All mc output is captured.
// A private temporary configuration is removed on success or ordinary failure.
const RECONCILE_SCRIPT: &str = r#"
import json, os, selectors, signal, subprocess, sys, tempfile, time, urllib.parse

class ReconcileError(Exception):
    pass

def fail(code):
    raise ReconcileError(code)

def run_mc(argv, environment, timeout):
    # Bound both pipes while reading. stderr counts towards the shared budget but
    # is never retained. A separate group lets cleanup reach inherited pipe owners.
    process = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment,
        start_new_session=True)
    deadline = time.monotonic() + timeout
    output, total = bytearray(), 0
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            selector.register(process.stderr, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail("ProviderTimeout")
                for key, _ in selector.select(min(0.1, remaining)):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    total += len(chunk)
                    if total > 1024 * 1024:
                        fail("InvalidProviderResponse")
                    if key.fileobj is process.stdout:
                        output.extend(chunk)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                fail("ProviderTimeout")
            try:
                process.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                fail("ProviderTimeout")
        return subprocess.CompletedProcess([], process.returncode, bytes(output), b"")
    finally:
        # Also clean up descendants after a parent exit. They may retain pipes or
        # ignore TERM. The group belongs exclusively to this mc invocation.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=0.2)
        except subprocess.TimeoutExpired:
            pass
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=1)
            process.stdout.close()
            process.stderr.close()

def decode_stream(raw):
    decoder, values = json.JSONDecoder(), []
    while raw.strip():
        value, end = decoder.raw_decode(raw.lstrip())
        values.append(value)
        raw = raw.lstrip()[end:]
    return values

def reconcile(data):
    os.umask(0o077)
    deadline = time.monotonic() + 110
    with tempfile.TemporaryDirectory(prefix="vzctl-s3-") as directory:
        def alias(access, secret):
            return {"url": data["endpoint"], "accessKey": access, "secretKey": secret,
                    "api": "S3v4", "path": "on"}
        with open(os.path.join(directory, "config.json"), "w") as file:
            json.dump({"version":"10", "aliases":{
                "admin": alias(data["adminAccess"], data["adminSecret"]),
                "application": alias(data["accessKey"], data["secretKey"])}}, file)
        environment = {k:v for k,v in os.environ.items() if not k.startswith("MC_")}
        environment["MC_NO_COLOR"] = "1"
        def mc(*args, operation, failure="ProviderUnavailable"):
            try:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail("ProviderTimeout")
                try:
                    result = run_mc(["mc", "--config-dir", directory, "--json", *args],
                        environment, min(20, remaining))
                except OSError:
                    fail("ProviderUnavailable")
                if result.returncode:
                    fail(failure)
                try:
                    values = decode_stream(result.stdout.decode("utf-8"))
                except (ValueError, UnicodeError):
                    fail("InvalidProviderResponse")
                if any(not isinstance(v,dict) or v.get("status") == "error" for v in values):
                    fail(failure)
                return values
            except ReconcileError as error:
                # Only literal call-site tokens; no provider strings or argv.
                error.operation = operation
                raise

        bucket, owner, access = data["bucket"], data["owner"], data["accessKey"]
        target = "admin/" + bucket
        # Successful enumeration is required. Authentication/transport failures
        # never become "absent" and thus never trigger a create or ownership edit.
        buckets = mc("ls", "admin", operation="bucket.list")
        if any("key" not in item for item in buckets):
            fail("InvalidProviderResponse")
        exists = any(item["key"].rstrip("/") == bucket for item in buckets)
        accounts = mc("admin", "user", "svcacct", "ls", "admin", data["adminAccess"],
            operation="account.list")
        if any("accessKey" not in item for item in accounts):
            fail("InvalidProviderResponse")
        account_exists = any(item["accessKey"] == access for item in accounts)
        account = None
        if account_exists:
            info = mc("admin", "user", "svcacct", "info", "admin", access, operation="account.info")
            if len(info) != 1:
                fail("InvalidProviderResponse")
            account = info[0]
            if account.get("description") != owner or account.get("parentUser") != data["adminAccess"]:
                fail("OwnershipConflict")
        if exists:
            tags = mc("tag", "list", target, operation="bucket.tags.read", failure="OwnershipUncertain")
            if len(tags) != 1 or not isinstance(tags[0].get("tagset"), dict):
                fail("OwnershipUncertain")
            tag_owner = tags[0]["tagset"].get("vzctl-owner")
            if tag_owner is None:
                fail("OwnershipUncertain")
            if tag_owner != owner:
                fail("OwnershipConflict")
        changed = False
        if not exists:
            # No --ignore-existing. A concurrent/foreign creation is never adopted.
            mc("mb", "--region", "us-east-1", target, operation="bucket.create")
            # Crash between mb and tagging deliberately fails closed on next run.
            mc("tag", "set", target, urllib.parse.urlencode({"vzctl-owner": owner}),
                operation="bucket.tags.write", failure="OwnershipUncertain")
            changed = True
        policy = {"Version":"2012-10-17", "Statement":[
            {"Effect":"Allow", "Action":["s3:GetBucketLocation","s3:ListBucket","s3:ListBucketMultipartUploads"],
             "Resource":["arn:aws:s3:::" + bucket]},
            {"Effect":"Allow", "Action":["s3:GetObject","s3:PutObject","s3:DeleteObject",
                "s3:AbortMultipartUpload","s3:ListMultipartUploadParts"],
             "Resource":["arn:aws:s3:::" + bucket + "/*"]}]}
        policy_path = os.path.join(directory, "policy.json")
        with open(policy_path, "w") as file:
            json.dump(policy, file)
        if not account_exists:
            mc("admin", "user", "svcacct", "add", "admin", data["adminAccess"],
                "--access-key", access, "--secret-key", data["secretKey"],
                "--description", owner, "--name", "vzctl",
                "--policy", policy_path, operation="account.create")
            changed = True
        else:
            # No implicit re-enable or password reset: either can invalidate an
            # explicit administrator action. Report it for an explicit repair.
            if account.get("accountStatus") not in ("on", "enabled"):
                fail("CredentialCheckFailed")
            existing_policy = account.get("policy")
            if isinstance(existing_policy, str):
                try:
                    existing_policy = json.loads(existing_policy)
                except ValueError:
                    fail("InvalidProviderResponse")
            def normalized(value):
                if isinstance(value, dict):
                    return {k:normalized([v] if k in ("Action", "Resource", "Statement")
                        and not isinstance(v, list) else v)
                        for k,v in value.items() if k != "Sid"}
                if isinstance(value, list):
                    return sorted((normalized(v) for v in value),
                        key=lambda v: json.dumps(v, sort_keys=True))
                return value
            if account.get("impliedPolicy") or normalized(existing_policy) != normalized(policy):
                mc("admin", "user", "svcacct", "edit", "admin", access, "--policy", policy_path,
                    operation="account.policy.update")
                changed = True
        # Verify retained credentials. Do not rotate them if the service rejects
        # authentication, and do not publish a Ready binding in that case.
        mc("stat", "application/" + bucket, operation="credentials.check", failure="CredentialCheckFailed")
        return {"changed": changed}

try:
    data = json.load(sys.stdin)
    result = reconcile(data)
    print(json.dumps(result, separators=(",",":")))
except ReconcileError as error:
    response = {"error": str(error)}
    if hasattr(error, "operation"):
        response["operation"] = error.operation
    print(json.dumps(response, separators=(",",":")))
    sys.exit(1)
except Exception:
    # Never interpolate exceptions: subprocess errors can contain secret argv,
    # and mc JSON may contain access/secret keys or credential-bearing URLs.
    print('{"error":"ProviderUnavailable"}')
    sys.exit(1)
"#;

fn failure(code: &str) -> ControllerError {
    match code {
        "OwnershipConflict" => ControllerError::new(
            "OwnershipConflict",
            "bucket or service account belongs to another owner; select another resource or repair ownership explicitly",
        ),
        "OwnershipUncertain" => ControllerError::new(
            "OwnershipUncertain",
            "existing bucket has no verified ownership tag; check the provider audit log and data before explicitly setting vzctl-owner to this claim owner, or choose a new bucket; no resource was adopted",
        ),
        "ProviderTimeout" => ControllerError::new(
            "ProviderTimeout",
            "MinIO provider operation exceeded its deadline",
        ),
        "InvalidProviderResponse" => ControllerError::new(
            "InvalidProviderResponse",
            "MinIO returned an invalid or oversized response",
        ),
        "CredentialCheckFailed" => ControllerError::new(
            "CredentialCheckFailed",
            "application S3 access failed or the service account is disabled; retained credentials were not rotated",
        ),
        _ => ControllerError::new(
            "ProviderUnavailable",
            "MinIO provisioning failed; check connectivity, administrator permissions, Python 3 and mc in the provider VM (remote diagnostics redacted)",
        ),
    }
}

fn failure_at(code: &str, operation: Option<&str>) -> ControllerError {
    let mut error = failure(code);
    // Guest JSON is untrusted. Admit only our fixed operation names, never argv,
    // resource identifiers, URLs or remote error text into agent diagnostics.
    if let Some(operation) = operation.filter(|operation| {
        matches!(
            *operation,
            "bucket.list"
                | "account.list"
                | "account.info"
                | "bucket.tags.read"
                | "bucket.create"
                | "bucket.tags.write"
                | "account.create"
                | "account.policy.update"
                | "credentials.check"
        )
    }) {
        error
            .message
            .push_str(&format!(" (operation: {operation})"));
    }
    error
}

impl ServiceController for S3Controller {
    fn reconcile(&self, ctx: &ControllerContext<'_>) -> ControllerResult<ReconcileResult> {
        if ctx.provider.kind != ServiceKind::S3
            || ctx.claim.kind != ServiceKind::S3
            || ctx.provider.implementation.as_deref() != Some("minio")
        {
            return Err(ControllerError::new(
                "UnsupportedProvider",
                "S3 IAM provisioning requires implementation: minio",
            ));
        }
        let endpoint = ctx.provider.endpoint.as_deref().filter(|v| valid_endpoint(v))
            .ok_or_else(|| ControllerError::new("InvalidProvider", "S3 endpoint must be an HTTP(S) origin without credentials, path, query or fragment"))?;
        if !valid_endpoint(ctx.advertised_endpoint) {
            return Err(ControllerError::new(
                "InvalidProvider",
                "S3 advertised endpoint must be an HTTP(S) origin",
            ));
        }
        let bucket = ctx.claim.bucket.as_deref().filter(|v| valid_bucket(v))
            .ok_or_else(|| ControllerError::new("InvalidClaim", "S3 bucket requires 3–63 lowercase ASCII letters, digits or hyphens, without reserved S3 prefixes or suffixes"))?;
        let admin_access =
            ctx.read_secret(ctx.provider.access_key_file.as_deref().ok_or_else(|| {
                ControllerError::new("InvalidProvider", "S3 provider requires accessKeyFile")
            })?)?;
        let admin_secret =
            ctx.read_secret(ctx.provider.secret_key_file.as_deref().ok_or_else(|| {
                ControllerError::new("InvalidProvider", "S3 provider requires secretKeyFile")
            })?)?;
        let owner = ctx.owner();
        if owner.len() > 256 {
            return Err(ControllerError::new(
                "InvalidClaim",
                "claim ownership identity exceeds the S3 tag limit",
            ));
        }
        let identity = format!("{:x}", Sha256::digest(owner.as_bytes()));
        // Persist both credentials before the first remote mutation. Hashing the
        // random access seed gives MinIO's conventional 20-character access key.
        let access_seed = ctx
            .secrets
            .get_or_create(&format!("s3/{identity}/access"))?;
        let access = format!("{:x}", Sha256::digest(access_seed.as_bytes()))[..20].to_uppercase();
        let stored_secret = ctx
            .secrets
            .get_or_create(&format!("s3/{identity}/secret"))?;
        let secret = service_account_secret(&stored_secret)?;
        if secret.len() < 8
            || [&secret, &admin_access, &admin_secret]
                .iter()
                .any(|v| v.contains('\0'))
        {
            return Err(ControllerError::new(
                "InvalidSecret",
                "S3 credentials are invalid",
            ));
        }
        let input = serde_json::to_vec(&serde_json::json!({
            "endpoint":endpoint, "bucket":bucket, "owner":owner,
            "adminAccess":admin_access, "adminSecret":admin_secret,
            "accessKey":access, "secretKey":secret,
        }))
        .map_err(|_| failure("InvalidProviderResponse"))?;
        let result = ctx
            .guest
            .run(
                &ctx.provider_vm()?,
                &["python3".into(), "-c".into(), RECONCILE_SCRIPT.into()],
                &BTreeMap::new(),
                &input,
                120_000,
            )
            .map_err(|_| failure("ProviderUnavailable"))?;
        if result.truncated {
            return Err(failure("InvalidProviderResponse"));
        }
        let response: serde_json::Value =
            serde_json::from_str(&result.stdout).map_err(|_| failure("InvalidProviderResponse"))?;
        if result.exit_code != 0 || response.get("error").is_some() {
            return Err(failure_at(
                response["error"].as_str().unwrap_or("ProviderUnavailable"),
                response["operation"].as_str(),
            ));
        }
        let changed = response["changed"]
            .as_bool()
            .ok_or_else(|| failure("InvalidProviderResponse"))?;
        Ok(ReconcileResult {
            changed,
            binding: Binding {
                public: BTreeMap::from([
                    ("endpoint".into(), ctx.advertised_endpoint.into()),
                    ("bucket".into(), bucket.into()),
                    ("region".into(), "us-east-1".into()),
                ]),
                secrets: BTreeMap::from([
                    ("accessKey".into(), access),
                    ("secretKey".into(), secret),
                ]),
            },
            resource: serde_json::json!({"kind":"s3", "bucket":bucket, "owner":owner, "implementation":"minio"}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn persisted_seed_respects_minio_account_limit_without_rotation() {
        use std::cell::RefCell;
        struct MinioBoundary {
            secret: RefCell<Option<String>>,
        }
        impl GuestExec for MinioBoundary {
            fn run(
                &self,
                vm: &str,
                _argv: &[String],
                _env: &BTreeMap<String, String>,
                stdin: &[u8],
                _timeout_ms: u64,
            ) -> ControllerResult<ExecOutput> {
                assert_eq!(vm, "demo/objects");
                let input: serde_json::Value = serde_json::from_slice(stdin).unwrap();
                let candidate = input["secretKey"].as_str().unwrap();
                // Pinned MinIO NewServiceAccount -> CreateNewCredentialsWithMetadata
                // enforces 8..40 bytes, unlike its generic root-credential check.
                let mut current = self.secret.borrow_mut();
                let response = if !(8..=40).contains(&candidate.len()) {
                    serde_json::json!({"error":"ProviderUnavailable"})
                } else if current.as_ref().is_some_and(|old| old != candidate) {
                    serde_json::json!({"error":"CredentialCheckFailed"})
                } else {
                    let changed = current.is_none();
                    *current = Some(candidate.into());
                    serde_json::json!({"changed":changed})
                };
                Ok(ExecOutput {
                    exit_code: i32::from(response.get("error").is_some()),
                    stdout: response.to_string(),
                    stderr: String::new(),
                    truncated: false,
                })
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "vzctl-s3-format-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("admin-access"), "administrator").unwrap();
        std::fs::write(directory.join("admin-secret"), "administrator-secret").unwrap();
        let store = storage::FileSecretStore::new(directory.join("secrets")).unwrap();
        let identity = format!("{:x}", Sha256::digest(b"vzctl:demo/files"));
        let key = format!("s3/{identity}/secret");
        let seed = store.get_or_create(&key).unwrap();
        assert_eq!(seed.len(), 64);
        let guest = MinioBoundary {
            secret: RefCell::new(None),
        };
        let provider = serde_json::from_value(
            serde_json::json!({"kind":"s3","implementation":"minio","via":"objects",
            "endpoint":"http://127.0.0.1:9000","accessKeyFile":"admin-access",
            "secretKeyFile":"admin-secret"}),
        )
        .unwrap();
        let claim =
            serde_json::from_value(serde_json::json!({"kind":"s3","bucket":"app-files"})).unwrap();
        let context = ControllerContext {
            project: "demo",
            claim_name: "files",
            provider: &provider,
            advertised_endpoint: "http://objects:9000",
            claim: &claim,
            config_dir: &directory,
            guest: &guest,
            secrets: &store,
        };
        let first = S3Controller.reconcile(&context);
        if let Err(error) = &first {
            assert_eq!(error.code, "ProviderUnavailable");
        }
        assert!(
            first.is_ok(),
            "persisted seed must provision within MinIO's 40-byte boundary"
        );
        let first = first.unwrap();
        assert!(first.changed);
        assert_eq!(first.binding.secrets["secretKey"].len(), 40);
        assert!(guest.secret.borrow().as_ref() == Some(&first.binding.secrets["secretKey"]));
        let second = S3Controller.reconcile(&context).unwrap();
        assert!(!second.changed);
        assert!(
            first.binding.secrets == second.binding.secrets,
            "binding credentials must be stable"
        );
        assert!(
            store.get_or_create(&key).unwrap() == seed,
            "persisted seed must remain unchanged"
        );
        // Unusual pre-existing accounts are never reset to our derived candidate.
        *guest.secret.borrow_mut() = Some(seed.clone());
        let error = S3Controller
            .reconcile(&context)
            .err()
            .expect("old remote mismatch must fail");
        assert_eq!(error.code, "CredentialCheckFailed");
        assert!(
            guest.secret.borrow().as_ref() == Some(&seed),
            "remote credential must not be replaced"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn service_account_formats_preserve_valid_secrets_and_reject_unknown_values() {
        for value in ["a".repeat(8), "existing-secret".into(), "z".repeat(40)] {
            assert!(service_account_secret(&value).unwrap() == value);
        }
        for value in [
            "a".repeat(7),
            "g".repeat(64),
            "A".repeat(64),
            "z".repeat(41),
            "0".repeat(65),
            "contains\0null".into(),
        ] {
            assert_eq!(
                service_account_secret(&value).unwrap_err().code,
                "InvalidSecret"
            );
        }
        let seed = "0123456789abcdef".repeat(4);
        let first = service_account_secret(&seed).unwrap();
        assert_eq!(
            first, "E4nqp5uYcTImazEJqjHduNtkneK-MJv5HvpEdOro",
            "v1 mapping is a persisted credential contract"
        );
        assert!(first == service_account_secret(&seed).unwrap());
        assert!(first != service_account_secret(&"f".repeat(64)).unwrap());
    }

    #[test]
    fn rejects_ambiguous_endpoints_and_invalid_bucket_names() {
        for value in [
            "http://objects:9000",
            "https://objects.example.test",
            "http://[::1]:9000/",
        ] {
            assert!(valid_endpoint(value), "{value}");
        }
        for value in [
            "http://user:secret@objects:9000",
            "https://objects/a",
            "https://objects?x",
            "http://objects/#x",
            "file:///tmp/test",
            "http://objects:0",
        ] {
            assert!(!valid_endpoint(value), "{value}");
        }
        for name in [
            "UPPERCASE",
            "ab",
            "app.files",
            "bad/key",
            "-bucket",
            "bucket-",
            "xn--test",
            "app--x-s3",
        ] {
            assert!(!valid_bucket(name), "{name}");
        }
        assert!(valid_bucket("app-files"));
    }

    #[test]
    fn errors_are_allowlisted_and_never_echo_provider_output() {
        for raw in [
            "password=secret-value",
            "OwnershipConflict secret-value",
            "AccessKey=ABC SecretKey=secret-value",
        ] {
            let error = failure(raw);
            assert_eq!(error.code, "ProviderUnavailable");
            assert!(!format!("{error:?}").contains("secret-value"));
        }
        assert_eq!(failure("OwnershipUncertain").code, "OwnershipUncertain");
        for operation in [
            "bucket.list",
            "account.list",
            "account.info",
            "bucket.tags.read",
            "bucket.create",
            "bucket.tags.write",
            "account.create",
            "account.policy.update",
            "credentials.check",
        ] {
            let error = failure_at("ProviderTimeout", Some(operation));
            assert_eq!(error.code, "ProviderTimeout");
            assert!(error
                .message
                .ends_with(&format!("(operation: {operation})")));
        }
        for operation in [
            None,
            Some("account.create SECRET"),
            Some("https://secret@provider"),
            Some("secret"),
        ] {
            let error = failure_at("secret", operation);
            assert_eq!(error.code, "ProviderUnavailable");
            assert!(!error.message.contains("secret"));
            assert!(!error.message.contains("operation:"));
        }
    }

    #[test]
    fn real_mc_processes_bound_streams_deadlines_and_descendants() {
        let harness = include_str!("../../tests/s3_process_harness.py");
        let mut child = Command::new("python3")
            .args(["-c", harness])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("controller process tests require Python 3");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(RECONCILE_SCRIPT).unwrap())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "real process limits, deadlines, cleanup and redaction passed"
        );
    }

    #[test]
    fn runner_reconciles_idempotently_and_fails_closed_across_crashes() {
        // Execute the actual guest runner with a stateful fake mc transport.
        // No S3 server, guest, network or real mc process is used by this test.
        let harness = r#"
import ast, contextlib, io, json, os, subprocess, sys, types
script = json.load(sys.stdin)
data = {"endpoint":"http://objects:9000", "bucket":"app-files", "owner":"vzctl:demo/files",
        "adminAccess":"admin", "adminSecret":"ADMIN-SECRET-NEVER-PRINT",
        "accessKey":"APPACCESS", "secretKey":"APP-SECRET-NEVER-PRINT"}
state = {}
calls = []
def reset(**values):
    state.clear()
    state.update(bucket=False, owner=None, account=None, offline=False, tag_crash=False, credential_fail=False)
    state.update(values)
    calls.clear()
def fake_run(argv, environment, timeout):
    assert argv[:2] == ["mc", "--config-dir"], argv
    directory = argv[2]
    assert os.stat(directory).st_mode & 0o777 == 0o700
    assert os.stat(os.path.join(directory,"config.json")).st_mode & 0o777 == 0o600
    config = json.load(open(os.path.join(directory, "config.json")))
    assert config["aliases"]["application"]["secretKey"] == data["secretKey"]
    args = argv[4:]
    calls.append(args)
    def result(value=None, status=0):
        return types.SimpleNamespace(returncode=status,
            stdout=(json.dumps(value).encode() if value is not None else b""),
            stderr=b"ADMIN-SECRET-NEVER-PRINT APP-SECRET-NEVER-PRINT")
    if state["offline"]:
        return result({"error":"ADMIN-SECRET-NEVER-PRINT"}, 1)
    if args == ["ls", "admin"]:
        return result({"key":"app-files/","status":"success"} if state["bucket"] else None)
    if args[:4] == ["admin","user","svcacct","ls"]:
        return result({"accessKey":"APPACCESS","status":"success"} if state["account"] else None)
    if args[:4] == ["admin","user","svcacct","info"]:
        return result(state["account"])
    if args[:2] == ["tag","list"]:
        return result({"tagset": {} if state["owner"] is None else {"vzctl-owner":state["owner"]}})
    if args[0] == "mb":
        assert not state["bucket"], "must never recreate a known bucket"
        state["bucket"] = True
        return result({"status":"success"})
    if args[:2] == ["tag","set"]:
        if state["tag_crash"]:
            return result({"error":"crash ADMIN-SECRET-NEVER-PRINT"},1)
        state["owner"] = data["owner"]
        return result({"status":"success"})
    if args[:4] == ["admin","user","svcacct","add"]:
        assert state["account"] is None, "must never replace an account"
        assert args[args.index("--secret-key")+1] == data["secretKey"]
        policy = json.load(open(args[args.index("--policy")+1]))
        state["account"] = {"description":data["owner"],"parentUser":"admin",
            "accountStatus":"on","policy":policy,"impliedPolicy":False}
        return result({"accessKey":"APPACCESS","secretKey":data["secretKey"]})
    if args[:4] == ["admin","user","svcacct","edit"]:
        assert "--secret-key" not in args, "policy update must never rotate credentials"
        state["account"]["policy"] = json.load(open(args[args.index("--policy")+1]))
        state["account"]["impliedPolicy"] = False
        return result({"status":"success"})
    if args[0] == "stat":
        return result({"status":"success"}, 1 if state["credential_fail"] else 0)
    raise AssertionError(args)
tree = ast.parse(script)
def once():
    output = io.StringIO()
    old_stdin = sys.stdin
    sys.stdin = io.StringIO(json.dumps(data))
    try:
        with contextlib.redirect_stdout(output):
            try:
                scope = {}
                definitions = [node for node in tree.body if not isinstance(node, ast.Try)]
                exec(compile(ast.Module(body=definitions, type_ignores=[]), "<definitions>", "exec"), scope)
                scope["run_mc"] = fake_run
                entry = [node for node in tree.body if isinstance(node, ast.Try)]
                exec(compile(ast.Module(body=entry, type_ignores=[]), "<entry>", "exec"), scope)
            except SystemExit:
                pass
    finally:
        sys.stdin = old_stdin
    assert "SECRET-NEVER-PRINT" not in output.getvalue(), output.getvalue()
    return json.loads(output.getvalue())
def mutations():
    return [x for x in calls if x[0] == "mb" or x[:2] == ["tag","set"]
        or x[:4] in (["admin","user","svcacct","add"],["admin","user","svcacct","edit"])]
reset()
assert once() == {"changed":True}
assert len(mutations()) == 3
calls.clear()
assert once() == {"changed":False}
assert mutations() == []
policy = state["account"]["policy"]
assert policy["Statement"][0]["Resource"] == ["arn:aws:s3:::app-files"]
assert policy["Statement"][1]["Resource"] == ["arn:aws:s3:::app-files/*"]
# Provider canonicalization of scalar-vs-array must not trigger repeated edits.
policy["Statement"][0]["Resource"] = "arn:aws:s3:::app-files"
assert once() == {"changed":False}
state["account"]["policy"] = {"Statement":[{"Action":"s3:*","Resource":"*"}]}
calls.clear()
assert once() == {"changed":True}
assert len(mutations()) == 1
assert once() == {"changed":False}
reset(bucket=True, owner="somebody-else")
assert once() == {"error":"OwnershipConflict"}
assert mutations() == []
reset(account={"description":"other-owner","parentUser":"admin"})
assert once() == {"error":"OwnershipConflict"}
assert mutations() == []
reset(bucket=True)
assert once() == {"error":"OwnershipUncertain"}
assert mutations() == []
reset(tag_crash=True)
assert once() == {"error":"OwnershipUncertain", "operation":"bucket.tags.write"}
assert state["bucket"]
calls.clear()
assert once() == {"error":"OwnershipUncertain"}
assert mutations() == [], "restart after ambiguous creation must never adopt"
reset(offline=True)
assert once() == {"error":"ProviderUnavailable", "operation":"bucket.list"}
assert mutations() == []
reset()
assert once() == {"changed":True}
state["credential_fail"] = True
calls.clear()
assert once() == {"error":"CredentialCheckFailed", "operation":"credentials.check"}
assert mutations() == []
print("runner scenarios passed")
"#;
        let mut child = Command::new("python3")
            .args(["-c", harness])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("controller runner tests require Python 3");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(RECONCILE_SCRIPT).unwrap())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "runner scenarios passed"
        );
    }
}
