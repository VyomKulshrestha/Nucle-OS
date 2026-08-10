//! A rename is metadata-only but must move the complete version chain across
//! real CLI processes while preserving every historical payload.

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_rename_test_{}", std::process::id()))
}

fn run(pool: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(pool)
        .arg("--json")
        .args(args)
        .output()
        .expect("failed to spawn nucle-cli")
}

#[test]
fn rename_moves_current_and_historical_versions() {
    let root = scratch_dir();
    let pool = root.join("pool");
    let input = root.join("report.txt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    std::fs::write(&input, b"version one").unwrap();
    assert!(Command::new(nucle_cli_bin()).arg("--pool-dir").arg(&pool).arg("store").arg(&input).output().unwrap().status.success());
    std::fs::write(&input, b"version two").unwrap();
    assert!(Command::new(nucle_cli_bin()).arg("--pool-dir").arg(&pool).arg("store").arg(&input).output().unwrap().status.success());

    let renamed = run(&pool, &["rename", "report.txt", "archive/report.txt"]);
    assert!(renamed.status.success(), "stderr: {}", String::from_utf8_lossy(&renamed.stderr));
    let result: serde_json::Value = serde_json::from_slice(&renamed.stdout).unwrap();
    assert_eq!(result["versions_moved"], 2);

    let old = run(&pool, &["retrieve", "report.txt"]);
    assert!(!old.status.success());
    let current = run(&pool, &["retrieve", "archive/report.txt"]);
    assert!(current.status.success());
    let current_json: serde_json::Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(current_json["content"], "version two");
    let historical = run(&pool, &["retrieve", "archive/report.txt", "--version", "1"]);
    assert!(historical.status.success());
    let historical_json: serde_json::Value = serde_json::from_slice(&historical.stdout).unwrap();
    assert_eq!(historical_json["content"], "version one");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rename_refuses_to_overwrite_an_existing_name() {
    let root = scratch_dir().with_extension("collision");
    let pool = root.join("pool");
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    for input in [&first, &second] {
        assert!(Command::new(nucle_cli_bin()).arg("--pool-dir").arg(&pool).arg("store").arg(input).output().unwrap().status.success());
    }

    let result = run(&pool, &["rename", "first.txt", "second.txt"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("already exists"));
    assert!(run(&pool, &["retrieve", "first.txt"]).status.success());
    assert!(run(&pool, &["retrieve", "second.txt"]).status.success());

    let _ = std::fs::remove_dir_all(&root);
}
