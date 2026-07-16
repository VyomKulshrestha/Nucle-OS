//! Proves the optimistic-concurrency safety: two `nucle store`
//! invocations racing on the same pool directory never
//! silently lose one's write. Spawns two real, concurrent `nucle-cli`
//! processes (`std::process::Command::spawn`, not `.output()`, so neither
//! blocks the other) -- unlike `nucle_vfs`'s own unit tests, which prove
//! the version-check logic deterministically by directly constructing two
//! stale `NucleOS` instances, this is the genuine, unforced, real-process
//! version of the same guarantee.
//!
//! The specific winner of a real OS race is inherently non-deterministic,
//! so this asserts the invariant that must hold regardless of who wins:
//! every process either (a) succeeds, and its own file is genuinely
//! retrievable afterward, or (b) fails with a clear, actionable "changed by
//! another process... retry" error -- never (c) succeeds while its file is
//! silently missing, which is the exact bug this step closes.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_concurrency_test_{}_{}", name, std::process::id()))
}

#[test]
fn two_concurrent_stores_never_silently_lose_a_write() {
    let pool_dir = scratch_dir("race_pool");
    let src_dir = scratch_dir("race_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file_a = src_dir.join("race_a.txt");
    let file_b = src_dir.join("race_b.txt");
    std::fs::write(&file_a, b"race file A").unwrap();
    std::fs::write(&file_b, b"race file B").unwrap();

    // Spawn (not run-and-wait) both as close together as possible, so they
    // genuinely overlap rather than running strictly sequentially.
    let mut child_a = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file_a)
        .arg("--redundancy").arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn nucle-cli for process A");
    let mut child_b = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file_b)
        .arg("--redundancy").arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn nucle-cli for process B");

    let output_a = child_a.wait_with_output().unwrap();
    let output_b = child_b.wait_with_output().unwrap();

    let list = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("list")
        .output()
        .expect("failed to spawn nucle-cli for list");
    let listed = String::from_utf8_lossy(&list.stdout).to_string();

    for (label, output, filename) in [("A", &output_a, "race_a.txt"), ("B", &output_b, "race_b.txt")] {
        if output.status.success() {
            // Success must mean genuinely persisted -- never a silent loss.
            assert!(
                listed.contains(filename),
                "process {} reported success but '{}' is missing from the final pool -- silent data loss:\nlist output: {}",
                label, filename, listed
            );
        } else {
            // Failure must be the specific, actionable conflict error --
            // not some other, unrelated crash.
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("another process") && stderr.contains("retry"),
                "process {} failed for an unexpected reason: {}",
                label, stderr
            );
        }
    }

    // At least one of the two must have actually succeeded.
    assert!(output_a.status.success() || output_b.status.success(), "both processes failed -- neither write landed");

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}
