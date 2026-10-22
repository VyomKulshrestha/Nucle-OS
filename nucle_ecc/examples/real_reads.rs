//! Score consensus calling against real nanopore reads.
//!
//! Reads the Clustered Nanopore Reads (CNR) dataset format published with
//! Srinivasavaradhan et al., "Trellis BMA: Coded trace reconstruction on IDS
//! channels for DNA storage" (ISIT 2021,
//! <https://github.com/microsoft/clustered-nanopore-reads-dataset>):
//! `Centers.txt` holds one reference strand per line and `Clusters.txt`
//! holds the real MinION reads of each reference, one cluster per block,
//! every block preceded by a line of `=` characters.
//!
//! ```text
//! cargo run --release -p nucle_ecc --example real_reads -- <Centers.txt> <Clusters.txt> [max_clusters]
//! ```
//!
//! A trimmed 60-cluster sample ships in `nucle_ecc/tests/fixtures/cnr/`.

use nucle_ecc::consensus::{build_consensus, build_consensus_with_length};
use nucle_ecc::polish::polish;
use nucle_ecc::realdata::{evaluate_clusters, evaluate_with, parse_cnr};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: real_reads <Centers.txt> <Clusters.txt> [max_clusters]");
        std::process::exit(2);
    }
    let centers = std::fs::read_to_string(&args[1]).expect("read centers");
    let clusters = std::fs::read_to_string(&args[2]).expect("read clusters");
    let limit = args.get(3).and_then(|s| s.parse::<usize>().ok());

    let mut dataset = parse_cnr(&centers, &clusters).expect("parse CNR dataset");
    if let Some(n) = limit {
        dataset.truncate(n);
    }

    let blind_only = std::env::var_os("NUCLE_BLIND_ONLY").is_some();
    let start = Instant::now();
    let report = if blind_only { evaluate_with(&[], |_, _| None) } else { evaluate_clusters(&dataset) };
    let elapsed = start.elapsed();
    println!("== partial-order vote only");
    println!("{report}");
    println!("elapsed: {:.2}s\n", elapsed.as_secs_f64());

    let start = Instant::now();
    let report = evaluate_with(if blind_only { &[] } else { &dataset }, |reads, _| {
        build_consensus(reads).map(|r| polish(&r.sequence, reads, None))
    });
    println!("== vote + median polish (length unknown)");
    println!("{report}");
    println!("elapsed: {:.2}s\n", start.elapsed().as_secs_f64());

    let start = Instant::now();
    let report = evaluate_with(if blind_only { &[] } else { &dataset }, |reads, len| {
        build_consensus_with_length(reads, len).map(|r| r.sequence)
    });
    println!("== vote + length-aware median polish");
    println!("{report}");
    println!("elapsed: {:.2}s", start.elapsed().as_secs_f64());

    let start = Instant::now();
    let mut config = nucle_ecc::cluster::ClusterConfig::default();
    if let Some(frac) = std::env::var("NUCLE_CLUSTER_FRAC").ok().and_then(|v| v.parse().ok()) {
        config.max_distance_fraction = frac;
    }
    let blind = nucle_ecc::realdata::evaluate_blind(&dataset, &config, 2021);
    println!();
    println!("== labels discarded: cluster, then length-aware consensus");
    println!("{blind}");
    println!("elapsed: {:.2}s", start.elapsed().as_secs_f64());

    if std::env::var_os("NUCLE_DIAG").is_some() {
        let mut shown = 0;
        for c in dataset.iter().filter(|c| c.reads.len() >= 21) {
            let Some(r) = nucle_ecc::consensus::build_consensus(&c.reads) else { continue };
            let d = nucle_ecc::realdata::edit_distance(r.sequence.bases(), c.reference.bases());
            if d == 0 {
                continue;
            }
            println!("--- {} reads, dist {}, len {} vs {}", c.reads.len(), d, r.sequence.len(), c.reference.len());
            println!("ref {}", c.reference);
            println!("got {}", r.sequence);
            shown += 1;
            if shown == 12 {
                break;
            }
        }
    }
}
