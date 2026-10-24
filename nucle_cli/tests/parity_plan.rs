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

#[test]
fn store_and_migrate_plan_parity_for_the_channel_they_name() {
    let dir = scratch("store");
    let pool = dir.join("pool");
    let file = dir.join("payload.bin");
    std::fs::write(&file, (0..256u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<u8>>()).unwrap();
    let run = |args: &[&str]| {
        let out = nucle().arg("--pool-dir").arg(&pool).arg("--json").args(args).output().expect("spawn nucle-cli");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        (value, String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let path = file.to_str().unwrap();

    // Without a channel the default stays two parity strands.
    let (plain, _) = run(&["store", path, "-c", "hedges"]);
    assert_eq!(plain["parity_strand_count"], 2);
    assert!(plain.get("parity_plan").is_none());

    // Named, the parity is planned for it and the file still comes back.
    let (planned, warnings) = run(&["store", path, "-c", "hedges", "--channel", "array-lofi", "--coverage", "3"]);
    let parity = planned["parity_plan"]["parity"].as_u64().unwrap();
    assert!(parity > 2, "{planned}");
    assert_eq!(planned["parity_strand_count"].as_u64().unwrap(), parity);
    assert!(warnings.is_empty(), "{warnings}");

    // An explicit -r below the plan is honoured, with a warning.
    let (short, warnings) = run(&["store", path, "-c", "hedges", "-r", "2", "--channel", "array-lofi", "--coverage", "3"]);
    assert_eq!(short["parity_strand_count"], 2);
    assert!(warnings.contains("lose this file") && warnings.contains(&format!("the plan is {parity}")), "{warnings}");

    // Migrate plans the same way, from the stored file's own bytes.
    let name = planned["filename"].as_str().unwrap().to_string();
    let (migrated, _) = run(&["migrate", &name, "--channel", "array-hifi", "--coverage", "3"]);
    let hifi = migrated["parity_plan"]["parity"].as_u64().unwrap();
    assert!(hifi > 0 && hifi < parity, "{migrated}");
    assert_eq!(migrated["redundancy"].as_u64().unwrap(), hifi);

    // --coverage alone means nothing without a channel.
    assert!(!nucle().arg("--pool-dir").arg(&pool).args(["store", path, "--coverage", "3"]).output().unwrap().status.success());
}

#[test]
fn store_points_at_hedges_when_ternary_cannot_hold_up() {
    let dir = scratch("ternary");
    let file = dir.join("payload.bin");
    std::fs::write(&file, (0..256u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<u8>>()).unwrap();
    let out = nucle()
        .arg("--pool-dir").arg(dir.join("pool"))
        .args(["store", file.to_str().unwrap(), "-c", "ternary", "--channel", "array-lofi", "--coverage", "6"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--codec hedges"), "{stderr}");
}
