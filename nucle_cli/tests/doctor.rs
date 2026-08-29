use std::process::Command;

#[test]
fn doctor_verifies_the_complete_pinned_workspace() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("nucle_cli must be a workspace member");
    let output = Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
        .args(["--json", "doctor"])
        .current_dir(workspace)
        .output()
        .expect("failed to run nucle doctor");

    assert!(output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output must be JSON");
    assert_eq!(report["status"], "Healthy");

    let checks = report["checks"].as_array().expect("checks must be an array");
    for expected in ["Pinned source environment", "Workspace crate versions"] {
        let check = checks
            .iter()
            .find(|check| check["name"] == expected)
            .unwrap_or_else(|| panic!("missing doctor check: {expected}"));
        assert_eq!(check["ok"], true, "{expected}: {check}");
        assert_eq!(check["skipped"], false, "{expected}: {check}");
    }
}
