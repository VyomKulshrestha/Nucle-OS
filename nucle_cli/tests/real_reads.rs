//! `nucle reads` on the checked-in sample of real MinION reads
//! (`nucle_ecc/tests/fixtures/cnr/`), through the compiled binary.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nucle_ecc/tests/fixtures/cnr").join(name)
}

#[test]
fn scores_real_reads_with_and_without_labels() {
    let out = Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
        .arg("--json")
        .arg("reads")
        .arg(fixture("Centers.txt"))
        .arg(fixture("Clusters.txt"))
        .arg("--blind")
        .output()
        .expect("spawn nucle-cli");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["clusters"], 50);
    let raw = v["raw_read_error_rate"].as_f64().unwrap();
    assert!(raw > 0.03 && raw < 0.10, "raw error {raw}");
    assert!(v["exact_rate"].as_f64().unwrap() >= 0.9);
    assert!(v["blind"]["purity"].as_f64().unwrap() > 0.99);
    assert!(v["blind"]["recovered_exact"].as_u64().unwrap() >= 43);
}

#[test]
fn limit_and_text_output() {
    let out = Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
        .arg("reads")
        .arg(fixture("Centers.txt"))
        .arg(fixture("Clusters.txt"))
        .args(["--limit", "5"])
        .output()
        .expect("spawn nucle-cli");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("clusters scored:        5"), "{text}");
    assert!(!text.contains("labels discarded"), "blind section only with --blind");
}

#[test]
fn rejects_files_that_are_not_a_clustered_dataset() {
    let out = Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
        .arg("reads")
        .arg(fixture("Centers.txt"))
        .arg(fixture("README.md"))
        .output()
        .expect("spawn nucle-cli");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Not a clustered-reads dataset"));
}
