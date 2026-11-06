//! The NucleScript language version the CLI implements, as `--version` and
//! `check --json` report it.

use std::process::Command;

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

#[test]
fn the_cli_reports_the_language_version_it_implements() {
    let version = nucle().arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains(&format!("NucleScript {}", nucle_lang::LANGUAGE_VERSION)));

    let minimal = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples").join("minimal.nsl");
    let out = nucle().args(["--json", "check"]).arg(&minimal).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["language"], nucle_lang::LANGUAGE_VERSION);
}
