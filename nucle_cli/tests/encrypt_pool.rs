//! Proves encryption at rest end to end through the real CLI: a stored
//! file survives `encrypt-pool` -> reopen-with-passphrase -> `decrypt-pool`,
//! a plain (no `--pool-key`) invocation is refused once encrypted, a
//! wrong passphrase is refused clearly, the on-disk `state.json` no
//! longer contains the plaintext filename once encrypted, and
//! `NUCLEOS_POOL_PASSPHRASE` works as well as `--pool-key`. Spawns the
//! real, compiled `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_encrypt_test_{}_{}", name, std::process::id()))
}

#[test]
fn a_stored_file_survives_encrypt_then_decrypt_round_trip() {
    let pool_dir = scratch_dir("roundtrip_pool");
    let src_dir = scratch_dir("roundtrip_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("secret.txt");
    std::fs::write(&file, b"a very sensitive payload").unwrap();

    let store = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");
    assert!(store.status.success());

    let encrypt = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("correct horse battery staple")
        .arg("encrypt-pool")
        .output()
        .expect("failed to spawn nucle-cli for encrypt-pool");
    assert!(encrypt.status.success(), "stderr: {}", String::from_utf8_lossy(&encrypt.stderr));

    // The raw on-disk file no longer contains the plaintext filename.
    let raw = std::fs::read(pool_dir.join("state.json")).unwrap();
    let raw_str = String::from_utf8_lossy(&raw);
    assert!(!raw_str.contains("secret.txt"));

    let retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("correct horse battery staple")
        .arg("retrieve").arg("secret.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve");
    assert!(retrieve.status.success());
    assert!(String::from_utf8_lossy(&retrieve.stdout).contains("a very sensitive payload"));

    let decrypt = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("correct horse battery staple")
        .arg("decrypt-pool")
        .output()
        .expect("failed to spawn nucle-cli for decrypt-pool");
    assert!(decrypt.status.success());

    // A plain retrieve (no --pool-key) works again after decrypting.
    let plain_retrieve = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("retrieve").arg("secret.txt")
        .output()
        .expect("failed to spawn nucle-cli for plain retrieve");
    assert!(plain_retrieve.status.success());
    assert!(String::from_utf8_lossy(&plain_retrieve.stdout).contains("a very sensitive payload"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn a_plain_invocation_is_refused_once_the_pool_is_encrypted() {
    let pool_dir = scratch_dir("refuse_plain_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("a passphrase")
        .arg("encrypt-pool")
        .output()
        .expect("failed to spawn nucle-cli for encrypt-pool");

    let plain = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("pool")
        .output()
        .expect("failed to spawn nucle-cli for plain pool status");
    assert!(!plain.status.success());
    assert!(String::from_utf8_lossy(&plain.stderr).contains("is encrypted"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn a_wrong_passphrase_is_refused_clearly() {
    let pool_dir = scratch_dir("wrong_passphrase_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("the real passphrase")
        .arg("encrypt-pool")
        .output()
        .expect("failed to spawn nucle-cli for encrypt-pool");

    let wrong = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("definitely not it")
        .arg("pool")
        .output()
        .expect("failed to spawn nucle-cli with the wrong passphrase");
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("wrong passphrase"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn encrypt_pool_without_a_passphrase_is_a_clear_error() {
    let pool_dir = scratch_dir("no_passphrase_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let result = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("encrypt-pool")
        .output()
        .expect("failed to spawn nucle-cli for encrypt-pool with no passphrase");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("requires a passphrase"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn the_env_var_works_as_well_as_the_flag() {
    let pool_dir = scratch_dir("env_var_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let encrypt = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("encrypt-pool")
        .env("NUCLEOS_POOL_PASSPHRASE", "from the environment")
        .output()
        .expect("failed to spawn nucle-cli for encrypt-pool via env var");
    assert!(encrypt.status.success(), "stderr: {}", String::from_utf8_lossy(&encrypt.stderr));

    let status = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("pool")
        .env("NUCLEOS_POOL_PASSPHRASE", "from the environment")
        .output()
        .expect("failed to spawn nucle-cli for pool status via env var");
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("yes"));

    let _ = std::fs::remove_dir_all(&pool_dir);
}

#[test]
fn a_passphrase_given_to_an_unencrypted_pool_is_a_harmless_noted_no_op() {
    let pool_dir = scratch_dir("noop_passphrase_pool");
    let src_dir = scratch_dir("noop_passphrase_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("a.txt");
    std::fs::write(&file, b"data").unwrap();

    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("0")
        .output()
        .expect("failed to spawn nucle-cli for store");

    let with_unused_key = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("--pool-key").arg("nobody asked for this")
        .arg("retrieve").arg("a.txt")
        .output()
        .expect("failed to spawn nucle-cli for retrieve with an unused key");
    assert!(with_unused_key.status.success());
    assert!(String::from_utf8_lossy(&with_unused_key.stderr).contains("isn't encrypted"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}
