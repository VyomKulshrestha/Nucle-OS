//! `nucle shelf-life` and `nucle simulate --years`, through the real
//! compiled binary (`CARGO_BIN_EXE_nucle-cli`).

use std::path::PathBuf;
use std::process::Command;

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_cli_shelf_life_{}_{}", name, std::process::id()));
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
fn encapsulated_half_life_matches_the_published_figure() {
    let v = json(nucle().args(["--json", "shelf-life", "--medium", "encapsulated", "--celsius", "25", "--length", "150"]));
    let half_life = v["strand_half_life_years"].as_f64().unwrap();
    // Coudy et al. 2021: about 38,000 years for 150 nt at 25 °C.
    assert!((half_life - 38_000.0).abs() < 1_000.0, "{half_life}");
    assert_eq!(v["kinetics_source"], "Coudy et al., PLoS ONE 16(11): e0259868 (2021)");
    assert!(v["shelf_life_years"].as_f64().unwrap() > 0.0);
}

#[test]
fn colder_storage_and_more_parity_extend_shelf_life() {
    let at = |celsius: &str, parity: &str| {
        json(nucle().args(["--json", "shelf-life", "--medium", "bone", "--celsius", celsius, "--parity", parity]))["shelf_life_years"]
            .as_f64()
            .unwrap()
    };
    assert!(at("4", "20") > at("25", "20"));
    assert!(at("25", "40") > at("25", "20"));

    let v = json(nucle().args(["--json", "shelf-life", "--medium", "bone"]));
    let rows = v["by_temperature"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let lives: Vec<f64> = rows.iter().map(|r| r["shelf_life_years"].as_f64().unwrap()).collect();
    assert!(lives.windows(2).all(|w| w[0] > w[1]), "warmer must never last longer: {lives:?}");
}

#[test]
fn shelf_life_reads_strand_counts_from_a_stored_file() {
    let pool = scratch_dir("pool");
    let src = scratch_dir("src");
    let file = src.join("records.csv");
    std::fs::write(&file, b"id,value\n1,42\n2,43\n3,44\n4,45\n").unwrap();
    let store = nucle().arg("--pool-dir").arg(&pool).arg("store").arg(&file).args(["--redundancy", "4"]).output().unwrap();
    assert!(store.status.success(), "{}", String::from_utf8_lossy(&store.stderr));

    let v = json(nucle().arg("--json").arg("--pool-dir").arg(&pool).args(["shelf-life", "--stored", "records.csv"]));
    assert_eq!(v["source"], "stored file 'records.csv'");
    assert!(v["data_strands"].as_u64().unwrap() >= 1);
    assert_eq!(v["parity_strands"].as_u64().unwrap(), 4);

    let missing = nucle().arg("--pool-dir").arg(&pool).args(["shelf-life", "--stored", "nope.csv"]).output().unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("No stored file named 'nope.csv'"));
}

#[test]
fn rejects_unknown_media_and_bad_targets() {
    let bad_medium = nucle().args(["shelf-life", "--medium", "tape"]).output().unwrap();
    assert!(!bad_medium.status.success());
    assert!(String::from_utf8_lossy(&bad_medium.stderr).contains("Unknown storage medium 'tape'"));
    let bad_target = nucle().args(["shelf-life", "--target", "100"]).output().unwrap();
    assert!(!bad_target.status.success());
}

#[test]
fn simulate_years_decays_molecules_and_reports_them() {
    let src = scratch_dir("sim");
    let file = src.join("payload.bin");
    std::fs::write(&file, vec![7u8; 64]).unwrap();
    let fresh = json(nucle().arg("--json").arg("simulate").arg(&file).args(["-p", "pristine", "-c", "20"]));
    assert!(fresh.get("storage").is_none());
    let aged = json(
        nucle()
            .arg("--json")
            .arg("simulate")
            .arg(&file)
            .args(["-p", "pristine", "-c", "20", "--years", "2000", "--celsius", "25", "--medium", "bone"]),
    );
    let decayed = aged["storage"]["decayed_molecules"].as_u64().unwrap();
    assert!(decayed > 0, "2,000 years in bone at 25 °C must break molecules");
    assert!(aged["survival_rate"].as_f64().unwrap() < fresh["survival_rate"].as_f64().unwrap());
}
