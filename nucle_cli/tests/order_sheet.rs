//! `nucle hardware order` through the compiled binary: the strands a
//! program stores, as a sheet a vendor's ordering page accepts.

use std::path::PathBuf;
use std::process::Command;

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_cli_order_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn json(cmd: &mut Command) -> serde_json::Value {
    let out = cmd.output().expect("spawn nucle-cli");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("bad JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

/// A program that really stores `records.csv` and only simulates storing
/// `draft.txt`.
fn program(dir: &std::path::Path) -> PathBuf {
    std::fs::write(dir.join("records.csv"), "id,value\n1,42\n2,43\n3,44\n").unwrap();
    std::fs::write(dir.join("draft.txt"), "not for synthesis").unwrap();
    let source = dir.join("order.nsl");
    std::fs::write(
        &source,
        "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }\n\
         store \"records.csv\" into archive\n\
         simulate store \"draft.txt\" into archive { coverage: 3x }\n",
    )
    .unwrap();
    source
}

#[test]
fn a_twist_order_holds_the_stored_strands_and_the_header_that_describes_them() {
    let dir = scratch("twist");
    let source = program(&dir);
    let sheet = dir.join("order.fasta");
    let v = json(nucle().arg("--json").args(["hardware", "order"]).arg(&source).args(["--vendor", "twist", "--out"]).arg(&sheet));
    assert_eq!(v["vendor"], "Twist");

    let text = std::fs::read_to_string(&sheet).unwrap();
    let names: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix('>')).collect();
    assert_eq!(names.len() as u64, v["oligos"].as_u64().unwrap());
    assert!(names.iter().any(|n| n.starts_with("header_")));
    assert!(names.iter().any(|n| n.starts_with("archive_records.csv_d")) && names.iter().any(|n| n.starts_with("archive_records.csv_p")));
    assert!(!names.iter().any(|n| n.contains("draft")), "a simulated store is not ordered: {names:?}");
    assert!(text.lines().filter(|l| !l.starts_with('>')).all(|l| l.chars().all(|c| "ACGT".contains(c))));

    // The sheet alone says what it holds: its header strands give back the
    // file table.
    let header = json(nucle().arg("--json").args(["bootstrap", "inspect"]).arg(&sheet));
    let files: Vec<&str> = header["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(files, vec!["archive/records.csv"]);
}

#[test]
fn an_idt_order_is_a_csv_with_the_scales_that_take_its_size() {
    let dir = scratch("idt");
    let source = program(&dir);
    let sheet = dir.join("order.csv");
    let v = json(nucle().arg("--json").args(["hardware", "order"]).arg(&source).args(["--vendor", "idt", "--out"]).arg(&sheet));
    let oligos = v["oligos"].as_u64().unwrap();
    assert!((2..=384).contains(&oligos), "{oligos}");
    assert_eq!(v["idt_scales"][0], "50 pmol");
    assert!(v["shortest"].as_u64().unwrap() >= 40 && v["longest"].as_u64().unwrap() <= 350);
    let text = std::fs::read_to_string(&sheet).unwrap();
    assert!(text.starts_with("Name,Sequence\n"));
    assert_eq!(text.lines().count() as u64, oligos + 1);
}

#[test]
fn a_sheet_a_vendor_would_refuse_is_not_written() {
    let dir = scratch("refuse");
    let source = program(&dir);

    let fasta = dir.join("order.fasta");
    let out = nucle().args(["hardware", "order"]).arg(&source).args(["--vendor", "idt", "--out"]).arg(&fasta).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Excel uploads only"));
    assert!(!fasta.exists());

    let simulated_only = dir.join("simulated.nsl");
    std::fs::write(&simulated_only, "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }\nsimulate store \"draft.txt\" into archive { coverage: 3x }\n").unwrap();
    let sheet = dir.join("nothing.fasta");
    let out = nucle().args(["hardware", "order"]).arg(&simulated_only).args(["--vendor", "twist", "--out"]).arg(&sheet).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("stores nothing to synthesize"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!sheet.exists());
}
