//! End-to-end CLI coverage for selecting the VFS storage codec. This uses
//! separate real processes for store and retrieve so codec metadata must be
//! persisted and used by the later decoder.

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_storage_codec_test_{}_{}", name, std::process::id()))
}

#[test]
fn yin_yang_store_round_trips_across_processes() {
    let root = scratch_dir("yin_yang");
    let pool = root.join("pool");
    let input = root.join("codec-data.bin");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let original: Vec<u8> = (0..=255).collect();
    std::fs::write(&input, &original).unwrap();

    let stored = Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(&pool)
        .arg("--json")
        .arg("store")
        .arg(&input)
        .arg("--codec")
        .arg("yin-yang")
        .arg("--redundancy")
        .arg("2")
        .output()
        .expect("failed to spawn store");
    assert!(stored.status.success(), "stderr: {}", String::from_utf8_lossy(&stored.stderr));
    let result: serde_json::Value = serde_json::from_slice(&stored.stdout).unwrap();
    assert_eq!(result["codec"], "yin-yang");

    let retrieved = Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(&pool)
        .arg("--json")
        .arg("retrieve")
        .arg("codec-data.bin")
        .output()
        .expect("failed to spawn retrieve");
    assert!(retrieved.status.success(), "stderr: {}", String::from_utf8_lossy(&retrieved.stderr));
    let result: serde_json::Value = serde_json::from_slice(&retrieved.stdout).unwrap();
    assert_eq!(result["is_text"], false);
    let expected_hex = original.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    assert_eq!(result["content"], expected_hex);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unknown_storage_codec_is_rejected_without_creating_a_pool() {
    let root = scratch_dir("unknown");
    let pool = root.join("pool");
    let input = root.join("data.bin");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&input, b"data").unwrap();

    let result = Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(&pool)
        .arg("store")
        .arg(&input)
        .arg("--codec")
        .arg("imaginary")
        .output()
        .expect("failed to spawn store");
    assert!(!result.status.success());
    assert!(!pool.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unknown codec 'imaginary'"));

    let _ = std::fs::remove_dir_all(&root);
}
