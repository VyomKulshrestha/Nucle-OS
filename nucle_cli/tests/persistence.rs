//! Proves `nucle store` then `nucle retrieve` genuinely persist across
//! separate `nucle-cli` process invocations -- not just within one
//! process's own memory. Before `NucleOS::open`/`persist` existed, this
//! exact sequence failed with "file not found" because every command
//! started a fresh, empty, in-memory `NucleOS::new()`.
//!
//! Spawns the real, compiled `nucle-cli` binary twice via
//! `CARGO_BIN_EXE_nucle-cli` -- an in-memory `to_json`/`from_json`
//! roundtrip (already covered by `nucle_vfs`'s own unit tests) doesn't
//! prove this; only a real second OS process does.

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_persistence_test_{}_{}", name, std::process::id()))
}

#[test]
fn store_then_retrieve_across_separate_processes_succeeds() {
    let pool_dir = scratch_dir("store_retrieve_pool");
    let source_dir = scratch_dir("store_retrieve_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&source_dir);
    std::fs::create_dir_all(&source_dir).unwrap();

    let source_file = source_dir.join("persisted.txt");
    std::fs::write(&source_file, b"cross-process persistence proof").unwrap();

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&source_file)
        .arg("--redundancy").arg("2")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(
        store.status.success(),
        "store failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&store.stdout),
        String::from_utf8_lossy(&store.stderr)
    );

    // A genuinely separate process -- not the one that just stored.
    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("persisted.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(
        retrieve.status.success(),
        "retrieve failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&retrieve.stdout),
        String::from_utf8_lossy(&retrieve.stderr)
    );
    assert!(String::from_utf8_lossy(&retrieve.stdout).contains("cross-process persistence proof"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&source_dir);
}

#[test]
fn retrieve_without_a_prior_store_fails_cleanly_not_silently() {
    // Guards against a regression where a missing pool directory is
    // treated as "file exists but is empty" instead of a clean not-found.
    let pool_dir = scratch_dir("retrieve_only_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("never_stored.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(!retrieve.status.success());
    assert!(String::from_utf8_lossy(&retrieve.stderr).contains("not found"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn a_stray_partial_tmp_file_does_not_corrupt_a_later_open() {
    // Simulates a process killed mid-persist: a leftover state.json.tmp
    // sitting next to an already-finalized, good state.json must never be
    // picked up -- proves the atomic write path end to end, through the
    // real CLI, not just nucle_vfs's own unit test of NucleOS::open.
    let pool_dir = scratch_dir("atomic_write_pool");
    let source_dir = scratch_dir("atomic_write_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&source_dir);
    std::fs::create_dir_all(&source_dir).unwrap();

    let source_file = source_dir.join("good.txt");
    std::fs::write(&source_file, b"last good state").unwrap();

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&source_file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    std::fs::write(pool_dir.join("state.json.tmp"), b"{ not even valid json").unwrap();

    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("good.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve.status.success());
    assert!(String::from_utf8_lossy(&retrieve.stdout).contains("last good state"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&source_dir);
}
