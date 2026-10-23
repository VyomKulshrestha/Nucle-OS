//! `nucle cost` and `nucle bootstrap export|inspect` through the compiled
//! binary.

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

    // Read-side carbon follows the sequencer's published kg CO2e per Gbase;
    // life-cycle carbon follows the synthesis chemistry.
    let gbase = nt * 10.0 / 1e9;
    assert!((v["sequencing_kg_co2e"].as_f64().unwrap() - gbase * 0.04).abs() < 1e-15);
    let miseq = json(nucle().arg("--json").arg("cost").arg(&file).args(["--sequencer", "miseq"]));
    assert!((miseq["sequencing_kg_co2e"].as_f64().unwrap() - gbase * 3.81).abs() < 1e-12);
    let enzymatic = json(nucle().arg("--json").arg("cost").arg(&file).args(["--chemistry", "enzymatic"]));
    assert!(enzymatic["lifecycle_kg_co2e_per_year"].as_f64().unwrap() < v["lifecycle_kg_co2e_per_year"].as_f64().unwrap());
    assert!(!nucle().arg("cost").arg(&file).args(["--sequencer", "hiseq"]).output().unwrap().status.success());
}

#[test]
fn a_pool_exported_with_its_header_describes_itself() {
    let dir = scratch("boot");
    let pool = dir.join("pool");
    for (name, body) in [("records.csv", &b"id,value\n1,42\n2,43\n"[..]), ("notes.txt", &b"remember the primers"[..])] {
        let f = dir.join(name);
        std::fs::write(&f, body).unwrap();
        let out = nucle().arg("--pool-dir").arg(&pool).arg("store").arg(&f).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let fasta = dir.join("archive.fasta");
    let exported = json(nucle().arg("--json").arg("--pool-dir").arg(&pool).args(["bootstrap", "export"]).arg(&fasta));
    assert!(exported["header_strands"].as_u64().unwrap() >= 2);

    // Shuffle the dump so the header is buried among data strands.
    let text = std::fs::read_to_string(&fasta).unwrap();
    let mut records: Vec<String> = text.split('>').filter(|r| !r.is_empty()).map(|r| format!(">{r}")).collect();
    records.reverse();
    let shuffled = dir.join("shuffled.fasta");
    std::fs::write(&shuffled, records.concat()).unwrap();

    let header = json(nucle().arg("--json").args(["bootstrap", "inspect"]).arg(&shuffled));
    let names: Vec<&str> = header["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["notes.txt", "records.csv"]);
    assert_eq!((header["files"][0]["size"].as_u64(), header["files"][1]["size"].as_u64()), (Some(20), Some(19)));

    let junk = dir.join("junk.fasta");
    std::fs::write(&junk, ">a\nACGTACGTACGT\n").unwrap();
    let out = nucle().args(["bootstrap", "inspect"]).arg(&junk).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("No usable archive header"));
}
