//! Proves the system-wide audit log: `store`/`retrieve`/`retrieve` (a
//! failing one) each leave a permanent record in `pool_dir/audit.log`,
//! readable across separate `nucle-cli` invocations, with `--tail`
//! correctly limiting to the most recent N events. Spawns the real,
//! compiled `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_audit_test_{}_{}", name, std::process::id()))
}

#[test]
fn store_retrieve_and_a_failed_retrieve_each_appear_in_the_audit_log() {
    let pool_dir = scratch_dir("pool");
    let src_dir = scratch_dir("src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("note.txt");
    std::fs::write(&file, b"audit me").unwrap();

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("note.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve.status.success());

    let failed_retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("missing.txt")
        .output()
        .expect("failed to spawn nucle-cli for failing retrieve");
    assert!(!failed_retrieve.status.success());

    // A separate invocation reads the log back.
    let audit = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--json")
        .arg("audit")
        .output()
        .expect("failed to spawn nucle-cli for audit");
    assert!(audit.status.success());
    let events: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["operation"], "write");
    assert_eq!(events[0]["success"], true);
    assert_eq!(events[1]["operation"], "read");
    assert_eq!(events[1]["success"], true);
    assert_eq!(events[2]["operation"], "read");
    assert_eq!(events[2]["success"], false);
    assert_eq!(events[2]["filename"], "missing.txt");

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn tail_limits_to_the_most_recent_n_events() {
    let pool_dir = scratch_dir("tail_pool");
    let src_dir = scratch_dir("tail_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("note.txt");
    std::fs::write(&file, b"tail me").unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("note.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");

    let tailed = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--json")
        .arg("audit").arg("--tail").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for tailed audit");
    assert!(tailed.status.success());
    let events: serde_json::Value = serde_json::from_slice(&tailed.stdout).unwrap();
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["operation"], "read");

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn an_empty_pool_reports_no_events_not_an_error() {
    let pool_dir = scratch_dir("empty_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let audit = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("audit")
        .output()
        .expect("failed to spawn nucle-cli for audit on an empty pool");
    assert!(audit.status.success());
    assert!(String::from_utf8_lossy(&audit.stdout).contains("No audit events"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}
