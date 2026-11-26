//! Simulated sequencing reads of an order sheet: a dry run of a wet-lab
//! pilot before anything is synthesized. Every oligo in a FASTA sheet
//! (`nucle hardware order ... --out order.fasta`) goes through a measured
//! channel at the given number of copies, with that channel's coverage
//! spread, and comes out as full-length FASTQ reads in shuffled order, as
//! a sequencer returns them once paired reads are merged.
//!
//! ```text
//! cargo run --release -p nucle_ecc --example pilot_reads -- order.fasta reads.fastq [channel] [copies] [seed]
//! ```
//!
//! `channel` is `array-hifi` (default), `array-lofi`, `twist-illumina` or
//! `twist-nanopore`; `copies` defaults to 10. Then
//! `decode reads "reads.fastq" against "order.fasta" { blind }` scores the
//! dry run the way it will score the real run (see scripts/pilot/).

use nucle_codec::base::StrandCollection;
use nucle_codec::stable_rng::StableRng;
use nucle_ecc::realdata::parse_sequences;
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: pilot_reads <order.fasta> <reads.fastq> [array-hifi|array-lofi|twist-illumina|twist-nanopore] [copies] [seed]");
        std::process::exit(2);
    }
    let channel = args.get(3).map(String::as_str).unwrap_or("array-hifi");
    let profile = match channel {
        "array-hifi" => HardwareProfile::ArrayHighFidelity,
        "array-lofi" => HardwareProfile::ArrayLowFidelity,
        "twist-illumina" => HardwareProfile::Illumina,
        "twist-nanopore" => HardwareProfile::OxfordNanopore,
        other => {
            eprintln!("unknown channel '{other}'");
            std::process::exit(2);
        }
    };
    let copies: u32 = args.get(4).map(|c| c.parse().expect("copies is a whole number")).unwrap_or(10);
    let seed: u64 = args.get(5).map(|s| s.parse().expect("seed is a whole number")).unwrap_or(2026);

    let text = std::fs::read_to_string(&args[1]).expect("read the order sheet");
    let (oligos, _) = parse_sequences(&text, false).expect("parse the order sheet");
    let collection = StrandCollection { strands: oligos.clone(), original_size: 0 };
    let result = NoiseEngine::new(SimulationConfig::realistic(profile, copies, seed)).simulate(&collection);

    let mut reads: Vec<String> = result.pool.strands.iter().filter(|s| s.is_intact).map(|s| s.sequence.to_string()).collect();
    let mut rng = StableRng::seed_from_u64(seed ^ 0x5eed);
    for i in (1..reads.len()).rev() {
        let j = rng.gen_range(0..i + 1);
        reads.swap(i, j);
    }
    let mut fastq = String::new();
    for (i, read) in reads.iter().enumerate() {
        fastq.push_str(&format!("@dry_run_{i}\n{read}\n+\n{}\n", "I".repeat(read.len())));
    }
    std::fs::write(&args[2], fastq).expect("write the reads");
    println!(
        "{} oligos through {channel} at {copies} copies each: {} reads written to {}, {} copies lost before sequencing.",
        oligos.len(),
        reads.len(),
        args[2],
        result.pool.strands.len() - reads.len()
    );
}
