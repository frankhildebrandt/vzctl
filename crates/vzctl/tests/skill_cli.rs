use std::fs;
use std::process::Command;

fn vzctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vzctl"))
}

#[test]
fn skill_prints_skill_and_attachments() {
    let output = vzctl().arg("skill").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("===== SKILL.md ====="));
    assert!(stdout.contains("name: vzctl"));
    assert!(stdout.contains("===== yaml.md ====="));
    assert!(stdout.contains("apiVersion: hypernetwork/v1"));
    assert!(stdout.contains("===== cli.md ====="));
    assert!(stdout.contains("vzctl validate"));
    assert!(stdout.contains("===== example.yaml ====="));
    assert!(stdout.contains("domain: demo.vz.test"));
}

#[test]
fn skill_help_lists_install_flags() {
    let output = vzctl().args(["skill", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--install-local"));
    assert!(stdout.contains("--install-global"));
    assert!(stdout.contains(".agents/skills/vzctl"));
}

#[test]
fn skill_unknown_flag_is_usage() {
    let output = vzctl().args(["skill", "--wat"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown skill option"));
}

#[test]
fn skill_rejects_both_install_flags() {
    let output = vzctl()
        .args(["skill", "--install-local", "--install-global"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn skill_install_local_writes_agents_skills() {
    let root = std::env::temp_dir().join(format!(
        "vzctl-skill-local-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let output = vzctl()
        .arg("skill")
        .arg("--install-local")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let skill_dir = root.join(".agents/skills/vzctl");
    let skill = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(skill.contains("name: vzctl"));
    assert!(skill_dir.join("yaml.md").is_file());
    assert!(skill_dir.join("cli.md").is_file());
    assert!(skill_dir.join("example.yaml").is_file());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(".agents/skills/vzctl"));

    let again = vzctl()
        .arg("skill")
        .arg("--install-local")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(again.status.success());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn skill_install_global_uses_home_agents() {
    let root = std::env::temp_dir().join(format!(
        "vzctl-skill-global-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let output = vzctl()
        .arg("skill")
        .arg("--install-global")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let skill_dir = home.join(".agents/skills/vzctl");
    assert!(skill_dir.join("SKILL.md").is_file());
    assert!(skill_dir.join("example.yaml").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn top_level_help_lists_skill() {
    let output = vzctl().arg("help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("skill"));
    assert!(stdout.contains("help exit-codes"));
    assert!(stdout.contains("services"));
}
