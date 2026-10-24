//! `nucle parity` through the compiled binary.

use std::path::PathBuf;
use std::process::Command;

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_cli_parity_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn json(cmd: &mut Command) -> serde_json::Value {
    let out = cmd.output().expect("spawn nucle-cli");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("bad JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn parity_plans_past_the_store_default_for_low_fidelity_reads_at_three_per_strand() {
    let dir = scratch("lofi");
    let file = dir.join("payload.bin");
    std::fs::write(&file, (0..1024u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<u8>>()).unwrap();
    let lofi = ["-c", "hedges", "-p", "array-lofi", "--coverage", "3"];

    let v = json(nucle().arg("--json").arg("parity").arg(&file).args(lofi));
    assert_eq!(v["data_strands"], 64);
    let unread = v["unread"].as_f64().unwrap();
    assert!((0.17..0.30).contains(&unread), "{unread}");
    let parity = v["parity"].as_u64().unwrap();
    assert!((30..60).contains(&parity), "{parity}");
    assert!(v["failure_probability"].as_f64().unwrap() <= 0.001);
    assert!(v["default_failure_probability"].as_f64().unwrap() > 0.99);

    let text = nucle().arg("parity").arg(&file).args(lofi).output().unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains(&format!("--redundancy {parity}")), "{text}");

    // A clean channel reads every strand; the trial bound alone keeps a
    // little parity.
    let clean = json(nucle().arg("--json").arg("parity").arg(&file).args(["-c", "hedges", "-p", "pristine", "--coverage", "1"]));
    assert_eq!(clean["unread"], 0.0);
    assert!(clean["parity"].as_u64().unwrap() < parity / 3, "{clean}");

    assert!(!nucle().arg("parity").arg(&file).args(["--max-failure", "0"]).output().unwrap().status.success());
    assert!(!nucle().arg("parity").arg(&file).args(["-c", "morse"]).output().unwrap().status.success());
    assert!(!nucle().arg("parity").arg(&file).args(["--coverage", "0"]).output().unwrap().status.success());
}
