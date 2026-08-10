use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn nucle_cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nucle_cli_version_lifecycle_{}_{}",
        name,
        std::process::id()
    ))
}

fn run(pool_dir: &Path, args: &[&str]) -> Output {
    Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(pool_dir)
        .args(args)
        .output()
        .expect("failed to spawn nucle-cli")
}

fn store_two_versions(root: &Path) -> PathBuf {
    let pool_dir = root.join("pool");
    let source_dir = root.join("source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("versioned.txt");

    std::fs::write(&source, b"first version").unwrap();
    let first = run(&pool_dir, &["store", source.to_str().unwrap()]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));

    std::fs::write(&source, b"second version").unwrap();
    let second = run(&pool_dir, &["store", source.to_str().unwrap()]);
    assert!(second.status.success(), "{}", String::from_utf8_lossy(&second.stderr));

    pool_dir
}

#[test]
fn history_and_versioned_retrieve_work_across_real_processes() {
    let root = scratch_dir("history");
    let _ = std::fs::remove_dir_all(&root);
    let pool_dir = store_two_versions(&root);

    let history = run(&pool_dir, &["--json", "history", "versioned.txt"]);
    assert!(history.status.success(), "{}", String::from_utf8_lossy(&history.stderr));
    let versions: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    let versions = versions.as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["version"], 1);
    assert_eq!(versions[0]["is_current"], false);
    assert_eq!(versions[1]["version"], 2);
    assert_eq!(versions[1]["is_current"], true);

    let old = run(
        &pool_dir,
        &["--json", "retrieve", "versioned.txt", "--version", "1"],
    );
    assert!(old.status.success(), "{}", String::from_utf8_lossy(&old.stderr));
    let old: serde_json::Value = serde_json::from_slice(&old.stdout).unwrap();
    assert_eq!(old["version"], 1);
    assert_eq!(old["content"], "first version");

    let current = run(&pool_dir, &["--json", "retrieve", "versioned.txt"]);
    assert!(current.status.success(), "{}", String::from_utf8_lossy(&current.stderr));
    let current: serde_json::Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(current["version"], 2);
    assert_eq!(current["content"], "second version");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_requires_confirmation_and_removes_the_complete_version_chain() {
    let root = scratch_dir("delete");
    let _ = std::fs::remove_dir_all(&root);
    let pool_dir = store_two_versions(&root);

    let refused = run(&pool_dir, &["delete", "versioned.txt"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("rerun with --confirm"));

    let still_there = run(&pool_dir, &["--json", "history", "versioned.txt"]);
    assert!(still_there.status.success());

    let deleted = run(
        &pool_dir,
        &["--json", "delete", "versioned.txt", "--confirm"],
    );
    assert!(deleted.status.success(), "{}", String::from_utf8_lossy(&deleted.stderr));
    let deleted: serde_json::Value = serde_json::from_slice(&deleted.stdout).unwrap();
    assert_eq!(deleted["versions_removed"], 2);
    assert!(deleted["strands_removed"].as_u64().unwrap() > 0);

    let missing = run(&pool_dir, &["history", "versioned.txt"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("not found"));

    let _ = std::fs::remove_dir_all(&root);
}
