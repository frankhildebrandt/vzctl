//! Credentials are sent on stdin and never included in command arguments or status.
use super::*;
use serde_json::{json, Value};

const WRITE_BINDING: &str = r#"import json,os,stat,sys,secrets
p=json.load(sys.stdin)
name=p['name']; owner=p['owner']; shared=p.get('containerReadable',False)
if not name or len(name)>63 or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-' for c in name): raise SystemExit(2)
flags=os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW
parent=os.open('/',flags)
try:
 for part in ['var','lib','vzctl','bindings']:
  try: os.mkdir(part,0o700,dir_fd=parent)
  except FileExistsError: pass
  child=os.open(part,flags,dir_fd=parent); os.close(parent); parent=child
 if os.fstat(parent).st_uid!=0: raise SystemExit(3)
 os.fchmod(parent,0o700)
 try: os.mkdir(name,0o700,dir_fd=parent)
 except FileExistsError: pass
 root=os.open(name,flags,dir_fd=parent)
finally: os.close(parent)
try:
 if os.fstat(root).st_uid!=0: raise SystemExit(3)
 os.fchmod(root,0o755 if shared else 0o700)
 mode=0o444 if shared else 0o600
 try: fd=os.open('binding.json',os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=root)
 except FileNotFoundError: fd=None
 if fd is not None:
  with os.fdopen(fd) as f:
   st=os.fstat(f.fileno())
   if not stat.S_ISREG(st.st_mode) or st.st_uid!=0 or st.st_nlink!=1 or st.st_size>262144: raise SystemExit(3)
   old=json.load(f)
   if old.get('owner')!=owner: raise SystemExit(4)
   os.fchmod(f.fileno(),mode)
  if old==p['binding']: print('unchanged'); raise SystemExit(0)
 tmp='.binding-'+secrets.token_hex(16)
 fd=os.open(tmp,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=root)
 try:
  with os.fdopen(fd,'w') as f:
   json.dump(p['binding'],f,sort_keys=True); f.flush(); os.fchmod(f.fileno(),mode); os.fsync(f.fileno())
  os.rename(tmp,'binding.json',src_dir_fd=root,dst_dir_fd=root); os.fsync(root)
 finally:
  try: os.unlink(tmp,dir_fd=root)
  except FileNotFoundError: pass
finally: os.close(root)
print('updated')
"#;

pub(super) fn deliver(
    environment: &crate::config::Environment,
    name: &str,
    claim: &ClaimConfig,
    binding: &Binding,
    guest: &dyn GuestExec,
) -> ControllerResult<()> {
    let owner = format!("vzctl:{}/{name}", environment.spec.project);
    let payload = json!({"owner":owner,"public":binding.public,"secrets":binding.secrets});
    let mut targets = BTreeMap::<String, bool>::new();
    for consumer in &claim.consumers {
        match consumer {
            Consumer::Vm(target) => {
                targets.entry(target.vm.clone()).or_insert(false);
            }
            Consumer::Container(target) => {
                let (vm, _) = target.container.split_once('/').ok_or_else(|| {
                    ControllerError::new("InvalidConsumer", "container must be vm/name")
                })?;
                targets.insert(vm.into(), true);
            }
            Consumer::Kubernetes(target) => {
                deliver_kubernetes(environment, &target.kubernetes, &owner, binding, guest)?
            }
        }
    }
    for (vm, shared) in targets {
        deliver_vm(environment, &vm, name, &owner, &payload, shared, guest)?;
    }
    Ok(())
}
fn deliver_vm(
    environment: &crate::config::Environment,
    vm: &str,
    name: &str,
    owner: &str,
    payload: &Value,
    container_readable: bool,
    guest: &dyn GuestExec,
) -> ControllerResult<()> {
    let input=serde_json::to_vec(&json!({"name":name,"owner":owner,"binding":payload,"containerReadable":container_readable})).map_err(|_|ControllerError::new("InvalidBinding","binding cannot be encoded"))?;
    let result = guest.run(
        &format!("{}/{vm}", environment.spec.project),
        &[
            "sudo".into(),
            "-n".into(),
            "python3".into(),
            "-c".into(),
            WRITE_BINDING.into(),
        ],
        &BTreeMap::new(),
        &input,
        15_000,
    )?;
    if result.exit_code == 4 {
        return Err(ControllerError::new(
            "OwnershipConflict",
            "guest binding is owned by another claim",
        ));
    }
    if result.exit_code != 0 || result.truncated {
        return Err(ControllerError::new(
            "BindingDeliveryFailed",
            "private guest binding could not be written",
        ));
    }
    Ok(())
}
fn deliver_kubernetes(
    environment: &crate::config::Environment,
    target: &KubernetesTarget,
    owner: &str,
    binding: &Binding,
    guest: &dyn GuestExec,
) -> ControllerResult<()> {
    let cluster = environment
        .spec
        .kubernetes
        .clusters
        .get(&target.cluster)
        .ok_or_else(|| ControllerError::new("InvalidConsumer", "unknown Kubernetes cluster"))?;
    let server = environment
        .spec
        .vms
        .get(&cluster.server)
        .and_then(|vm| vm.networks.iter().find(|n| n.name == cluster.network))
        .ok_or_else(|| {
            ControllerError::new("InvalidConsumer", "Kubernetes server network is missing")
        })?;
    let vm = format!("{}/{}", environment.spec.project, cluster.server);
    let mut command = vec![
        "sudo".into(),
        "-n".into(),
        "/usr/local/bin/k3s".into(),
        "kubectl".into(),
        "--kubeconfig=/etc/rancher/k3s/k3s.yaml".into(),
        format!("--server=https://{}:6443", server.ip),
        format!("--namespace={}", target.namespace),
        "--request-timeout=20s".into(),
    ];
    let prefix = command.len();
    command.extend([
        "get".into(),
        "secret".into(),
        target.secret.clone(),
        "--ignore-not-found".into(),
        "-o".into(),
        "json".into(),
    ]);
    let result = guest.run(&vm, &command, &BTreeMap::new(), &[], 30_000)?;
    if result.exit_code != 0 || result.truncated {
        return Err(ControllerError::new(
            "BindingDeliveryFailed",
            "Kubernetes binding lookup failed",
        ));
    }
    let previous = if result.stdout.trim().is_empty() {
        None
    } else {
        Some(serde_json::from_str::<Value>(&result.stdout).map_err(|_| {
            ControllerError::new("InvalidResponse", "invalid Kubernetes secret response")
        })?)
    };
    if previous
        .as_ref()
        .is_some_and(|p| p["metadata"]["annotations"]["services.vzctl.dev/owner"] != owner)
    {
        return Err(ControllerError::new(
            "OwnershipConflict",
            "Kubernetes secret belongs to another owner",
        ));
    }
    let mut fields = binding.public.clone();
    fields.extend(binding.secrets.clone());
    use base64::Engine;
    let fields: BTreeMap<_, _> = fields
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                base64::engine::general_purpose::STANDARD.encode(value),
            )
        })
        .collect();
    if previous
        .as_ref()
        .is_some_and(|p| p["data"] == json!(fields))
    {
        return Ok(());
    }
    let mut secret = json!({"apiVersion":"v1","kind":"Secret","type":"Opaque","metadata":{"name":target.secret,"namespace":target.namespace,
        "annotations":{"services.vzctl.dev/owner":owner},"labels":{"app.kubernetes.io/managed-by":"vzctl"}},"data":fields});
    if let Some(previous) = &previous {
        let version = previous["metadata"]["resourceVersion"]
            .as_str()
            .ok_or_else(|| {
                ControllerError::new("InvalidResponse", "secret resourceVersion is missing")
            })?;
        secret["metadata"]["resourceVersion"] = json!(version);
    }
    command.truncate(prefix);
    command.extend([
        if previous.is_some() {
            "replace".into()
        } else {
            "create".into()
        },
        "-f".into(),
        "-".into(),
        "--field-manager=vzctl-services".into(),
    ]);
    let input = serde_json::to_vec(&secret)
        .map_err(|_| ControllerError::new("InvalidBinding", "secret cannot be encoded"))?;
    let result = guest.run(&vm, &command, &BTreeMap::new(), &input, 30_000)?;
    if result.exit_code != 0 || result.truncated {
        return Err(ControllerError::new(
            "BindingDeliveryFailed",
            "Kubernetes secret write failed; credentials withheld from diagnostics",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Guest {
        outputs: RefCell<VecDeque<ExecOutput>>,
        calls: RefCell<Vec<(Vec<String>, Vec<u8>)>>,
    }
    impl GuestExec for Guest {
        fn run(
            &self,
            vm: &str,
            argv: &[String],
            env: &BTreeMap<String, String>,
            stdin: &[u8],
            _: u64,
        ) -> ControllerResult<ExecOutput> {
            assert_eq!(vm, "kube-lab/kube-server");
            assert!(env.is_empty());
            assert!(!argv.join(" ").contains("private-password"));
            self.calls
                .borrow_mut()
                .push((argv.to_vec(), stdin.to_vec()));
            Ok(self
                .outputs
                .borrow_mut()
                .pop_front()
                .expect("unexpected guest mutation"))
        }
    }
    fn guest(outputs: Vec<ExecOutput>) -> Guest {
        Guest {
            outputs: RefCell::new(outputs.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
    fn output(value: Value) -> ExecOutput {
        ExecOutput {
            exit_code: 0,
            stdout: if value.is_null() {
                String::new()
            } else {
                value.to_string()
            },
            stderr: "private-password".into(),
            truncated: false,
        }
    }
    fn fixture() -> (crate::config::Environment, KubernetesTarget, Binding) {
        (
            serde_yaml::from_str(include_str!(
                "../../tests/fixtures/validate/valid-kubernetes.yaml"
            ))
            .unwrap(),
            KubernetesTarget {
                cluster: "dev".into(),
                namespace: "app".into(),
                secret: "database".into(),
            },
            Binding {
                public: BTreeMap::from([("host".into(), "db.lan.example.test".into())]),
                secrets: BTreeMap::from([("password".into(), "private-password".into())]),
            },
        )
    }
    fn secret() -> Value {
        json!({"metadata":{"resourceVersion":"15","annotations":{"services.vzctl.dev/owner":"vzctl:kube-lab/database"}},
            "data":{"host":base64::engine::general_purpose::STANDARD.encode("db.lan.example.test"),
                "password":base64::engine::general_purpose::STANDARD.encode("private-password")}})
    }
    #[test]
    fn kubernetes_binding_is_namespace_local_and_credentials_only_use_stdin() {
        let (env, target, binding) = fixture();
        let guest = guest(vec![output(Value::Null), output(json!({}))]);
        deliver_kubernetes(&env, &target, "vzctl:kube-lab/database", &binding, &guest).unwrap();
        let calls = guest.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].0.contains(&"create".into()));
        assert!(calls[1].0.contains(&"--namespace=app".into()));
        let payload: Value = serde_json::from_slice(&calls[1].1).unwrap();
        assert_eq!(payload["metadata"]["namespace"], "app");
        assert_eq!(
            payload["metadata"]["annotations"]["services.vzctl.dev/owner"],
            "vzctl:kube-lab/database"
        );
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(payload["data"]["password"].as_str().unwrap())
                .unwrap(),
            b"private-password"
        );
    }
    #[test]
    fn unchanged_secret_is_noop_and_updates_use_resource_version() {
        let (env, target, binding) = fixture();
        let unchanged = guest(vec![output(secret())]);
        deliver_kubernetes(
            &env,
            &target,
            "vzctl:kube-lab/database",
            &binding,
            &unchanged,
        )
        .unwrap();
        assert_eq!(unchanged.calls.borrow().len(), 1);
        let mut old = secret();
        old["data"]["host"] = json!("old");
        let changed = guest(vec![output(old), output(json!({}))]);
        deliver_kubernetes(&env, &target, "vzctl:kube-lab/database", &binding, &changed).unwrap();
        let calls = changed.calls.borrow();
        assert!(calls[1].0.contains(&"replace".into()));
        let payload: Value = serde_json::from_slice(&calls[1].1).unwrap();
        assert_eq!(payload["metadata"]["resourceVersion"], "15");
    }
    #[test]
    fn foreign_secret_and_failed_lookup_never_write_or_echo_credentials() {
        let (env, target, binding) = fixture();
        let mut foreign = secret();
        foreign["metadata"]["annotations"]["services.vzctl.dev/owner"] = json!("other-owner");
        let foreign = guest(vec![output(foreign)]);
        let error =
            deliver_kubernetes(&env, &target, "vzctl:kube-lab/database", &binding, &foreign)
                .unwrap_err();
        assert_eq!(error.code, "OwnershipConflict");
        assert_eq!(foreign.calls.borrow().len(), 1);
        let mut failed = output(Value::Null);
        failed.exit_code = 1;
        let failed = guest(vec![failed]);
        let error = deliver_kubernetes(&env, &target, "vzctl:kube-lab/database", &binding, &failed)
            .unwrap_err();
        assert!(!error.to_string().contains("private-password"));
        assert_eq!(failed.calls.borrow().len(), 1);
    }
    #[test]
    fn vm_and_container_delivery_are_deduplicated_with_explicit_read_mode() {
        let (env, _, binding) = fixture();
        let claim:ClaimConfig=serde_json::from_value(json!({"kind":"postgres","database":"app","consumers":[
            {"vm":"kube-server"},{"container":"kube-server/app"},{"container":"kube-server/worker"}
        ]})).unwrap();
        let guest = guest(vec![output(json!({}))]);
        deliver(&env, "database", &claim, &binding, &guest).unwrap();
        let calls = guest.calls.borrow();
        assert_eq!(calls.len(), 1);
        let payload: Value = serde_json::from_slice(&calls[0].1).unwrap();
        assert_eq!(payload["containerReadable"], true);
        assert_eq!(payload["binding"]["owner"], "vzctl:kube-lab/database");
    }
}
