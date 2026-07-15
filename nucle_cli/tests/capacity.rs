//! Proves the pool capacity limits from actions2.md's Step 7: a small
//! configured capacity refuses a write that would exceed it with a clear,
//! capacity-specific error (not the old confusing primer-exhaustion
//! message), the limit persists across separate `nucle-cli` invocations,
//! and clearing it removes the restriction. Spawns the real, compiled
//! `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_capacity_test_{}_{}", name, std::process::id()))
}

#[test]
fn a_small_capacity_refuses_an_oversized_write_and_persists_across_invocations() {
    let pool_dir = scratch_dir("pool");
    let src_dir = scratch_dir("src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let big_file = src_dir.join("big.bin");
    std::fs::write(&big_file, vec![0u8; 5000]).unwrap();

    // Set a capacity too small for the file above.
    let set_capacity = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("capacity").arg("100")
        .output()
        .expect("failed to spawn nucle-cli for capacity set");
    assert!(set_capacity.status.success());
    assert!(String::from_utf8_lossy(&set_capacity.stdout).contains("100"));

    // A separate invocation still sees the persisted limit.
    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&big_file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(!store.status.success());
    let stderr = String::from_utf8_lossy(&store.stderr);
    assert!(stderr.contains("capacity exceeded"), "expected a capacity-specific error, got: {}", stderr);

    // The refused write must not have created a catalog entry.
    let list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for list");
    assert!(String::from_utf8_lossy(&list.stdout).contains("No files stored"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn clearing_the_capacity_limit_lets_a_previously_refused_write_through() {
    let pool_dir = scratch_dir("clear_pool");
    let src_dir = scratch_dir("clear_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let big_file = src_dir.join("big.bin");
    std::fs::write(&big_file, vec![0u8; 5000]).unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("capacity").arg("100")
        .output()
        .expect("failed to spawn nucle-cli for capacity set");

    let blocked = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&big_file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for blocked store");
    assert!(!blocked.status.success());

    let clear = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("capacity")
        .arg("--unlimited")
        .output()
        .expect("failed to spawn nucle-cli for capacity clear");
    assert!(clear.status.success());
    assert!(String::from_utf8_lossy(&clear.stdout).contains("unlimited"));

    let now_fine = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&big_file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for now-allowed store");
    assert!(now_fine.status.success(), "store failed: {}", String::from_utf8_lossy(&now_fine.stderr));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}
