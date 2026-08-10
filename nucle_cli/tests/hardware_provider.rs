//! Provider selection must fail closed. An unknown or library-only vendor
//! must never be silently changed into a local file export.

use std::path::PathBuf;
use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_output(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nucle_cli_hardware_provider_test_{}_{}_export.json",
        name,
        std::process::id()
    ))
}

const PURE_PROGRAM: &str = "../docs/examples/probabilistic_recovery.nsl";

fn assert_provider_rejected(provider: &str, output: &PathBuf) -> String {
    let _ = std::fs::remove_file(output);
    let result = Command::new(nucle_cli_bin())
        .arg("hardware")
        .arg("export")
        .arg(PURE_PROGRAM)
        .arg("--provider")
        .arg(provider)
        .arg("--output")
        .arg(output)
        .output()
        .expect("failed to spawn nucle-cli");

    assert!(!result.status.success());
    assert!(!output.exists(), "a rejected provider must not create a file export");
    String::from_utf8_lossy(&result.stderr).into_owned()
}

#[test]
fn unknown_provider_is_rejected_without_creating_an_export() {
    let output = scratch_output("unknown");
    let stderr = assert_provider_rejected("typo-provider", &output);
    assert!(stderr.contains("provider 'typo-provider' is not available through the CLI"), "got: {stderr}");
}

#[test]
fn library_only_vendor_is_rejected_without_creating_an_export() {
    let output = scratch_output("vendor");
    let stderr = assert_provider_rejected("twist", &output);
    assert!(stderr.contains("resumable CLI job handling"), "got: {stderr}");
}
