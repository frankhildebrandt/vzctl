use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;

const SKILL_DIR_NAME: &str = "vzctl";
const FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../skill/SKILL.md")),
    ("yaml.md", include_str!("../skill/yaml.md")),
    ("cli.md", include_str!("../skill/cli.md")),
    ("example.yaml", include_str!("../skill/example.yaml")),
];

#[derive(Debug, Eq, PartialEq)]
enum Action {
    Print,
    Help,
    InstallLocal,
    InstallGlobal,
}

/// Print or install the bundled vzctl agent skill and its attachments.
pub(crate) fn command(args: impl Iterator<Item = String>) -> ExitCode {
    match parse_action(args) {
        Ok(Action::Help) => {
            crate::help::print_topic("skill");
            ExitCode::SUCCESS
        }
        Ok(Action::Print) => {
            print_skill();
            ExitCode::SUCCESS
        }
        Ok(Action::InstallLocal) => install(&local_skill_dir()),
        Ok(Action::InstallGlobal) => match global_skill_dir() {
            Ok(path) => install(&path),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(EXIT_INVALID)
            }
        },
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn parse_action(args: impl Iterator<Item = String>) -> Result<Action, String> {
    let mut install_local = false;
    let mut install_global = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" | "help" => return Ok(Action::Help),
            "--install-local" => install_local = true,
            "--install-global" => install_global = true,
            other => return Err(format!("unknown skill option: {other}")),
        }
    }
    match (install_local, install_global) {
        (false, false) => Ok(Action::Print),
        (true, false) => Ok(Action::InstallLocal),
        (false, true) => Ok(Action::InstallGlobal),
        (true, true) => {
            Err("skill accepts only one of --install-local or --install-global".to_string())
        }
    }
}

/// Dump each bundled file to stdout with a `===== name =====` separator.
fn print_skill() {
    for (index, (name, body)) in FILES.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("===== {name} =====");
        print!("{body}");
        if !body.ends_with('\n') {
            println!();
        }
    }
}

fn local_skill_dir() -> PathBuf {
    PathBuf::from(".agents").join("skills").join(SKILL_DIR_NAME)
}

fn global_skill_dir() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".agents")
        .join("skills")
        .join(SKILL_DIR_NAME))
}

/// Write bundled skill files into `dir`, creating parent directories as needed.
fn install(dir: &Path) -> ExitCode {
    if let Err(error) = fs::create_dir_all(dir) {
        eprintln!("cannot create {}: {error}", dir.display());
        return ExitCode::from(EXIT_INVALID);
    }
    for (name, body) in FILES {
        let path = dir.join(name);
        if let Err(error) = fs::write(&path, body) {
            eprintln!("cannot write {}: {error}", path.display());
            return ExitCode::from(EXIT_INVALID);
        }
    }
    let names: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
    println!("wrote {} to {}", names.join(", "), dir.display());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_print_with_no_args() {
        assert_eq!(
            parse_action(Vec::<String>::new().into_iter()).unwrap(),
            Action::Print
        );
    }

    #[test]
    fn parse_install_flags() {
        assert_eq!(
            parse_action(["--install-local".into()].into_iter()).unwrap(),
            Action::InstallLocal
        );
        assert_eq!(
            parse_action(["--install-global".into()].into_iter()).unwrap(),
            Action::InstallGlobal
        );
    }

    #[test]
    fn parse_rejects_both_install_flags() {
        let error = parse_action(["--install-local".into(), "--install-global".into()].into_iter())
            .unwrap_err();
        assert!(error.contains("--install-local"));
    }

    #[test]
    fn parse_help_wins_over_install() {
        assert_eq!(
            parse_action(["--install-local".into(), "--help".into()].into_iter()).unwrap(),
            Action::Help
        );
    }

    #[test]
    fn bundled_files_use_skill_frontmatter() {
        let skill = FILES
            .iter()
            .find(|(name, _)| *name == "SKILL.md")
            .map(|(_, body)| *body)
            .unwrap();
        assert!(skill.contains("name: vzctl"));
        assert!(skill.contains("hypernetwork.config.yaml"));
    }

    #[test]
    fn bundled_example_validates() {
        crate::config::validate_source(include_str!("../skill/example.yaml"))
            .expect("bundled example.yaml must validate");
    }
}
