//! Why blind decoding misses the references it misses.
//!
//! ```text
//! cargo run --release -p nucle_ecc --example blind_breakdown -- <Centers.txt> <Clusters.txt> [max_clusters]
//! ```
//!
//! Clusters every read with its label hidden, takes the length-aware
//! consensus of each cluster as `evaluate_blind` does, and sorts each
//! reference that no cluster reproduced into one cause: its consensus fails
//! even with the true labels, its sequence is a tandem repeat, its reads
//! were outvoted inside a cluster dominated by another reference, or its
//! reads were split so that no fragment voted to it.

use nucle_codec::base::DnaStrand;
use nucle_ecc::cluster::{cluster_reads, ClusterConfig};
use nucle_ecc::consensus::build_consensus_with_length;
use nucle_ecc::realdata::{parse_cnr, MAX_CONSENSUS_READS};
use std::collections::{HashMap, HashSet};

/// Whether `s` holds a perfect tandem repeat of period 1 to 10 spanning at
/// least 20 bases.
fn tandem_repeat(s: &DnaStrand) -> bool {
    let b = s.bases();
    (1..=10).any(|p| {
        let mut run = 0;
        (0..b.len().saturating_sub(p)).any(|i| {
            run = if b[i] == b[i + p] { run + 1 } else { 0 };
            run + p >= 20
        })
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: blind_breakdown <Centers.txt> <Clusters.txt> [max_clusters]");
        std::process::exit(2);
    }
    let centers = std::fs::read_to_string(&args[1]).expect("read Centers");
    let clusters = std::fs::read_to_string(&args[2]).expect("read Clusters");
    let mut data = parse_cnr(&centers, &clusters).expect("parse CNR dataset");
    if let Some(n) = args.get(3).and_then(|s| s.parse::<usize>().ok()) {
        data.truncate(n);
    }
    let len = {
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for c in &data {
            *counts.entry(c.reference.len()).or_insert(0) += 1;
        }
        counts.into_iter().max_by_key(|&(l, n)| (n, l)).map(|(l, _)| l).unwrap_or(0)
    };

    let mut labels = Vec::new();
    let mut reads = Vec::new();
    for (label, c) in data.iter().enumerate() {
        for r in &c.reads {
            labels.push(label);
            reads.push(r.clone());
        }
    }
    let mut config = ClusterConfig::fitted(&reads);
    if let Some(frac) = std::env::var("NUCLE_CLUSTER_FRAC").ok().and_then(|v| v.parse().ok()) {
        config.max_distance_fraction = frac;
    }
    println!("merge threshold: {:.3}", config.max_distance_fraction);
    let groups = cluster_reads(&reads, &config);

    let mut recovered: HashSet<DnaStrand> = HashSet::new();
    // For each reference: (cluster holding most of its reads, how many).
    let mut home: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut majority: Vec<usize> = Vec::with_capacity(groups.len());
    for (g, members) in groups.iter().enumerate() {
        let group: Vec<DnaStrand> = members.iter().take(MAX_CONSENSUS_READS).map(|&i| reads[i].clone()).collect();
        if let Some(r) = build_consensus_with_length(&group, len) {
            recovered.insert(r.sequence);
        }
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for &i in members {
            *counts.entry(labels[i]).or_insert(0) += 1;
        }
        majority.push(counts.iter().max_by_key(|&(l, n)| (*n, std::cmp::Reverse(*l))).map(|(l, _)| *l).unwrap_or(0));
        for (label, n) in counts {
            let best = home.entry(label).or_insert((g, 0));
            if n > best.1 {
                *best = (g, n);
            }
        }
    }

    let (mut total, mut missed) = (0usize, 0usize);
    let (mut labelled_fails, mut repeats, mut outvoted, mut split, mut other) = (0, 0, 0, 0, 0);
    for (label, c) in data.iter().enumerate() {
        if c.reads.is_empty() {
            continue;
        }
        total += 1;
        if recovered.contains(&c.reference) {
            continue;
        }
        missed += 1;
        let labelled = build_consensus_with_length(&c.reads, len).map(|r| r.sequence);
        let (g, n) = home[&label];
        if labelled.as_ref() != Some(&c.reference) {
            labelled_fails += 1;
        } else if tandem_repeat(&c.reference) {
            repeats += 1;
        } else if majority[g] != label {
            outvoted += 1;
        } else if 2 * n <= c.reads.len() {
            split += 1;
        } else {
            other += 1;
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / total.max(1) as f64;
    println!("references with reads: {total}; clusters: {}", groups.len());
    println!("recovered blind:       {} ({:.2}%)", total - missed, pct(total - missed));
    println!("missed:                {missed} ({:.2}%)", pct(missed));
    println!("  consensus fails even with true labels: {labelled_fails} ({:.2}%)", pct(labelled_fails));
    println!("  tandem-repeat reference:               {repeats} ({:.2}%)", pct(repeats));
    println!("  outvoted in another reference's cluster: {outvoted} ({:.2}%)", pct(outvoted));
    println!("  reads split, no fragment voted to it:  {split} ({:.2}%)", pct(split));
    println!("  other:                                  {other} ({:.2}%)", pct(other));
    let repeat_refs = data.iter().filter(|c| !c.reads.is_empty() && tandem_repeat(&c.reference)).count();
    println!("tandem-repeat references in the set: {repeat_refs}");
}
