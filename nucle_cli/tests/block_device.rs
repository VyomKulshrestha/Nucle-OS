use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn nucle_cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_block_device_{}_{}", name, std::process::id()))
}

fn run(args: &[&str]) -> Output {
    Command::new(nucle_cli_bin())
        .args(args)
        .output()
        .expect("failed to spawn nucle-cli")
}

fn create(root: &Path, backend: &str) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let image = root.join(format!("{backend}.json"));
    let created = run(&[
        "block", "create", image.to_str().unwrap(),
        "--backend", backend,
        "--block-size", "16",
        "--capacity-blocks", "4",
        "--latency-ms", "0",
        "--coverage", "12",
    ]);
    assert!(created.status.success(), "{}", String::from_utf8_lossy(&created.stderr));
    image
}

#[test]
fn a_persistent_rewritable_device_round_trips_across_cli_processes() {
    let root = scratch_dir("rewritable");
    let _ = std::fs::remove_dir_all(&root);
    let image = create(&root, "rewritable-nanopore");
    let input = root.join("input.bin");
    let output = root.join("output.bin");
    std::fs::write(&input, *b"0123456789ABCDEF").unwrap();

    let write = run(&[
        "block", "write", image.to_str().unwrap(), "2", input.to_str().unwrap(),
    ]);
    assert!(write.status.success(), "{}", String::from_utf8_lossy(&write.stderr));

    let info = run(&["--json", "block", "info", image.to_str().unwrap()]);
    assert!(info.status.success(), "{}", String::from_utf8_lossy(&info.stderr));
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(info["backend"], "rewritable-nanopore");
    assert_eq!(info["written_blocks"], 1);
    assert_eq!(info["rewritable"], true);

    let read = run(&[
        "block", "read", image.to_str().unwrap(), "2", "--output", output.to_str().unwrap(),
    ]);
    assert!(read.status.success(), "{}", String::from_utf8_lossy(&read.stderr));
    assert_eq!(std::fs::read(&output).unwrap(), b"0123456789ABCDEF");

    std::fs::write(&input, *b"FEDCBA9876543210").unwrap();
    let rewrite = run(&[
        "block", "write", image.to_str().unwrap(), "2", input.to_str().unwrap(),
    ]);
    assert!(rewrite.status.success(), "{}", String::from_utf8_lossy(&rewrite.stderr));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_persistent_synthesis_array_refuses_overwrite_without_losing_data() {
    let root = scratch_dir("write_once");
    let _ = std::fs::remove_dir_all(&root);
    let image = create(&root, "synthesis-array");
    let input = root.join("input.bin");
    let output = root.join("output.bin");
    std::fs::write(&input, *b"write-once-data!").unwrap();

    let first = run(&[
        "block", "write", image.to_str().unwrap(), "0", input.to_str().unwrap(),
    ]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));

    std::fs::write(&input, *b"replacement-data").unwrap();
    let second = run(&[
        "block", "write", image.to_str().unwrap(), "0", input.to_str().unwrap(),
    ]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("does not support overwriting"));

    let read = run(&[
        "block", "read", image.to_str().unwrap(), "0", "--output", output.to_str().unwrap(),
    ]);
    assert!(read.status.success(), "{}", String::from_utf8_lossy(&read.stderr));
    assert_eq!(std::fs::read(&output).unwrap(), b"write-once-data!");

    let _ = std::fs::remove_dir_all(&root);
}
