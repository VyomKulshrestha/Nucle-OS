//! `nucle restore` through the compiled binary: files rebuilt from reads of
//! an order sheet alone, with no pool directory.

use std::path::{Path, PathBuf};
use std::process::Command;

const PAIR: &str = "ACGTCAGTGCATGACTGACT,TGCATCAGTCAGACTGCATG";

fn nucle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nucle-cli"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_cli_restore_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A flanked order sheet for a pool holding `payload` as `notes.txt`.
fn sheet(dir: &Path, codec: &str, payload: &str) -> Vec<String> {
    std::fs::write(dir.join("notes.txt"), payload).unwrap();
    std::fs::write(dir.join("p.nsl"), format!("pool lab: DnaPool {{ codec: {codec}, redundancy: 4x, profile: Twist }}\nstore \"notes.txt\" into lab\n")).unwrap();
    let out = nucle().args(["hardware", "order"]).arg(dir.join("p.nsl")).args(["--vendor", "twist", "--flank", PAIR, "--out"]).arg(dir.join("order.fasta")).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::read_to_string(dir.join("order.fasta")).unwrap().lines().filter(|l| !l.starts_with('>')).map(str::to_string).collect()
}

/// Three reads of each oligo, shuffled: one exact, one with a base
/// changed, one with a base dropped, as FASTQ.
fn reads(oligos: &[String]) -> String {
    let mut reads = Vec::new();
    for (i, oligo) in oligos.iter().enumerate() {
        let mut changed: Vec<char> = oligo.chars().collect();
        let at = 45 + (i * 7) % 50;
        changed[at] = if changed[at] == 'A' { 'C' } else { 'A' };
        let mut dropped: Vec<char> = oligo.chars().collect();
        dropped.remove(60 + (i * 5) % 40);
        reads.push(oligo.clone());
        reads.push(changed.into_iter().collect());
        reads.push(dropped.into_iter().collect());
    }
    reads.reverse();
    reads.iter().enumerate().map(|(i, r)| format!("@r{i}\n{r}\n+\n{}\n", "I".repeat(r.len()))).collect()
}

#[test]
fn a_hedges_file_comes_back_from_reads_of_its_order_sheet() {
    let dir = scratch("hedges");
    let payload = "Written by NucleScript, read back from its reads alone.\n".repeat(4);
    std::fs::write(dir.join("reads.fastq"), reads(&sheet(&dir, "Hedges", &payload))).unwrap();
    let out = nucle().arg("--json").arg("restore").arg(dir.join("reads.fastq")).args(["--flank", PAIR, "--out"]).arg(dir.join("restored")).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["files"][0]["name"], "lab/notes.txt");
    assert_eq!(v["files"][0]["restored"], true);
    assert_eq!(std::fs::read_to_string(dir.join("restored").join("lab").join("notes.txt")).unwrap(), payload);

    // Without the flanks, the reads do not start with what the pool wrote.
    let bare = nucle().arg("restore").arg(dir.join("reads.fastq")).args(["--out"]).arg(dir.join("bare")).output().unwrap();
    assert!(!bare.status.success());
}

#[test]
fn a_file_in_another_codec_is_named_but_not_written() {
    let dir = scratch("ternary");
    std::fs::write(dir.join("reads.fastq"), reads(&sheet(&dir, "Ternary", "kept in order by the pool directory"))).unwrap();
    let out = nucle().arg("restore").arg(dir.join("reads.fastq")).args(["--flank", PAIR, "--out"]).arg(dir.join("restored")).output().unwrap();
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("lab/notes.txt") && text.contains("only HEDGES files are rebuilt"), "{text}");
    assert!(!dir.join("restored").join("lab").join("notes.txt").exists());
}
