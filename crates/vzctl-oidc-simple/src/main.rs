//! Dev-only OIDC IdP: pick a username, get tokens, logout clears session.

mod server;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use server::{load_config, run};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = env::args().skip(1);
    let mut config_path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                let Some(path) = args.next() else {
                    eprintln!("vzctl-oidc-simple: --config requires a path");
                    return ExitCode::from(2);
                };
                config_path = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                eprintln!("usage: vzctl-oidc-simple --config <path.json>");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("vzctl-oidc-simple: unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(config_path) = config_path else {
        eprintln!("usage: vzctl-oidc-simple --config <path.json>");
        return ExitCode::from(2);
    };

    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vzctl-oidc-simple: {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = run(config, config_path.parent().map(|p| p.to_path_buf())).await {
        eprintln!("vzctl-oidc-simple: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
