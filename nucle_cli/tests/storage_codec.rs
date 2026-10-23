//! End-to-end CLI coverage for selecting the VFS storage codec. This uses
//! separate real processes for store and retrieve so codec metadata must be
//! persisted and used by the later decoder.

use std::path::PathBuf;
use std::process::{Command, Output};

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_storage_codec_test_{}_{}", name, std::process::id()))
}

fn run(pool: &std::path::Path, args: &[&str]) -> Output {
    Command::new(nucle_cli_bin())
        .arg("--pool-dir")
        .arg(pool)
        .args(args)
        .output()
        .expect("failed to spawn nucle-cli")
}

fn expected_hex(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
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
    assert_eq!(result["content"], expected_hex(&original));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fountain_store_round_trips_across_processes_with_persisted_metadata() {
    let root = scratch_dir("fountain");
    let pool = root.join("pool");
    let input = root.join("fountain-data.bin");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let original: Vec<u8> = (0..=255).collect();
    std::fs::write(&input, &original).unwrap();

    let stored = run(
        &pool,
        &[
            "--json",
            "store",
            input.to_str().unwrap(),
            "--codec",
            "fountain",
            "--redundancy",
            "2",
        ],
    );
    assert!(stored.status.success(), "stderr: {}", String::from_utf8_lossy(&stored.stderr));
    let result: serde_json::Value = serde_json::from_slice(&stored.stdout).unwrap();
    assert_eq!(result["codec"], "dna-fountain");

    let retrieved = run(&pool, &["--json", "retrieve", "fountain-data.bin"]);
    assert!(retrieved.status.success(), "stderr: {}", String::from_utf8_lossy(&retrieved.stderr));
    let result: serde_json::Value = serde_json::from_slice(&retrieved.stdout).unwrap();
    assert_eq!(result["is_text"], false);
    assert_eq!(result["content"], expected_hex(&original));

    let history = run(&pool, &["--json", "history", "fountain-data.bin"]);
    assert!(history.status.success(), "stderr: {}", String::from_utf8_lossy(&history.stderr));
    let history: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(history[0]["codec"], "dna-fountain");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cli_migrates_to_and_from_fountain_without_losing_version_reads() {
    let root = scratch_dir("fountain_migration");
    let pool = root.join("pool");
    let input = root.join("migration.txt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&input, b"migration content survives every codec").unwrap();

    let stored = run(&pool, &["store", input.to_str().unwrap(), "--codec", "ternary"]);
    assert!(stored.status.success(), "stderr: {}", String::from_utf8_lossy(&stored.stderr));

    let to_fountain = run(
        &pool,
        &["--json", "migrate", "migration.txt", "--codec", "fountain"],
    );
    assert!(to_fountain.status.success(), "stderr: {}", String::from_utf8_lossy(&to_fountain.stderr));
    let manifest: serde_json::Value = serde_json::from_slice(&to_fountain.stdout).unwrap();
    assert_eq!(manifest["codec"], "dna-fountain");

    let from_fountain = run(
        &pool,
        &["--json", "migrate", "migration.txt", "--codec", "yin-yang"],
    );
    assert!(from_fountain.status.success(), "stderr: {}", String::from_utf8_lossy(&from_fountain.stderr));
    let manifest: serde_json::Value = serde_json::from_slice(&from_fountain.stdout).unwrap();
    assert_eq!(manifest["codec"], "yin-yang");

    let history = run(&pool, &["--json", "history", "migration.txt"]);
    assert!(history.status.success(), "stderr: {}", String::from_utf8_lossy(&history.stderr));
    let history: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    let codecs = history.as_array().unwrap().iter()
        .map(|entry| entry["codec"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(codecs, vec!["ternary-rotating-cipher", "dna-fountain", "yin-yang"]);

    for version in ["1", "2", "3"] {
        let retrieved = run(
            &pool,
            &["--json", "retrieve", "migration.txt", "--version", version],
        );
        assert!(retrieved.status.success(), "stderr: {}", String::from_utf8_lossy(&retrieved.stderr));
        let result: serde_json::Value = serde_json::from_slice(&retrieved.stdout).unwrap();
        assert_eq!(result["content"], "migration content survives every codec");
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cli_extends_fountain_pool_as_a_new_readable_version() {
    let root = scratch_dir("fountain_extend");
    let pool = root.join("pool");
    let input = root.join("extend.txt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&input, b"continued rateless Fountain stream").unwrap();

    let stored = run(
        &pool,
        &[
            "--json",
            "store",
            input.to_str().unwrap(),
            "--codec",
            "fountain",
            "--redundancy",
            "2",
        ],
    );
    assert!(stored.status.success(), "stderr: {}", String::from_utf8_lossy(&stored.stderr));
    let first: serde_json::Value = serde_json::from_slice(&stored.stdout).unwrap();

    let extended = run(
        &pool,
        &["--json", "extend-fountain", "extend.txt", "--droplets", "7"],
    );
    assert!(extended.status.success(), "stderr: {}", String::from_utf8_lossy(&extended.stderr));
    let second: serde_json::Value = serde_json::from_slice(&extended.stdout).unwrap();
    assert_eq!(second["version"], 2);
    assert_eq!(
        second["data_strand_count"].as_u64().unwrap(),
        first["data_strand_count"].as_u64().unwrap() + 7
    );

    for version in ["1", "2"] {
        let retrieved = run(
            &pool,
            &["--json", "retrieve", "extend.txt", "--version", version],
        );
        assert!(retrieved.status.success(), "stderr: {}", String::from_utf8_lossy(&retrieved.stderr));
        let result: serde_json::Value = serde_json::from_slice(&retrieved.stdout).unwrap();
        assert_eq!(result["content"], "continued rateless Fountain stream");
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fountain_rejects_empty_input_without_persisting_a_pool() {
    let root = scratch_dir("fountain_empty");
    let pool = root.join("pool");
    let input = root.join("empty.bin");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&input, []).unwrap();

    let result = run(
        &pool,
        &["store", input.to_str().unwrap(), "--codec", "fountain"],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("empty input data"));
    assert!(!pool.join("state.json").exists());

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
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("unknown codec 'imaginary'"));
    assert!(stderr.contains("ternary, fountain, yin-yang, or hedges"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hedges_store_round_trips_across_processes_and_shows_in_the_header() {
    let root = scratch_dir("hedges");
    let pool = root.join("pool");
    let input = root.join("hedges-data.bin");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let original: Vec<u8> = (0..=255).rev().collect();
    std::fs::write(&input, &original).unwrap();

    let stored = run(&pool, &["--json", "store", input.to_str().unwrap(), "--codec", "hedges", "--redundancy", "3"]);
    assert!(stored.status.success(), "stderr: {}", String::from_utf8_lossy(&stored.stderr));
    let result: serde_json::Value = serde_json::from_slice(&stored.stdout).unwrap();
    assert_eq!(result["codec"], "hedges");
    // 256 bytes are 16 HEDGES strands of 16 bytes, plus 3 parity strands.
    assert_eq!(result["data_strand_count"], 16);
    assert_eq!(result["parity_strand_count"], 3);

    let retrieved = run(&pool, &["--json", "retrieve", "hedges-data.bin"]);
    assert!(retrieved.status.success(), "stderr: {}", String::from_utf8_lossy(&retrieved.stderr));
    let result: serde_json::Value = serde_json::from_slice(&retrieved.stdout).unwrap();
    assert_eq!(result["content"], expected_hex(&original));

    let fasta = root.join("archive.fasta");
    let exported = run(&pool, &["bootstrap", "export", fasta.to_str().unwrap()]);
    assert!(exported.status.success(), "stderr: {}", String::from_utf8_lossy(&exported.stderr));
    let header = run(&pool, &["--json", "bootstrap", "inspect", fasta.to_str().unwrap()]);
    let header: serde_json::Value = serde_json::from_slice(&header.stdout).unwrap();
    // Codec id 4 is HEDGES in the header's file table.
    assert_eq!(header["files"][0]["codec"], 4);

    let _ = std::fs::remove_dir_all(&root);
}
