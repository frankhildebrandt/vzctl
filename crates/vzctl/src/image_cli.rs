//! Image command parsing, bake/seal orchestration and disk backends.

use crate::*;

pub(crate) fn image_command(args: impl Iterator<Item = String>) -> ExitCode {
    let args: Vec<_> = args.collect();
    let json = json_requested(&args);
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("list") => image_list_command(args.collect()),
        Some("pull") => image_pull_command(args.collect()),
        Some("bake") => image_bake_command(args.collect()),
        Some("seal") => image_seal_command(args.collect()),
        Some("rebake") => image_rebake_command(args.collect()),
        Some(command) => emit_command_failure(
            json,
            "image",
            EXIT_USAGE,
            &format!("unknown image command: {command}"),
        ),
        None => {
            eprintln!(
                "usage: vzctl image list [--format human|json] | \
                 image pull <alias> [--format human|json] | \
                 image bake <alias> --tag <tag> [--force] [--format human|json] | \
                 image seal <name|path> --tag <tag> [--force] [--format human|json] | \
                 image rebake <alias> --tag <tag> [--format human|json]"
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

pub(crate) fn image_list_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let format = match parse_format_options(args.into_iter(), "image list") {
        Ok(format) => format,
        Err(message) => {
            emit_image_list_failure(
                requested_format,
                &image::PullFailure {
                    code: EXIT_USAGE,
                    message,
                },
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match image::list(&images_dir()) {
        Ok(result) => {
            match format {
                OutputFormat::Human => print_image_list_human(&result),
                OutputFormat::Json => println!("{}", image_list_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_image_list_failure(format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

pub(crate) fn print_image_list_human(result: &image::ListResult) {
    println!("images_dir: {}", result.images_dir.display());
    if result.images.is_empty() {
        println!("(no local images)");
    } else {
        println!(
            "{:<24} {:<10} {:<8} {:<8} {}",
            "ALIAS", "STATE", "BAKED", "SEALED", "PATH"
        );
        for image in &result.images {
            let state = if image.sealed {
                "sealed"
            } else if image.baked {
                "baked"
            } else {
                "pulled"
            };
            println!(
                "{:<24} {:<10} {:<8} {:<8} {}",
                image.alias,
                state,
                if image.baked { "yes" } else { "no" },
                if image.sealed { "yes" } else { "no" },
                image.path.display()
            );
        }
    }
    println!();
    println!("catalog ({} aliases):", result.catalog.len());
    for entry in &result.catalog {
        let aliases = if entry.aliases.len() > 1 {
            format!(" ({})", entry.aliases.join(", "))
        } else {
            String::new()
        };
        println!(
            "  {}{} — {} {}",
            entry.alias, aliases, entry.distribution, entry.release
        );
    }
}

pub(crate) fn image_list_json(result: &image::ListResult) -> Value {
    let host_agent_version = guest_utils::agent_version_string().ok();
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.list",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.images.is_empty() {
                "no local images"
            } else {
                "image cache listed"
            },
            "count": result.images.len(),
            "images_dir": result.images_dir,
            "host_agent_version": host_agent_version,
        },
        "images": result.images.iter().map(|image| json!({
            "alias": image.alias,
            "canonical_alias": image.canonical_alias,
            "aliases": image.aliases,
            "distribution": image.distribution,
            "release": image.release,
            "architecture": image.architecture,
            "path": image.path,
            "format": image.format,
            "sha256": image.sha256,
            "baked": image.baked,
            "sealed": image.sealed,
            "agent_version": image.agent_version,
            "tags": image.tags.iter().map(|tag| json!({
                "tag": tag.tag,
                "path": tag.path,
                "format": tag.format,
                "sha256": tag.sha256,
                "baked": tag.baked,
                "sealed": tag.sealed,
                "agent_version": tag.agent_version,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "catalog": result.catalog.iter().map(|entry| json!({
            "alias": entry.alias,
            "aliases": entry.aliases,
            "distribution": entry.distribution,
            "release": entry.release,
        })).collect::<Vec<_>>(),
    })
}

pub(crate) fn emit_image_list_failure(format: OutputFormat, failure: &image::PullFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.list",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

pub(crate) fn image_pull_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_pull_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_image_pull_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    match image::pull(&options.alias, &images_dir()) {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => print_image_pull_human(&result),
                OutputFormat::Json => println!("{}", image_pull_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_image_pull_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

pub(crate) fn parse_image_pull_options(
    args: impl Iterator<Item = String>,
) -> Result<ImagePullOptions, image::PullFailure> {
    let mut alias = None;
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" => {
                let value = args.next().ok_or_else(|| image::PullFailure {
                    code: EXIT_USAGE,
                    message: "--format requires human or json".to_string(),
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(image::PullFailure {
                            code: EXIT_USAGE,
                            message: format!("unsupported image pull format: {value}"),
                        })
                    }
                };
            }
            "-h" | "--help" => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: "usage: vzctl image pull <alias> [--format human|json]".to_string(),
                })
            }
            _ if arg.starts_with('-') => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: format!("unknown image pull option: {arg}"),
                })
            }
            _ if alias.is_none() => alias = Some(arg),
            _ => {
                return Err(image::PullFailure {
                    code: EXIT_USAGE,
                    message: format!("unexpected image pull argument: {arg}"),
                })
            }
        }
    }
    let alias = alias.ok_or_else(|| image::PullFailure {
        code: EXIT_USAGE,
        message: "usage: vzctl image pull <alias> [--format human|json]".to_string(),
    })?;
    Ok(ImagePullOptions { alias, format })
}

pub(crate) fn print_image_pull_human(result: &image::PullResult) {
    println!(
        "Image {} {}",
        result.requested_alias,
        if result.unchanged {
            "is unchanged"
        } else {
            "pulled"
        }
    );
    println!(
        "  release: {} {} (arm64)",
        result.distribution, result.release
    );
    println!("  sha256: {}", result.normalized_digest);
    println!("  path: {}", result.image_path.display());
    println!("  sealed: {}", if result.sealed { "yes" } else { "no" });
}

pub(crate) fn image_pull_json(result: &image::PullResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.pull",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.unchanged { "image unchanged" } else { "image pulled" },
            "change": if result.unchanged { "unchanged" } else { "pulled" },
        },
        "image": {
            "alias": result.requested_alias,
            "canonical_alias": result.canonical_alias,
            "aliases": result.aliases,
            "distribution": result.distribution,
            "release": result.release,
            "architecture": "arm64",
            "path": result.image_path,
            "format": "raw",
            "sha256": result.normalized_digest,
            "sealed": result.sealed,
            "manifest": result.manifest_path,
        },
        "source": {
            "url": result.source_url,
            "format": result.source_format,
            "algorithm": result.source_algorithm,
            "digest": result.source_digest,
        },
    })
}

pub(crate) fn emit_image_pull_failure(format: OutputFormat, failure: &image::PullFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.pull",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

pub(crate) fn image_bake_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_bake_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_bake_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    match bake_image(&options) {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => {
                    println!(
                        "Image {} {}",
                        result.requested_alias,
                        if result.unchanged {
                            "bake unchanged"
                        } else {
                            "baked"
                        }
                    );
                    println!("  agent: {}", result.agent_version);
                    println!("  path: {}", result.image_path.display());
                }
                OutputFormat::Json => println!("{}", image_bake_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_bake_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

pub(crate) fn image_rebake_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_rebake_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_rebake_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    let bake = ImageBakeOptions {
        alias: options.alias.clone(),
        tag: options.tag.clone(),
        format: options.format,
        force: true,
    };
    let bake_result = match bake_image(&bake) {
        Ok(result) => result,
        Err(failure) => {
            emit_rebake_failure(options.format, &failure);
            return ExitCode::from(failure.code);
        }
    };
    let seal = ImageSealOptions {
        input: options.alias.clone(),
        tag: Some(options.tag.clone()),
        format: options.format,
        force: true,
    };
    let images = images_dir();
    let progress = (io::stderr().is_terminal() && options.format == OutputFormat::Human)
        || image::progress_env_enabled();
    let backend_kind = match builder::select_backend_kind() {
        Ok(kind) => kind,
        Err(failure) => {
            emit_rebake_failure(
                options.format,
                &SealFailure::new(failure.code, failure.message),
            );
            return ExitCode::from(failure.code);
        }
    };
    let seal_result = match backend_kind {
        builder::ImageBackendKind::Local => seal_image(&seal, &LibguestfsBackend),
        builder::ImageBackendKind::Builder => seal_image(
            &seal,
            &BuilderVmBackend {
                images_dir: images,
                progress,
            },
        ),
    };
    match seal_result {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => {
                    println!(
                        "Image {} rebaked and sealed @{}",
                        options.alias, options.tag
                    );
                    println!("  agent: {}", bake_result.agent_version);
                    println!("  path: {}", result.source_path.display());
                }
                OutputFormat::Json => println!(
                    "{}",
                    json!({
                        "apiVersion": CLI_API_VERSION,
                        "command": "image.rebake",
                        "status": "ok",
                        "exit_code": 0,
                        "summary": {
                            "message": "image rebaked and sealed",
                            "change": "rebaked",
                        },
                        "image": {
                            "alias": options.alias,
                            "tag": options.tag,
                            "path": result.source_path,
                            "agent_version": bake_result.agent_version,
                            "sealed": true,
                        },
                    })
                ),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_rebake_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

fn emit_rebake_failure(format: OutputFormat, failure: &SealFailure) {
    emit_command_failure(
        format == OutputFormat::Json,
        "image.rebake",
        failure.code,
        &failure.message,
    );
}

pub(crate) fn parse_image_rebake_options(
    args: impl Iterator<Item = String>,
) -> Result<ImageRebakeOptions, SealFailure> {
    let mut alias = None;
    let mut tag = None;
    let mut format = OutputFormat::Human;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--tag requires a value".to_string(),
                })?;
                if !image::valid_image_tag(&value) {
                    return Err(SealFailure {
                        code: EXIT_USAGE,
                        message: format!(
                            "invalid image tag {value}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*"
                        ),
                    });
                }
                tag = Some(value);
            }
            "--format" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--format requires human or json".to_string(),
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(SealFailure {
                            code: EXIT_USAGE,
                            message: format!("unsupported image rebake format: {value}"),
                        })
                    }
                };
            }
            "-h" | "--help" => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: "usage: vzctl image rebake <alias> --tag <tag> [--format human|json]"
                        .to_string(),
                })
            }
            _ if arg.starts_with('-') => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unknown image rebake option: {arg}"),
                })
            }
            _ if alias.is_none() => alias = Some(arg),
            _ => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unexpected image rebake argument: {arg}"),
                })
            }
        }
    }
    let alias = alias.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "usage: vzctl image rebake <alias> --tag <tag> [--format human|json]".to_string(),
    })?;
    let tag = tag.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "image rebake requires --tag <tag>".to_string(),
    })?;
    Ok(ImageRebakeOptions { alias, tag, format })
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ImageBakeOptions {
    pub(crate) alias: String,
    pub(crate) tag: String,
    pub(crate) format: OutputFormat,
    pub(crate) force: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ImageRebakeOptions {
    pub(crate) alias: String,
    pub(crate) tag: String,
    pub(crate) format: OutputFormat,
}

pub(crate) fn parse_image_bake_options(
    args: impl Iterator<Item = String>,
) -> Result<ImageBakeOptions, SealFailure> {
    let mut alias = None;
    let mut tag = None;
    let mut format = OutputFormat::Human;
    let mut force = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--tag requires a value".to_string(),
                })?;
                if !image::valid_image_tag(&value) {
                    return Err(SealFailure {
                        code: EXIT_USAGE,
                        message: format!(
                            "invalid image tag {value}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*"
                        ),
                    });
                }
                tag = Some(value);
            }
            "--force" => force = true,
            "--format" => {
                let value = args.next().ok_or_else(|| SealFailure {
                    code: EXIT_USAGE,
                    message: "--format requires human or json".to_string(),
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(SealFailure {
                            code: EXIT_USAGE,
                            message: format!("unsupported image bake format: {value}"),
                        })
                    }
                };
            }
            "-h" | "--help" => return Err(SealFailure {
                code: EXIT_USAGE,
                message:
                    "usage: vzctl image bake <alias> --tag <tag> [--force] [--format human|json]"
                        .to_string(),
            }),
            _ if arg.starts_with('-') => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unknown image bake option: {arg}"),
                })
            }
            _ if alias.is_none() => alias = Some(arg),
            _ => {
                return Err(SealFailure {
                    code: EXIT_USAGE,
                    message: format!("unexpected image bake argument: {arg}"),
                })
            }
        }
    }
    let alias = alias.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "usage: vzctl image bake <alias> --tag <tag> [--force] [--format human|json]"
            .to_string(),
    })?;
    let tag = tag.ok_or_else(|| SealFailure {
        code: EXIT_USAGE,
        message: "image bake requires --tag <tag>".to_string(),
    })?;
    Ok(ImageBakeOptions {
        alias,
        tag,
        format,
        force,
    })
}

pub(crate) fn bake_image(options: &ImageBakeOptions) -> Result<image::BakeResult, SealFailure> {
    let images = images_dir();
    let agent_version = guest_utils::agent_version_string()
        .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.message))?;
    if !options.force {
        if let Some(existing) =
            image::already_baked(&images, &options.alias, &options.tag, &agent_version)
                .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
        {
            return Ok(existing);
        }
    }

    let (target, manifest, _manifest_path) = image::prepare_alias_for_bake(
        &images,
        &options.alias,
        &options.tag,
        &agent_version,
        options.force,
    )
    .map_err(|error| SealFailure::new(EXIT_INVALID_INPUT, error))?;
    let canonical = manifest["canonical_alias"]
        .as_str()
        .unwrap_or(&options.alias)
        .to_string();

    let staging = guest_utils::build_agent_staging(&agent_version)
        .map_err(|error| SealFailure::new(EXIT_UNAVAILABLE, error.message))?;
    let progress = (io::stderr().is_terminal() && options.format == OutputFormat::Human)
        || image::progress_env_enabled();
    let backend_kind = builder::select_backend_kind()
        .map_err(|failure| SealFailure::new(failure.code, failure.message))?;

    match backend_kind {
        builder::ImageBackendKind::Local => {
            bake_with_virt_customize(&target, &staging)?;
        }
        builder::ImageBackendKind::Builder => {
            if progress {
                eprintln!("Baking via builder VM…");
            }
            let appliance = builder::ensure_builder_image(&images, progress)
                .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
            let runbook = builder::bake_runbook("staging");
            builder::run_builder_vm(builder::BuilderRunOptions {
                appliance: &appliance,
                target_raw: &target,
                runbook: &runbook,
                staging_dir: Some(&staging),
                timeout: builder::default_timeout(),
                progress,
            })
            .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
        }
    }

    let _ = fs::remove_dir_all(&staging);
    image::mark_aliases_baked(&images, &target, &options.tag, &agent_version)
        .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
    Ok(image::BakeResult {
        requested_alias: options.alias.clone(),
        canonical_alias: canonical,
        tag: options.tag.clone(),
        image_path: target,
        agent_version,
        unchanged: false,
    })
}

pub(crate) fn bake_with_virt_customize(target: &Path, staging: &Path) -> Result<(), SealFailure> {
    let output = Command::new("virt-customize")
        .args(["--format", "raw", "-a"])
        .arg(target)
        .arg("--mkdir")
        .arg("/usr/lib/vzctl-agent")
        .arg("--copy-in")
        .arg(format!("{}:/usr/local/sbin", staging.join("vzctl-agent").display()))
        .arg("--copy-in")
        .arg(format!(
            "{}:/etc/systemd/system",
            staging.join("vzctl-agent.service").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/etc/systemd/system",
            staging.join("vzctl-agent.path").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/usr/lib/tmpfiles.d",
            staging.join("vzctl-agent-tmpfiles.conf").display()
        ))
        .arg("--copy-in")
        .arg(format!(
            "{}:/usr/lib/vzctl-agent",
            staging.join("image-metadata.json").display()
        ))
        .arg("--run-command")
        .arg("id -u vzctl-agent >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin vzctl-agent")
        .arg("--run-command")
        .arg("chmod 0755 /usr/local/sbin/vzctl-agent && chmod 0644 /etc/systemd/system/vzctl-agent.service /etc/systemd/system/vzctl-agent.path /usr/lib/tmpfiles.d/vzctl-agent-tmpfiles.conf /usr/lib/vzctl-agent/image-metadata.json")
        .arg("--run-command")
        .arg("systemctl enable vzctl-agent.service vzctl-agent.path")
        .output()
        .map_err(|error| {
            SealFailure::new(
                EXIT_UNAVAILABLE,
                format!("virt-customize is required for local image bake: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SealFailure::new(
            EXIT_IMAGE_CUSTOMIZE_FAILED,
            format!("image bake failed: {}", command_text(&output)),
        ))
    }
}

pub(crate) fn image_bake_json(result: &image::BakeResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.bake",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.unchanged { "image bake unchanged" } else { "image baked" },
            "change": if result.unchanged { "unchanged" } else { "baked" },
        },
        "image": {
            "alias": result.requested_alias,
            "canonical_alias": result.canonical_alias,
            "tag": result.tag,
            "path": result.image_path,
            "format": "raw",
            "baked": true,
            "agent_version": result.agent_version,
        },
    })
}

pub(crate) fn emit_bake_failure(format: OutputFormat, failure: &SealFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.bake",
                "status": "fail",
                "exit_code": failure.code,
                "summary": { "message": failure.message },
            })
        );
    }
}

pub(crate) fn image_seal_command(args: Vec<String>) -> ExitCode {
    let requested_format = requested_output_format(&args);
    let options = match parse_image_seal_options(args.into_iter()) {
        Ok(options) => options,
        Err(failure) => {
            emit_seal_failure(requested_format, &failure);
            return ExitCode::from(failure.code);
        }
    };

    let images = images_dir();
    let progress = (io::stderr().is_terminal() && options.format == OutputFormat::Human)
        || image::progress_env_enabled();
    let backend_kind = match builder::select_backend_kind() {
        Ok(kind) => kind,
        Err(failure) => {
            emit_seal_failure(
                options.format,
                &SealFailure::new(failure.code, failure.message),
            );
            return ExitCode::from(failure.code);
        }
    };
    let result = match backend_kind {
        builder::ImageBackendKind::Local => seal_image(&options, &LibguestfsBackend),
        builder::ImageBackendKind::Builder => seal_image(
            &options,
            &BuilderVmBackend {
                images_dir: images,
                progress,
            },
        ),
    };
    match result {
        Ok(result) => {
            match options.format {
                OutputFormat::Human => print_image_seal_human(&result),
                OutputFormat::Json => println!("{}", image_seal_json(&result)),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            emit_seal_failure(options.format, &failure);
            ExitCode::from(failure.code)
        }
    }
}

pub(crate) fn requested_output_format(args: &[String]) -> OutputFormat {
    args.windows(2)
        .find(|pair| pair[0] == "--format" && pair[1] == "json")
        .map(|_| OutputFormat::Json)
        .unwrap_or(OutputFormat::Human)
}

pub(crate) fn parse_image_seal_options(
    args: impl Iterator<Item = String>,
) -> Result<ImageSealOptions, SealFailure> {
    let mut input = None;
    let mut tag = None;
    let mut format = OutputFormat::Human;
    let mut force = false;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => {
                let value = args
                    .next()
                    .ok_or_else(|| SealFailure::new(EXIT_USAGE, "--tag requires a value"))?;
                if !image::valid_image_tag(&value) {
                    return Err(SealFailure::new(
                        EXIT_USAGE,
                        format!(
                            "invalid image tag {value}; expected 1-64 [A-Za-z0-9][A-Za-z0-9._-]*"
                        ),
                    ));
                }
                tag = Some(value);
            }
            "--force" => force = true,
            "--format" => {
                let value = args.next().ok_or_else(|| {
                    SealFailure::new(EXIT_USAGE, "--format requires human or json")
                })?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(SealFailure::new(
                            EXIT_USAGE,
                            format!("unsupported image seal format: {value}"),
                        ))
                    }
                };
            }
            "-h" | "--help" => return Err(SealFailure::new(
                EXIT_USAGE,
                "usage: vzctl image seal <name|path> --tag <tag> [--force] [--format human|json]",
            )),
            _ if arg.starts_with('-') => {
                return Err(SealFailure::new(
                    EXIT_USAGE,
                    format!("unknown image seal option: {arg}"),
                ))
            }
            _ if input.is_none() => input = Some(arg),
            _ => {
                return Err(SealFailure::new(
                    EXIT_USAGE,
                    format!("unexpected image seal argument: {arg}"),
                ))
            }
        }
    }

    let input = input.ok_or_else(|| {
        SealFailure::new(
            EXIT_USAGE,
            "usage: vzctl image seal <name|path> --tag <tag> [--force] [--format human|json]",
        )
    })?;
    let tag = tag.ok_or_else(|| SealFailure::new(EXIT_USAGE, "image seal requires --tag <tag>"))?;
    Ok(ImageSealOptions {
        input,
        tag: Some(tag),
        format,
        force,
    })
}

pub(crate) fn seal_image(
    options: &ImageSealOptions,
    backend: &dyn ImageSealBackend,
) -> Result<ImageSealResult, SealFailure> {
    let images_dir = images_dir();
    seal_image_in_dir(options, backend, &images_dir)
}

pub(crate) fn seal_image_in_dir(
    options: &ImageSealOptions,
    backend: &dyn ImageSealBackend,
    images_dir: &Path,
) -> Result<ImageSealResult, SealFailure> {
    let tag = options.tag.clone().ok_or_else(|| {
        SealFailure::new(
            EXIT_USAGE,
            "image seal requires --tag <tag> for alias inputs",
        )
    })?;

    let source_path =
        match image::prepare_alias_for_seal(images_dir, &options.input, &tag, options.force)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
        {
            Some(path) => path,
            None => {
                // Direct path input: tag still required for marker/manifest bookkeeping
                // when the path is under the images store; otherwise treat as raw path seal.
                resolve_image_input(&options.input, images_dir)?
            }
        };
    let source_path = fs::canonicalize(&source_path).map_err(|error| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("cannot resolve image {}: {error}", source_path.display()),
        )
    })?;
    if !source_path.is_file() {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("image is not a regular file: {}", source_path.display()),
        ));
    }

    let name = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image")
        .to_string();
    let marker_path = seal_marker_path(images_dir, &source_path);
    if marker_path.exists() && !options.force {
        let mut result = read_existing_seal(&marker_path, &source_path, name)?;
        result.already_sealed = true;
        if image::tagged_seal_ready(images_dir, &options.input, &tag)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
        {
            return Ok(result);
        }
        // Repair incomplete tag metadata. Prefer the recorded digest; only hash
        // when this is a known alias that still lacks a tag entry.
        let recorded = image::existing_tag_sealed_digest(images_dir, &options.input, &tag)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        if let Some(digest) = recorded {
            image::mark_aliases_sealed(
                images_dir,
                &result.source_path,
                &result.marker_path,
                &tag,
                Some(&digest),
            )
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        } else if image::resolve_alias_pulled(images_dir, &options.input)
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
            .is_some()
        {
            image::mark_aliases_sealed(
                images_dir,
                &result.source_path,
                &result.marker_path,
                &tag,
                None,
            )
            .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
        }
        return Ok(result);
    }

    let image_format = backend.inspect_format(&source_path)?;
    if !matches!(image_format.as_str(), "raw" | "qcow" | "qcow2") {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("unsupported image format {image_format}; expected raw or qcow2"),
        ));
    }

    backend.seal_pipeline(&source_path, &image_format)?;

    let result = ImageSealResult {
        name,
        source_path,
        image_format,
        marker_path,
        already_sealed: false,
    };
    write_seal_marker_and_lock(&result)?;
    image::mark_aliases_sealed(
        images_dir,
        &result.source_path,
        &result.marker_path,
        &tag,
        None,
    )
    .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?;
    Ok(result)
}

pub(crate) fn images_dir() -> PathBuf {
    std::env::var_os("VZCTL_IMAGES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("images"))
}

pub(crate) fn resolve_vm_image_input(
    input: &str,
    images_dir: &Path,
) -> Result<PathBuf, SealFailure> {
    let direct = PathBuf::from(input);
    if direct.is_absolute() || direct.components().count() > 1 {
        return resolve_image_input(input, images_dir);
    }
    if let Some(path) = image::resolve_alias_for_vm(images_dir, input)
        .map_err(|error| SealFailure::new(EXIT_IMAGE_STATE_FAILED, error))?
    {
        return Ok(path);
    }
    let resolved = resolve_image_input(input, images_dir)?;
    if resolved
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "objects")
    {
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "image alias {input} is pulled but not sealed; run `vzctl image bake {input} --tag v1` and `vzctl image seal {input} --tag v1` first",
            ),
        ));
    }
    Ok(resolved)
}

pub(crate) fn resolve_image_input(input: &str, images_dir: &Path) -> Result<PathBuf, SealFailure> {
    let direct = PathBuf::from(input);
    if direct.exists() {
        return Ok(direct);
    }
    if direct.is_absolute() || direct.components().count() > 1 {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("image path does not exist: {}", direct.display()),
        ));
    }

    match image::resolve_alias(images_dir, input) {
        Ok(Some(path)) => return Ok(path),
        Ok(None) => {}
        Err(error) => return Err(SealFailure::new(EXIT_IMAGE_STATE_FAILED, error)),
    }

    let candidates = ["", ".raw", ".qcow", ".qcow2", ".img"]
        .into_iter()
        .map(|suffix| images_dir.join(format!("{input}{suffix}")))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "image {input} not found in {}; run `vzctl image pull {input}` first",
                images_dir.display()
            ),
        )),
        _ => Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "image name {input} is ambiguous in {}: {}",
                images_dir.display(),
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

pub(crate) fn seal_marker_path(images_dir: &Path, source_path: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let safe_stem = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let path_text = source_path.to_string_lossy();
    images_dir.join(format!(
        "{}-{:016x}.sealed.json",
        safe_stem,
        stable_path_hash(path_text.as_bytes())
    ))
}

pub(crate) fn stable_path_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub(crate) fn read_existing_seal(
    marker_path: &Path,
    source_path: &Path,
    name: String,
) -> Result<ImageSealResult, SealFailure> {
    let marker_text = fs::read_to_string(marker_path).map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("cannot read seal marker {}: {error}", marker_path.display()),
        )
    })?;
    let marker: Value = serde_json::from_str(&marker_text).map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("invalid seal marker {}: {error}", marker_path.display()),
        )
    })?;
    if marker["apiVersion"] != "vzctl.dev/image-seal/v1"
        || marker["sealed"] != true
        || marker["source_path"] != source_path.to_string_lossy().as_ref()
    {
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!("seal marker does not match {}", source_path.display()),
        ));
    }
    let image_format = marker["format"]
        .as_str()
        .filter(|format| matches!(*format, "raw" | "qcow" | "qcow2"))
        .ok_or_else(|| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!("seal marker has invalid format: {}", marker_path.display()),
            )
        })?
        .to_string();
    let permissions = fs::metadata(source_path)
        .map_err(|error| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!(
                    "cannot inspect sealed image {}: {error}",
                    source_path.display()
                ),
            )
        })?
        .permissions();
    if permissions.mode() & 0o222 != 0 {
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "seal marker exists but image is writable: {}",
                source_path.display()
            ),
        ));
    }
    Ok(ImageSealResult {
        name,
        source_path: source_path.to_path_buf(),
        image_format,
        marker_path: marker_path.to_path_buf(),
        already_sealed: true,
    })
}

pub(crate) fn write_seal_marker_and_lock(result: &ImageSealResult) -> Result<(), SealFailure> {
    fs::create_dir_all(
        result
            .marker_path
            .parent()
            .expect("seal marker always has a parent"),
    )
    .map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot create images directory for {}: {error}",
                result.marker_path.display()
            ),
        )
    })?;
    let sealed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let marker = seal_marker_json(result, sealed_at);
    let temporary_marker = result
        .marker_path
        .with_extension(format!("tmp-{}", std::process::id()));
    fs::write(
        &temporary_marker,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&marker).expect("marker JSON serializes")
        ),
    )
    .map_err(|error| {
        SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot write seal marker {}: {error}",
                temporary_marker.display()
            ),
        )
    })?;

    let original_permissions = fs::metadata(&result.source_path)
        .map_err(|error| {
            SealFailure::new(
                EXIT_IMAGE_STATE_FAILED,
                format!("cannot inspect {}: {error}", result.source_path.display()),
            )
        })?
        .permissions();
    let mut sealed_permissions = original_permissions.clone();
    sealed_permissions.set_mode(original_permissions.mode() & !0o222);
    if let Err(error) = fs::set_permissions(&result.source_path, sealed_permissions) {
        let _ = fs::remove_file(&temporary_marker);
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot make {} read-only: {error}",
                result.source_path.display()
            ),
        ));
    }
    if let Err(error) = fs::rename(&temporary_marker, &result.marker_path) {
        let _ = fs::set_permissions(&result.source_path, original_permissions);
        let _ = fs::remove_file(&temporary_marker);
        return Err(SealFailure::new(
            EXIT_IMAGE_STATE_FAILED,
            format!(
                "cannot publish seal marker {}: {error}",
                result.marker_path.display()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn seal_marker_json(result: &ImageSealResult, sealed_at: u64) -> Value {
    json!({
        "apiVersion": "vzctl.dev/image-seal/v1",
        "name": result.name,
        "source_path": result.source_path,
        "format": result.image_format,
        "sealed": true,
        "read_only": true,
        "sealed_at_unix_seconds": sealed_at,
        "cleanup": {
            "machine_id": true,
            "dbus_machine_id": true,
            "ssh_host_keys": true,
            "cloud_init": true,
            "random_seed": true,
        },
        "preserved": {
            "agent": "/usr/local/sbin/vzctl-agent",
            "systemd_unit": "/etc/systemd/system/vzctl-agent.service",
            "image_metadata": "/usr/lib/vzctl-agent/image-metadata.json",
        },
    })
}

pub(crate) fn print_image_seal_human(result: &ImageSealResult) {
    println!(
        "{} image {}",
        if result.already_sealed {
            "already sealed"
        } else {
            "sealed"
        },
        result.name
    );
    println!("  source: {}", result.source_path.display());
    println!("  format: {}", result.image_format);
    println!("  marker: {}", result.marker_path.display());
    println!("  read-only: yes");
    println!("  preserved: agent, systemd unit, image metadata");
    println!("  clone-safe: machine-id, SSH keys, cloud-init, random seed cleaned");
}

pub(crate) fn image_seal_json(result: &ImageSealResult) -> Value {
    json!({
        "apiVersion": CLI_API_VERSION,
        "command": "image.seal",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": if result.already_sealed { "image already sealed" } else { "image sealed" },
            "sealed": true,
            "already_sealed": result.already_sealed,
        },
        "image": {
            "name": result.name,
            "path": result.source_path,
            "format": result.image_format,
            "sealed": true,
            "read_only": true,
            "marker": result.marker_path,
        },
        "cleanup": {
            "machine_id": true,
            "dbus_machine_id": true,
            "ssh_host_keys": true,
            "cloud_init": true,
            "random_seed": true,
        },
        "preserved": {
            "agent": true,
            "systemd_unit": true,
            "image_metadata": true,
        },
    })
}

pub(crate) fn emit_seal_failure(format: OutputFormat, failure: &SealFailure) {
    eprintln!("{}", failure.message);
    if format == OutputFormat::Json {
        println!(
            "{}",
            json!({
                "apiVersion": CLI_API_VERSION,
                "command": "image.seal",
                "status": "fail",
                "exit_code": failure.code,
                "summary": {
                    "message": failure.message,
                },
            })
        );
    }
}

impl ImageSealBackend for LibguestfsBackend {
    fn inspect_format(&self, path: &Path) -> Result<String, SealFailure> {
        inspect_image_format(path)
    }

    fn verify_preserved(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_PRESERVATION_CHECKS,
            EXIT_IMAGE_INVARIANT_FAILED,
            "required guest-agent files are missing",
        )
    }

    fn customize(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_CLEANUP_COMMANDS,
            EXIT_IMAGE_CUSTOMIZE_FAILED,
            "image cleanup failed",
        )
    }

    fn verify_clone_safe(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        run_virt_customize(
            path,
            image_format,
            IMAGE_CLONE_SAFE_CHECKS,
            EXIT_IMAGE_INVARIANT_FAILED,
            "clone-safe cleanup verification failed",
        )
    }
}

impl ImageSealBackend for BuilderVmBackend {
    fn inspect_format(&self, path: &Path) -> Result<String, SealFailure> {
        if builder::qemu_img_available() {
            return inspect_image_format(path);
        }
        // Raw-only heuristic when qemu-img is missing: reject qcow magic.
        let mut header = [0_u8; 4];
        let mut file = File::open(path).map_err(|error| {
            SealFailure::new(
                EXIT_INVALID_INPUT,
                format!("cannot open {}: {error}", path.display()),
            )
        })?;
        use std::io::Read;
        let _ = file.read(&mut header);
        if &header == b"QFI\xfb" {
            return Err(SealFailure::new(
                EXIT_INVALID_INPUT,
                "builder backend supports raw images only; convert qcow2 on the host or use a sealed alias",
            ));
        }
        Ok("raw".to_string())
    }

    fn verify_preserved(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn customize(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn verify_clone_safe(&self, _path: &Path, _image_format: &str) -> Result<(), SealFailure> {
        Ok(())
    }

    fn seal_pipeline(&self, path: &Path, image_format: &str) -> Result<(), SealFailure> {
        if image_format != "raw" {
            return Err(SealFailure::new(
                EXIT_INVALID_INPUT,
                format!(
                    "builder backend supports raw images only (got {image_format}); \
                     pull aliases are normalized to raw"
                ),
            ));
        }
        if self.progress {
            eprintln!("Resolving builder appliance…");
        }
        let appliance = builder::ensure_builder_image(&self.images_dir, self.progress)
            .map_err(|failure| SealFailure::new(failure.code, failure.message))?;
        let runbook = builder::seal_runbook();
        if self.progress {
            eprintln!("Sealing via builder VM (one boot)…");
        }
        builder::run_builder_vm(builder::BuilderRunOptions {
            appliance: &appliance,
            target_raw: path,
            runbook: &runbook,
            staging_dir: None,
            timeout: builder::default_timeout(),
            progress: self.progress,
        })
        .map(|_| ())
        .map_err(|failure| SealFailure::new(failure.code, failure.message))
    }
}

pub(crate) fn inspect_image_format(path: &Path) -> Result<String, SealFailure> {
    let qemu = hostbin::resolve("qemu-img").ok_or_else(|| {
        SealFailure::new(
            EXIT_UNAVAILABLE,
            "qemu-img is required to inspect images; run make vendor-qemu-img (or set VZCTL_QEMU_IMG)",
        )
    })?;
    let output = Command::new(qemu)
        .args(["info", "--output=json"])
        .arg(path)
        .output()
        .map_err(|error| {
            SealFailure::new(
                EXIT_UNAVAILABLE,
                format!("qemu-img is required to inspect images: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "qemu-img cannot inspect {}: {}",
                path.display(),
                command_text(&output)
            ),
        ));
    }
    let info: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "qemu-img returned invalid JSON for {}: {error}",
                path.display()
            ),
        )
    })?;
    info["format"].as_str().map(str::to_string).ok_or_else(|| {
        SealFailure::new(
            EXIT_INVALID_INPUT,
            format!("qemu-img did not report a format for {}", path.display()),
        )
    })
}

pub(crate) fn run_virt_customize(
    path: &Path,
    image_format: &str,
    guest_commands: &[&str],
    failure_code: u8,
    failure_context: &str,
) -> Result<(), SealFailure> {
    let mut command = Command::new("virt-customize");
    command
        .arg("--format")
        .arg(image_format)
        .arg("-a")
        .arg(path);
    for guest_command in guest_commands {
        command.arg("--run-command").arg(guest_command);
    }
    let output = command.output().map_err(|error| {
        SealFailure::new(
            EXIT_UNAVAILABLE,
            format!("virt-customize is required on the Linux image builder: {error}"),
        )
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SealFailure::new(
            failure_code,
            format!(
                "{failure_context} for {}: {}",
                path.display(),
                command_text(&output)
            ),
        ))
    }
}
