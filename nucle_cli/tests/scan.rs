//! Proves proactive integrity scanning end to end through the real CLI:
//! a healthy pool scans clean with exit 0, a pool whose on-disk
//! `state.json` was corrupted afterward (silent bit-rot, simulated by
//! mutating its raw bytes directly -- the same style `persistence.rs`
//! already uses for its stray-`.tmp`-file test) scans as corrupted with
//! a non-zero exit, and prefix filtering only covers matching files.
//! Spawns the real, compiled `nucle-cli` binary
//! (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_scan_test_{}_{}", name, std::process::id()))
}

#[test]
fn a_healthy_pool_scans_clean_with_a_zero_exit_code() {
    let pool_dir = scratch_dir("healthy_pool");
    let src_dir = scratch_dir("healthy_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("a.txt");
    std::fs::write(&file, b"perfectly fine contents").unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");

    let scan = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("scan")
        .output()
        .expect("failed to spawn nucle-cli for scan");
    assert!(scan.status.success());
    let stdout = String::from_utf8_lossy(&scan.stdout);
    assert!(stdout.contains("1 healthy"));
    assert!(stdout.contains("0 corrupted"));
    assert!(stdout.contains("[OK"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn a_corrupted_state_json_scans_as_corrupted_with_a_nonzero_exit_code() {
    let pool_dir = scratch_dir("corrupted_pool");
    let src_dir = scratch_dir("corrupted_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("victim.txt");
    std::fs::write(&file, b"this file will get silently corrupted on disk").unwrap();

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    // Simulate silent bit-rot: flip every "A" base to "T" directly in the
    // persisted state, without going through any real API. The quoted
    // form ("A" -> "T") only ever matches a literal single-base JSON
    // array entry, never incidentally matching inside a longer string
    // like an archive ID or codec name.
    let state_path = pool_dir.join("state.json");
    let raw = std::fs::read_to_string(&state_path).unwrap();
    let corrupted = raw.replace("\"A\"", "\"T\"");
    assert_ne!(raw, corrupted, "expected at least one base to actually flip");
    std::fs::write(&state_path, corrupted).unwrap();

    let scan = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("scan")
        .output()
        .expect("failed to spawn nucle-cli for scan");
    assert!(!scan.status.success(), "expected a non-zero exit once corruption is found");
    let stdout = String::from_utf8_lossy(&scan.stdout);
    assert!(stdout.contains("1 corrupted"));
    assert!(stdout.contains("CORRUPTED"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn scanning_with_a_prefix_only_covers_matching_files() {
    let pool_dir = scratch_dir("prefix_pool");
    let src_dir = scratch_dir("prefix_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(src_dir.join("docs")).unwrap();
    std::fs::create_dir_all(src_dir.join("downloads")).unwrap();
    std::fs::write(src_dir.join("docs/a.txt"), b"in scope").unwrap();
    std::fs::write(src_dir.join("downloads/b.txt"), b"out of scope").unwrap();

    for rel in ["docs/a.txt", "downloads/b.txt"] {
        Command::new(nucle_cli_bin())
            .current_dir(&src_dir)
            .arg("--pool-dir").arg(&pool_dir)
            .arg("store").arg(rel)
            .arg("--redundancy").arg("1")
            .output()
            .expect("failed to spawn nucle-cli for store");
    }

    let scan = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--json")
        .arg("scan").arg("docs/")
        .output()
        .expect("failed to spawn nucle-cli for scoped scan");
    assert!(scan.status.success());
    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(report["total_files"], 1);
    assert_eq!(report["results"][0]["filename"], "docs/a.txt");

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}
