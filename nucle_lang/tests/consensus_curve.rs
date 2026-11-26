//! The error rate `consensus_vote` gives its result comes from consensus
//! decoding measured on real reads (`nucle_lang::probabilistic`): the
//! curve never rises with coverage, covers every measured point, matches
//! what the decoder does today on the MinION sample in the repository,
//! and is what the type checker infers.

use nucle_ecc::realdata::{evaluate_published, parse_cnr, ReadCluster};
use nucle_lang::check_source;
use nucle_lang::probabilistic::{consensus_error_rate_percent, consensus_residual_ratio, MEASURED_CONSENSUS};
use std::path::Path;

fn diagnostics(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| format!("{}: {}", d.code, d.message)).collect()
}

#[test]
fn one_read_leaves_the_rate_unchanged() {
    assert_eq!(consensus_residual_ratio(0), 1.0);
    assert_eq!(consensus_residual_ratio(1), 1.0);
    assert_eq!(consensus_error_rate_percent(5.0, 1), 5.0);
}

#[test]
fn more_coverage_never_raises_the_rate() {
    for c in 1..200 {
        assert!(
            consensus_residual_ratio(c + 1) <= consensus_residual_ratio(c),
            "{}x gives {} but {}x gives {}",
            c + 1,
            consensus_residual_ratio(c + 1),
            c,
            consensus_residual_ratio(c)
        );
    }
}

#[test]
fn two_reads_already_lower_the_rate() {
    // Every shipped program that asserts `recovered < noisy` relies on this.
    assert!(consensus_residual_ratio(2) < 1.0);
}

#[test]
fn the_curve_is_at_least_every_measured_point() {
    for set in MEASURED_CONSENSUS {
        assert!(!set.points.is_empty(), "{} has no points", set.reads);
        assert!(set.points.windows(2).all(|w| w[0].0 < w[1].0), "{} is not sorted by coverage", set.reads);
        for &(c, ratio) in set.points.iter().filter(|(c, _)| *c >= 2) {
            assert!(consensus_residual_ratio(c) >= ratio, "{}x: curve {} below {} measured on {}", c, consensus_residual_ratio(c), ratio, set.reads);
        }
    }
}

#[test]
fn coverage_between_or_past_the_measurements_takes_the_lower_point() {
    // 7x was never measured, so it may claim no more than 6x did; 100x
    // claims no more than the highest coverage measured.
    assert_eq!(consensus_residual_ratio(7), consensus_residual_ratio(6));
    let last = MEASURED_CONSENSUS.iter().flat_map(|s| s.points.iter().map(|(c, _)| *c)).max().unwrap();
    assert_eq!(consensus_residual_ratio(100), consensus_residual_ratio(last));
}

/// docs/architecture.md and docs/stdlib.md quote these shares.
#[test]
fn the_documented_shares_hold() {
    let shares: Vec<(usize, f64)> = [2, 3, 5, 10, 20, 30].iter().map(|&c| (c, consensus_residual_ratio(c))).collect();
    assert_eq!(shares, vec![(2, 0.92787), (3, 0.25248), (5, 0.07890), (10, 0.03670), (20, 0.02104), (30, 0.02104)]);
    assert!((consensus_error_rate_percent(5.0, 10) - 0.1835).abs() < 1e-9);
    assert!((consensus_error_rate_percent(0.35, 10) - 0.012845).abs() < 1e-9);
}

#[test]
fn the_minion_points_match_the_decoder_on_the_shipped_sample() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nucle_ecc/tests/fixtures/cnr");
    let centers = std::fs::read_to_string(dir.join("Centers.txt")).unwrap();
    let clusters = std::fs::read_to_string(dir.join("Clusters.txt")).unwrap();
    let data = parse_cnr(&centers, &clusters).unwrap();
    let minion = &MEASURED_CONSENSUS[0];
    assert!(minion.reads.contains("MinION"));
    let raw = evaluate_published(&data).raw_read_error_rate;
    assert!((raw * 100.0 - minion.raw_error_percent).abs() < 5e-5, "raw {}", raw * 100.0);
    for &(c, ratio) in minion.points {
        let subset: Vec<ReadCluster> = data
            .iter()
            .filter(|cluster| cluster.reads.len() >= c)
            .map(|cluster| ReadCluster { reference: cluster.reference.clone(), reads: cluster.reads[..c].to_vec() })
            .collect();
        assert!(subset.len() >= 30, "{c}x has {} strands", subset.len());
        let measured = evaluate_published(&subset).consensus_error_rate() / raw;
        assert!((measured - ratio).abs() < 5e-5, "{c}x: decoder gives {measured}, table says {ratio}");
    }
}

#[test]
fn the_checker_infers_the_measured_rate() {
    let rate = consensus_error_rate_percent(5.0, 10);
    let program = |annotated: f64| {
        format!(
            "pool archive: DnaPool {{ codec: Ternary, redundancy: 3x, profile: Nanopore }}
let noisy: Pool<Nanopore, 5%> = simulate archive under Nanopore
let recovered: Pool<Recovered, {annotated:.4}%> = consensus_vote(noisy, coverage: 10x)
"
        )
    };
    let measured = diagnostics(&program(rate));
    assert!(!measured.iter().any(|d| d.starts_with("E-BINDING-ERROR-RATE-MISMATCH")), "{measured:?}");
    // The old rule, rate / coverage², promised 0.05%; the MinION reads
    // kept 3.7% of their error at 10 reads per strand, 0.18% of 5%.
    let old = diagnostics(&program(0.05));
    assert!(old.iter().any(|d| d.starts_with("E-BINDING-ERROR-RATE-MISMATCH")), "{old:?}");
}
