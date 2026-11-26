//! How much per-base error a consensus leaves as the number of reads per
//! strand grows, measured on real reads in the CNR layout (`Centers.txt`,
//! `Clusters.txt`): for each coverage `c`, every strand with at least `c`
//! reads is decoded from its first `c` reads alone, with the published
//! decoder (partial-order vote, then median polish to the strand length).
//!
//! ```text
//! cargo run --release -p nucle_ecc --example coverage_curve -- <Centers.txt> <Clusters.txt>
//! ```
//!
//! The table it prints is what `nucle_lang::probabilistic` fits the
//! `consensus_vote` error rate to; see docs/architecture.md.

use nucle_ecc::realdata::{evaluate_published, parse_cnr, ReadCluster};

const COVERAGES: [usize; 13] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 25, 30];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: coverage_curve <Centers.txt> <Clusters.txt>");
        std::process::exit(2);
    }
    let centers = std::fs::read_to_string(&args[1]).expect("read centers");
    let clusters = std::fs::read_to_string(&args[2]).expect("read clusters");
    let data = parse_cnr(&centers, &clusters).expect("parse dataset");

    let raw = evaluate_published(&data).raw_read_error_rate;
    println!("raw read error {:.4}%", raw * 100.0);
    println!("coverage\tstrands\tedits\tbases\tresidual %\tresidual / raw");
    for c in COVERAGES {
        let subset: Vec<ReadCluster> = data
            .iter()
            .filter(|cluster| cluster.reads.len() >= c)
            .map(|cluster| ReadCluster { reference: cluster.reference.clone(), reads: cluster.reads[..c].to_vec() })
            .collect();
        if subset.is_empty() {
            continue;
        }
        let report = evaluate_published(&subset);
        let residual = report.consensus_error_rate();
        println!(
            "{c}\t{}\t{}\t{}\t{:.5}\t{:.5}",
            subset.len(),
            report.consensus_edit_sum,
            report.reference_bases,
            residual * 100.0,
            residual / raw
        );
    }
}
