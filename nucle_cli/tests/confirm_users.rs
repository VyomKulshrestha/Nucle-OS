//! Proves the OS-user confirm-allowlist: an unconfigured pool lets any
//! user pass `--confirm`; once a user is added, `--confirm` is refused
//! for every other OS user and allowed again for the real, live invoking
//! user. Reads the actual test-runner's OS user from the environment at
//! test time (never hardcoded) since this is exercising real process
//! identity, not a mock. Spawns the real, compiled `nucle-cli` binary
//! (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_confirm_users_test_{}_{}", name, std::process::id()))
}

/// The real OS user running this test, exactly as `confirm_policy::
/// current_os_user` would read it -- so the "allowed" case below is
/// asserting against a genuine identity, not a fixture.
fn real_current_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .expect("USER or USERNAME must be set to run this test")
}

const EXAMPLE_PROGRAM: &str = "../docs/examples/effect_confirmations.nsl";

#[test]
fn an_unconfigured_pool_lets_confirm_through_for_any_user() {
    let pool_dir = scratch_dir("unconfigured");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let export = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("hardware").arg("export").arg(EXAMPLE_PROGRAM)
        .arg("--provider").arg("mock")
        .arg("--confirm")
        .output()
        .expect("failed to spawn nucle-cli for export");
    assert!(export.status.success(), "stderr: {}", String::from_utf8_lossy(&export.stderr));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn a_configured_allowlist_refuses_confirm_for_a_user_not_on_it() {
    let pool_dir = scratch_dir("refuses");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let add = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("confirm-users").arg("--add").arg("someone_who_is_definitely_not_running_this_test")
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --add");
    assert!(add.status.success());

    let export = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("hardware").arg("export").arg(EXAMPLE_PROGRAM)
        .arg("--provider").arg("mock")
        .arg("--confirm")
        .output()
        .expect("failed to spawn nucle-cli for export");
    assert!(!export.status.success());
    let stderr = String::from_utf8_lossy(&export.stderr);
    assert!(stderr.contains("not on this pool's confirm-allowlist"), "got: {}", stderr);

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn adding_the_real_invoking_user_lets_confirm_through_again() {
    let pool_dir = scratch_dir("allowed");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let me = real_current_user();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("confirm-users").arg("--add").arg("someone_else")
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --add someone_else");

    let refused = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("hardware").arg("export").arg(EXAMPLE_PROGRAM)
        .arg("--provider").arg("mock")
        .arg("--confirm")
        .output()
        .expect("failed to spawn nucle-cli for export");
    assert!(!refused.status.success());

    let add_me = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("confirm-users").arg("--add").arg(&me)
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --add <me>");
    assert!(add_me.status.success());
    assert!(String::from_utf8_lossy(&add_me.stdout).contains(&me));

    let allowed = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("hardware").arg("export").arg(EXAMPLE_PROGRAM)
        .arg("--provider").arg("mock")
        .arg("--confirm")
        .output()
        .expect("failed to spawn nucle-cli for export");
    assert!(allowed.status.success(), "stderr: {}", String::from_utf8_lossy(&allowed.stderr));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn removing_a_user_takes_them_back_off_the_allowlist() {
    let pool_dir = scratch_dir("removed");
    let _ = std::fs::remove_dir_all(&pool_dir);

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("confirm-users").arg("--add").arg("temp_user")
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --add");

    let remove = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--json")
        .arg("confirm-users").arg("--remove").arg("temp_user")
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --remove");
    assert!(remove.status.success());
    let policy: serde_json::Value = serde_json::from_slice(&remove.stdout).unwrap();
    assert_eq!(policy["allowed_users"].as_array().unwrap().len(), 0);

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn a_batch_with_no_effectful_requests_never_needs_the_allowlist() {
    // Confirms the check is only ever consulted when a batch actually
    // needs --confirm -- a Qc/Recovery-only (Pure) batch is unaffected
    // by an allowlist that would otherwise refuse this OS user.
    let pool_dir = scratch_dir("pure_only");
    let _ = std::fs::remove_dir_all(&pool_dir);

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("confirm-users").arg("--add").arg("someone_else_entirely")
        .output()
        .expect("failed to spawn nucle-cli for confirm-users --add");

    // consensus_vote-only program: produces a Recovery request, which is
    // Pure and needs no --confirm at all, so the allowlist should never
    // even be consulted.
    let pure_program = "../docs/examples/probabilistic_recovery.nsl";
    let export = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("hardware").arg("export").arg(pure_program)
        .arg("--provider").arg("mock")
        .output()
        .expect("failed to spawn nucle-cli for export");
    assert!(export.status.success(), "stderr: {}", String::from_utf8_lossy(&export.stderr));

    let _ = std::fs::remove_dir_all(&pool_dir);
}
