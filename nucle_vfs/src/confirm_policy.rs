//! # OS-user allowlist for `--confirm`
//!
//! `nucle_hardware::confirm`'s `--confirm` flag is a bare acknowledgment
//! with zero identity behind it: anyone who can run the CLI can pass it
//! for any cost-bearing or destructive hardware batch. This module adds
//! an *optional*, pool-scoped allowlist of OS usernames (`pool_dir/
//! config.json`) that `--confirm` must additionally satisfy once
//! configured.
//!
//! **Be honest about what this is.** The "identity" here is just
//! `$USER`/`%USERNAME%` — an environment variable, not a credential.
//! Anyone with local shell access can set it to any value they like
//! before invoking the CLI. This is slightly more real than a shared
//! secret sitting next to the pool it protects (at least it's not a
//! literal password file to steal), but it is **not** cryptographic and
//! does **not** resist a determined attacker with filesystem/shell
//! access — only real key management or an OS-level credential check
//! would. What it *does* do: catch accidents and unintended automation
//! (a script running as the wrong user, a CI job that shouldn't have
//! hardware access) from ever reaching `--confirm`'s gate.
//!
//! An empty/unconfigured allowlist (the default — no `pool_dir/
//! config.json`, or one with an empty list) is a no-op: every pool
//! behaves exactly as before this module existed. The check only
//! activates once at least one user has been added via `nucle
//! confirm-users --add`.

use serde::{Deserialize, Serialize};
use std::path::Path;

const CONFIG_FILE_NAME: &str = "config.json";

/// Pool-scoped configuration. Currently just the confirm-allowlist, but
/// its own file (distinct from `state.json`) since it's operator policy,
/// not pool data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfirmPolicy {
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// Loads `pool_dir/config.json`, or a default (empty allowlist) if it
/// doesn't exist yet — a pool that's never configured one behaves as if
/// this module didn't exist.
pub fn load(pool_dir: &Path) -> Result<ConfirmPolicy, String> {
    let path = pool_dir.join(CONFIG_FILE_NAME);
    if !path.exists() {
        return Ok(ConfirmPolicy::default());
    }
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read pool config at '{}': {}", path.display(), e))?;
    serde_json::from_str(&json)
        .map_err(|e| format!("failed to parse pool config at '{}': {}", path.display(), e))
}

/// Writes `policy` to `pool_dir/config.json` atomically (temp file, then
/// rename over the real path), matching `NucleOS::persist`'s convention
/// for pool_dir-scoped files so a process killed mid-write never leaves a
/// half-written config behind.
pub fn save(pool_dir: &Path, policy: &ConfirmPolicy) -> Result<(), String> {
    std::fs::create_dir_all(pool_dir)
        .map_err(|e| format!("failed to create pool directory '{}': {}", pool_dir.display(), e))?;

    let json = serde_json::to_string_pretty(policy)
        .map_err(|e| format!("failed to serialize pool config: {}", e))?;

    let tmp_path = pool_dir.join(format!("{}.tmp", CONFIG_FILE_NAME));
    std::fs::write(&tmp_path, &json)
        .map_err(|e| format!("failed to write pool config to '{}': {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, pool_dir.join(CONFIG_FILE_NAME))
        .map_err(|e| format!("failed to finalize pool config: {}", e))
}

/// True if `user` may pass `--confirm` under `policy` — an empty
/// allowlist allows everyone (today's default, unconfigured behavior);
/// a non-empty one requires an exact match.
pub fn is_allowed(policy: &ConfirmPolicy, user: &str) -> bool {
    policy.allowed_users.is_empty() || policy.allowed_users.iter().any(|u| u == user)
}

/// The invoking OS user, read directly from the environment (`%USERNAME%`
/// on Windows, `$USER` elsewhere) — not a syscall like `GetUserNameW`/
/// `libc::getuid`, because this whole check is a courtesy gate against
/// accidents, not a real identity boundary (see this module's doc
/// comment). `None` if the variable isn't set at all.
#[cfg(windows)]
pub fn current_os_user() -> Option<String> {
    std::env::var("USERNAME").ok()
}

#[cfg(not(windows))]
pub fn current_os_user() -> Option<String> {
    std::env::var("USER").ok()
}

/// Checks the invoking OS user against `pool_dir`'s configured allowlist.
/// `Ok(())` if the allowlist is empty/unconfigured (a no-op) or the
/// current user is on it; `Err` naming why otherwise — either the user
/// isn't allowed, or (once a non-empty allowlist exists) the OS user
/// can't be determined at all, which refuses rather than guessing.
pub fn check(pool_dir: &Path) -> Result<(), String> {
    let policy = load(pool_dir)?;
    if policy.allowed_users.is_empty() {
        return Ok(());
    }

    match current_os_user() {
        Some(user) if is_allowed(&policy, &user) => Ok(()),
        Some(user) => Err(format!(
            "OS user '{}' is not on this pool's confirm-allowlist ({}) -- run \
             'nucle confirm-users --add {}' to add it",
            user, policy.allowed_users.join(", "), user
        )),
        None => Err(
            "could not determine the invoking OS user (no USER/USERNAME environment \
             variable set), and this pool has a confirm-allowlist configured -- refusing \
             rather than guessing"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nucle_vfs_confirm_policy_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn an_empty_allowlist_allows_any_user() {
        let policy = ConfirmPolicy::default();
        assert!(is_allowed(&policy, "alice"));
        assert!(is_allowed(&policy, "anyone"));
    }

    #[test]
    fn a_configured_allowlist_only_allows_listed_users() {
        let policy = ConfirmPolicy { allowed_users: vec!["alice".to_string(), "bob".to_string()] };
        assert!(is_allowed(&policy, "alice"));
        assert!(is_allowed(&policy, "bob"));
        assert!(!is_allowed(&policy, "eve"));
    }

    #[test]
    fn loading_a_config_that_does_not_exist_yet_returns_default_empty() {
        let dir = scratch_dir("missing");
        let _ = std::fs::remove_dir_all(&dir);
        let policy = load(&dir).unwrap();
        assert!(policy.allowed_users.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = scratch_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        let policy = ConfirmPolicy { allowed_users: vec!["alice".to_string()] };
        save(&dir, &policy).unwrap();

        let reloaded = load(&dir).unwrap();
        assert_eq!(reloaded, policy);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_is_a_no_op_against_an_unconfigured_pool() {
        let dir = scratch_dir("unconfigured");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(check(&dir).is_ok());
    }
}
