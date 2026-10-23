//! `nucle cost` through the compiled binary.

use std::path::PathBuf;
use std::process::Command;

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_cli_cost_boot_{}_{}", name, std::process::id()));
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
fn cost_reflects_the_real_encoding_and_the_stated_prices() {
    let dir = scratch("cost");
    let file = dir.join("payload.bin");
    std::fs::write(&file, vec![0x5Au8; 4096]).unwrap();
    let v = json(nucle().arg("--json").arg("cost").arg(&file).args(["-c", "ternary", "-r", "2"]));
    let nt = v["nucleotides"].as_f64().unwrap();
    assert!(nt > 4096.0 * 4.0, "{nt} nt for 4 KiB");
    assert!((v["synthesis_usd"].as_f64().unwrap() - nt * 0.00015).abs() < 1e-6);
    let bits = v["bits_per_nt"].as_f64().unwrap();
    assert!(bits > 0.5 && bits < 2.0, "{bits}");

    let doubled = json(nucle().arg("--json").arg("cost").arg(&file).args(["--synthesis-usd-per-nt", "0.0003"]));
    assert!((doubled["synthesis_usd"].as_f64().unwrap() - 2.0 * v["synthesis_usd"].as_f64().unwrap()).abs() < 1e-6);

    let more_parity = json(nucle().arg("--json").arg("cost").arg(&file).args(["-r", "6"]));
    assert!(more_parity["nucleotides"].as_f64().unwrap() > nt);

    let bad = nucle().arg("cost").arg(&file).args(["-c", "morse"]).output().unwrap();
    assert!(!bad.status.success());
}
