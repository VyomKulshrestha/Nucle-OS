//! `nucle reads`: score the decoder on real sequencing reads.
//!
//! Takes a references file and a clustered-reads file in the layout of
//! Microsoft's Clustered Nanopore Reads dataset (see
//! `nucle_ecc::realdata`) and reports how well consensus recovers each
//! reference, first with the dataset's own cluster labels and, with
//! `--blind`, again after discarding them and clustering the reads from
//! scratch.

use nucle_ecc::cluster::ClusterConfig;
use nucle_ecc::consensus::build_consensus_with_length;
use nucle_ecc::realdata::{evaluate_blind, evaluate_with, parse_cnr};

pub fn cmd_reads(centers: &str, clusters: &str, limit: Option<usize>, blind: bool, json: bool) {
    let read = |path: &str| {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Error reading '{}': {}", path, e);
            std::process::exit(1);
        })
    };
    let mut data = parse_cnr(&read(centers), &read(clusters)).unwrap_or_else(|e| {
        eprintln!("Not a clustered-reads dataset: {}", e);
        std::process::exit(1);
    });
    if let Some(n) = limit {
        data.truncate(n);
    }

    let report = evaluate_with(&data, |reads, len| build_consensus_with_length(reads, len).map(|r| r.sequence));
    // The merge threshold comes from the reads alone, like everything else
    // in the blind run.
    let config = blind.then(|| {
        let pooled: Vec<_> = data.iter().flat_map(|c| c.reads.iter().cloned()).collect();
        ClusterConfig::fitted(&pooled)
    });
    let blind_report = config.as_ref().map(|config| evaluate_blind(&data, config, 2021));

    if json {
        let mut out = serde_json::json!({
            "clusters": report.clusters,
            "empty_clusters": report.empty_clusters,
            "reads": report.reads,
            "raw_read_error_rate": report.raw_read_error_rate,
            "consensus_error_rate": report.consensus_error_rate(),
            "exact": report.exact,
            "exact_rate": report.exact_rate(),
        });
        if let (Some(b), Some(config)) = (&blind_report, &config) {
            out["blind"] = serde_json::json!({
                "merge_threshold": config.max_distance_fraction,
                "reads": b.reads,
                "references": b.references,
                "clusters_found": b.clusters,
                "multi_read_clusters": b.multi_read_clusters,
                "purity": b.purity,
                "recovered_exact": b.recovered_exact,
                "recovery_rate": b.recovery_rate(),
            });
        }
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("== with the dataset's cluster labels (vote + length-aware polish)");
    print!("{report}");
    if let (Some(b), Some(config)) = (blind_report, config) {
        println!();
        println!("== labels discarded: clustered from the reads alone");
        println!("merge threshold:        {:.3} of strand length (fitted to the reads)", config.max_distance_fraction);
        print!("{b}");
    }
}
