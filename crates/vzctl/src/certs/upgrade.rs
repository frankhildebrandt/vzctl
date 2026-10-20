//! Explicit offline staging. Promotion, trust propagation and service reload are separate.
use super::*;
use std::os::unix::fs::DirBuilderExt;

struct Artifact {
    relative: PathBuf,
    source: Vec<u8>,
    candidate: Vec<u8>,
    private: bool,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "upgrade source file missing")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return Err("upgrade source must be a bounded regular file, never a symlink".into());
    }
    fs::read(path).map_err(|_| "cannot read upgrade source".into())
}

fn add(
    artifacts: &mut Vec<Artifact>,
    sources: &std::collections::BTreeMap<PathBuf, Vec<u8>>,
    relative: PathBuf,
    candidate: Vec<u8>,
    private: bool,
) -> Result<(), String> {
    artifacts.push(Artifact {
        source: sources
            .get(&relative)
            .ok_or("source snapshot missing artifact")?
            .clone(),
        relative,
        candidate,
        private,
    });
    Ok(())
}

pub(super) fn stage(state_dir: &Path, output: &Path) -> Result<Value, String> {
    stage_with_snapshot_hook(state_dir, output, || {})
}

pub(super) fn stage_with_snapshot_hook(
    state_dir: &Path,
    output: &Path,
    after_snapshot: impl FnOnce(),
) -> Result<Value, String> {
    let state = state_dir
        .canonicalize()
        .map_err(|_| "state directory missing")?;
    let output = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(output)
    };
    let parent = output
        .parent()
        .ok_or("upgrade output needs a parent directory")?
        .canonicalize()
        .map_err(|_| "upgrade output parent must already exist")?;
    let name = output
        .file_name()
        .ok_or("upgrade output needs a new directory name")?;
    let output = parent.join(name);
    if output.starts_with(&state) {
        return Err("upgrade candidate must be outside the active state directory".into());
    }
    if fs::symlink_metadata(&output).is_ok() {
        return Err("upgrade output already exists; never reuse or overwrite a candidate".into());
    }
    // Resolve every source path without following links inside the CA tree.
    for directory in [
        ca_dir(&state),
        root_dir(&state),
        trust_dir(&state),
        issued_dir(&state),
    ] {
        let meta = fs::symlink_metadata(&directory).map_err(|_| "CA directory missing")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("CA directories must not be symlinks".into());
        }
    }
    let mut entries = fs::read_dir(issued_dir(&state))
        .map_err(|_| "cannot list issued certificates")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cannot list issued certificates")?;
    entries.sort_by_key(|entry| entry.file_name());
    let inventory: Vec<_> = entries.iter().map(|entry| entry.file_name()).collect();
    let mut paths: Vec<PathBuf> = [
        "ca/root/ca.crt",
        "ca/root/ca.key",
        "ca/root/fingerprint",
        "ca/trust/vzctl-local.crt",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    for entry in &entries {
        let kind = entry
            .file_type()
            .map_err(|_| "cannot inspect issued certificate")?;
        if !kind.is_dir() || kind.is_symlink() {
            return Err("issued certificate inventory contains an unexpected entry".into());
        }
        for name in ["cert.pem", "key.pem", "meta.json"] {
            paths.push(
                PathBuf::from("ca/issued")
                    .join(entry.file_name())
                    .join(name),
            );
        }
    }
    // Capture each input exactly once. Candidate bytes and manifest source hashes
    // derive from this same snapshot, never from a later second read.
    let mut sources = std::collections::BTreeMap::new();
    for relative in paths {
        let path = state.join(&relative);
        if (relative.ends_with("ca.key") || relative.ends_with("key.pem"))
            && fs::symlink_metadata(&path)
                .map_err(|_| "private key missing")?
                .permissions()
                .mode()
                & 0o077
                != 0
        {
            return Err("upgrade private key must not be group/world accessible".into());
        }
        sources.insert(relative, read(&path)?);
    }
    after_snapshot();
    let text = |relative: &Path| -> Result<String, String> {
        String::from_utf8(
            sources
                .get(relative)
                .ok_or("source snapshot incomplete")?
                .clone(),
        )
        .map_err(|_| "source certificate/key is not UTF-8 PEM".into())
    };
    let old_root = text(Path::new("ca/root/ca.crt"))?;
    let root_key_pem = text(Path::new("ca/root/ca.key"))?;
    let mut params = profile::ca_pair(&old_root, &root_key_pem, false)?;
    if text(Path::new("ca/root/fingerprint"))?.trim() != fingerprint_pem(&old_root)
        || sources[Path::new("ca/trust/vzctl-local.crt")] != old_root.as_bytes()
    {
        return Err(
            "existing CA fingerprint/trust copy is inconsistent; inspect before staging".into(),
        );
    }
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.use_authority_key_identifier_extension = true;
    let root_key = KeyPair::from_pem(&root_key_pem).map_err(|_| "invalid root key")?;
    let new_root = params
        .self_signed(&root_key)
        .map_err(|_| "cannot sign root candidate")?;
    let root_pem = new_root.pem();
    profile::ca_pair(&root_pem, &root_key_pem, true)?;
    profile::same_root_identity(&old_root, &root_pem)?;
    let mut artifacts = Vec::new();
    for (relative, data, private) in [
        ("ca/root/ca.crt", root_pem.as_bytes().to_vec(), false),
        ("ca/root/ca.key", root_key_pem.as_bytes().to_vec(), true),
        (
            "ca/root/fingerprint",
            fingerprint_pem(&root_pem).into_bytes(),
            false,
        ),
        (
            "ca/trust/vzctl-local.crt",
            root_pem.as_bytes().to_vec(),
            false,
        ),
    ] {
        add(&mut artifacts, &sources, relative.into(), data, private)?;
    }
    let mut identities = vec![json!({
        "certificate": "ca/root/ca.crt",
        "key": "ca/root/ca.key",
        "public_key_sha256": digest(&root_key.public_key_der()),
        "old_fingerprint": fingerprint_pem(&old_root),
        "new_fingerprint": fingerprint_pem(&root_pem),
    })];
    for entry in entries {
        let kind = entry
            .file_type()
            .map_err(|_| "cannot inspect issued certificate")?;
        if !kind.is_dir() || kind.is_symlink() {
            return Err("issued certificate inventory contains an unexpected entry".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "invalid issued certificate name")?;
        let (cert_path, key_path, meta_path) = leaf_paths(&state, &name);
        let old_leaf = text(
            cert_path
                .strip_prefix(&state)
                .map_err(|_| "leaf outside state")?,
        )?;
        let key_pem = text(
            key_path
                .strip_prefix(&state)
                .map_err(|_| "key outside state")?,
        )?;
        let mut params = profile::leaf_pair(&old_leaf, &key_pem, &old_root, false)?;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::from_pem(&key_pem).map_err(|_| "invalid issued private key")?;
        let leaf = params
            .signed_by(&key, &new_root, &root_key)
            .map_err(|_| "cannot sign leaf candidate")?;
        let pem = leaf.pem();
        profile::leaf_pair(&pem, &key_pem, &root_pem, true)?;
        profile::same_leaf_identity(&old_leaf, &pem)?;
        let mut metadata: Value = serde_json::from_slice(
            sources
                .get(
                    meta_path
                        .strip_prefix(&state)
                        .map_err(|_| "metadata outside state")?,
                )
                .ok_or("metadata snapshot missing")?,
        )
        .map_err(|_| "invalid issued certificate metadata")?;
        if metadata["fingerprint"] != fingerprint_pem(&old_leaf) {
            return Err("issued certificate metadata fingerprint is inconsistent".into());
        }
        metadata
            .as_object_mut()
            .ok_or("issued metadata must be an object")?
            .insert("fingerprint".into(), Value::String(fingerprint_pem(&pem)));
        for (path, data, private) in [
            (&cert_path, pem.as_bytes().to_vec(), false),
            (&key_path, key_pem.as_bytes().to_vec(), true),
            (
                &meta_path,
                serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
                false,
            ),
        ] {
            let relative = path
                .strip_prefix(&state)
                .map_err(|_| "certificate outside state")?
                .to_path_buf();
            add(&mut artifacts, &sources, relative, data, private)?;
        }
        identities.push(json!({
            "certificate": cert_path.strip_prefix(&state).map_err(|e| e.to_string())?,
            "key": key_path.strip_prefix(&state).map_err(|e| e.to_string())?,
            "public_key_sha256": digest(&key.public_key_der()),
            "old_fingerprint": fingerprint_pem(&old_leaf),
            "new_fingerprint": fingerprint_pem(&pem),
        }));
    }
    // Exclusive private directory. Any I/O failure leaves an incomplete candidate
    // without manifest.json; callers must never promote such a directory.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&output)
        .map_err(|e| format!("create candidate: {e}"))?;
    let mut files = Vec::new();
    for artifact in &artifacts {
        let path = output.join(&artifact.relative);
        if let Some(parent) = path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|e| e.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&artifact.candidate)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        files.push(if artifact.private {
            json!({"path": artifact.relative, "private": true, "key_unchanged": artifact.source == artifact.candidate})
        } else {
            json!({"path": artifact.relative, "private": false,
                "source_sha256": digest(&artifact.source), "candidate_sha256": digest(&artifact.candidate)})
        });
    }
    let mut current_inventory = fs::read_dir(issued_dir(&state))
        .map_err(|_| "cannot recheck issued certificate inventory")?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cannot recheck issued certificate inventory")?;
    current_inventory.sort();
    if current_inventory != inventory {
        return Err(
            "issued certificate inventory changed while staging; candidate incomplete".into(),
        );
    }
    // Re-read every source before publishing the completion manifest. No source
    // bytes are ever changed, including when staging fails or is interrupted.
    for artifact in &artifacts {
        if read(&state.join(&artifact.relative))? != artifact.source {
            return Err(
                "source changed while staging; candidate incomplete, do not promote".into(),
            );
        }
    }
    let manifest = json!({
        "schema": "vzctl.ca-upgrade/v1", "state": "candidate", "promoted": false,
        "source_state": state, "output": output,
        "created_unix": SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs(),
        "root_subject_and_ski_preserved": true, "private_keys_unchanged": true,
        "requires_strict_chain_verification": true, "requires_trust_rollout": true,
        "requires_edge_reload": true, "identities": identities, "files": files,
    });
    let data = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    let manifest_path = output.join("manifest.json");
    if let Err(error) = atomic_write(&manifest_path, &data, 0o600) {
        let _ = fs::remove_file(&manifest_path);
        return Err(format!(
            "candidate completion failed; do not promote: {error}"
        ));
    }
    Ok(json!({
        "staged": true, "promoted": false, "manifest": output.join("manifest.json"),
        "manifest_sha256": digest(&data), "certificates": identities.len(),
        "old_fingerprint": fingerprint_pem(&old_root), "new_fingerprint": fingerprint_pem(&root_pem),
        "next": "Independently verify every candidate chain and identity, then explicitly promote with public trust rollout and Edge reload; do not use --force",
    }))
}
