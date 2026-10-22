//! Consensus calling scored on real Oxford Nanopore MinION reads.
//!
//! The fixture is the first 50 clusters of Microsoft's Clustered Nanopore
//! Reads dataset (see `tests/fixtures/cnr/README.md`): Twist-synthesized
//! 110 nt strands with their real sequencing reads. Unlike every simulator
//! test in this crate, the noise here came from an instrument.

use nucle_ecc::consensus::{build_consensus, build_consensus_with_length};
use nucle_ecc::pipeline::vote_groups;
use nucle_ecc::realdata::{evaluate_clusters, evaluate_with, parse_cnr, ReadCluster};

fn fixture() -> Vec<ReadCluster> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cnr");
    let centers = std::fs::read_to_string(format!("{dir}/Centers.txt")).unwrap();
    let clusters = std::fs::read_to_string(format!("{dir}/Clusters.txt")).unwrap();
    parse_cnr(&centers, &clusters).unwrap()
}

#[test]
fn fixture_is_the_documented_sample() {
    let data = fixture();
    assert_eq!(data.len(), 50);
    assert!(data.iter().all(|c| c.reference.len() == 110));
    let reads: usize = data.iter().map(|c| c.reads.len()).sum();
    assert!(reads > 1000, "expected the real read clusters, got {reads} reads");
}

#[test]
fn raw_reads_carry_real_nanopore_error_rates() {
    let report = evaluate_clusters(&fixture());
    // MinION R9-era reads of short synthetic strands: several percent error.
    assert!(
        report.raw_read_error_rate > 0.03 && report.raw_read_error_rate < 0.10,
        "raw error {}",
        report.raw_read_error_rate
    );
}

#[test]
fn length_aware_polishing_beats_plain_voting_on_real_reads() {
    let data = fixture();
    let voted = evaluate_with(&data, |reads, _| build_consensus(reads).map(|r| r.sequence));
    let polished = evaluate_with(&data, |reads, len| build_consensus_with_length(reads, len).map(|r| r.sequence));

    assert!(
        polished.exact > voted.exact,
        "length-aware: {} exact, vote only: {} exact",
        polished.exact,
        voted.exact
    );
    assert!(polished.consensus_edit_sum < voted.consensus_edit_sum);
    assert!(polished.exact_rate() >= 0.9, "exact rate {}", polished.exact_rate());
}

#[test]
fn pipeline_voting_infers_the_strand_length_from_the_group_majority() {
    // vote_groups is what the RS decode paths call; it is never told the
    // strand length and has to infer 110 from the groups themselves.
    let data = fixture();
    let groups: Vec<_> = data.iter().map(|c| c.reads.clone()).collect();
    let voted = vote_groups(&groups);
    let exact = voted
        .iter()
        .zip(&data)
        .filter(|(v, c)| v.as_ref().is_some_and(|r| r.sequence == c.reference))
        .count();
    let off_length = voted.iter().flatten().filter(|r| r.sequence.len() != 110).count();
    assert_eq!(off_length, 0, "every polished strand should be 110 nt");
    assert!(exact >= 45, "{exact} / 50 exact");
}

#[test]
fn blind_decoding_without_cluster_labels_recovers_nearly_every_strand() {
    // Throw the cluster labels away, shuffle all reads together, and
    // decode from the reads alone: cluster, then length-aware consensus.
    let data = fixture();
    let labelled = evaluate_with(&data, |reads, len| build_consensus_with_length(reads, len).map(|r| r.sequence));
    let blind = nucle_ecc::realdata::evaluate_blind(&data, &nucle_ecc::cluster::ClusterConfig::default(), 2021);
    assert!(blind.purity > 0.99, "purity {}", blind.purity);
    assert!(
        blind.recovered_exact + 2 >= labelled.exact,
        "blind {} vs labelled {}",
        blind.recovered_exact,
        labelled.exact
    );
}
