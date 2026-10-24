//! # Real Sequencing Data
//!
//! Loads clustered reads from real sequencing runs and scores a consensus
//! strategy against the known reference strands, so decoder changes are
//! measured on genuine instrument noise rather than only on this
//! workspace's own simulator.
//!
//! The loader reads the Clustered Nanopore Reads (CNR) layout published with
//! Srinivasavaradhan, Gopi, Pfister and Yekhanin, "Trellis BMA: Coded trace
//! reconstruction on IDS channels for DNA storage" (ISIT 2021): 10,000
//! Twist-synthesized 110 nt references and 269,709 Oxford Nanopore MinION
//! reads of them. `Centers.txt` has one reference per line; `Clusters.txt`
//! has one block per reference, in the same order, each block introduced by
//! a line of `=` characters. Empty blocks (references that produced no reads)
//! are kept so the two files stay aligned.

use crate::consensus::build_consensus;
use nucle_codec::base::{DnaError, DnaStrand, Nucleotide};
use std::fmt;

/// One reference strand and the real reads attributed to it.
#[derive(Debug, Clone)]
pub struct ReadCluster {
    /// The strand that was actually synthesized.
    pub reference: DnaStrand,
    /// Sequenced reads of that strand, in file order.
    pub reads: Vec<DnaStrand>,
}

/// Parse the CNR `Centers.txt` / `Clusters.txt` pair.
///
/// Fails if a line holds anything other than `A`/`C`/`G`/`T`, or if the
/// number of cluster blocks does not match the number of references.
pub fn parse_cnr(centers: &str, clusters: &str) -> Result<Vec<ReadCluster>, RealDataError> {
    let references: Vec<DnaStrand> = centers
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, l)| DnaStrand::from_str(l).map_err(|e| RealDataError::BadBase { line: i + 1, source: e }))
        .collect::<Result<_, _>>()?;

    let mut blocks: Vec<Vec<DnaStrand>> = Vec::new();
    for (i, raw) in clusters.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('=') {
            blocks.push(Vec::new());
            continue;
        }
        let Some(current) = blocks.last_mut() else {
            return Err(RealDataError::ReadBeforeSeparator { line: i + 1 });
        };
        current.push(
            DnaStrand::from_str(line).map_err(|e| RealDataError::BadBase { line: i + 1, source: e })?,
        );
    }

    if blocks.len() != references.len() {
        return Err(RealDataError::CountMismatch { references: references.len(), clusters: blocks.len() });
    }

    Ok(references
        .into_iter()
        .zip(blocks)
        .map(|(reference, reads)| ReadCluster { reference, reads })
        .collect())
}

/// Errors from loading a real-read dataset.
#[derive(Debug, thiserror::Error)]
pub enum RealDataError {
    #[error("line {line}: {source}")]
    BadBase { line: usize, source: DnaError },
    #[error("line {line}: read appears before the first '=' cluster separator")]
    ReadBeforeSeparator { line: usize },
    #[error("{references} reference strands but {clusters} cluster blocks")]
    CountMismatch { references: usize, clusters: usize },
}

/// Levenshtein distance between two base sequences.
pub fn edit_distance(a: &[Nucleotide], b: &[Nucleotide]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, &x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Accuracy of one coverage bucket.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BucketStats {
    /// Inclusive lower bound on reads per cluster.
    pub min_reads: usize,
    /// Inclusive upper bound (`usize::MAX` for the open last bucket).
    pub max_reads: usize,
    /// Clusters that fell in this bucket.
    pub clusters: usize,
    /// Clusters whose consensus equals the reference exactly.
    pub exact: usize,
    /// Sum of consensus-to-reference edit distances.
    pub edit_sum: usize,
}

impl BucketStats {
    /// Fraction of clusters recovered exactly.
    pub fn exact_rate(&self) -> f64 {
        if self.clusters == 0 { 0.0 } else { self.exact as f64 / self.clusters as f64 }
    }
}

/// Result of scoring a consensus strategy on a set of real clusters.
#[derive(Debug, Clone, PartialEq)]
pub struct RealDataReport {
    /// Clusters with at least one read.
    pub clusters: usize,
    /// Clusters with no reads at all (counted as strand dropouts).
    pub empty_clusters: usize,
    /// Total reads across all clusters.
    pub reads: usize,
    /// Clusters whose consensus equals the reference exactly.
    pub exact: usize,
    /// Sum over clusters of consensus-to-reference edit distance.
    pub consensus_edit_sum: usize,
    /// Sum of reference lengths, the denominator for per-base error.
    pub reference_bases: usize,
    /// Mean per-read edit distance to the reference: the raw channel.
    pub raw_read_error_rate: f64,
    /// Accuracy split by reads per cluster.
    pub buckets: Vec<BucketStats>,
}

impl RealDataReport {
    /// Fraction of non-empty clusters recovered exactly.
    pub fn exact_rate(&self) -> f64 {
        if self.clusters == 0 { 0.0 } else { self.exact as f64 / self.clusters as f64 }
    }

    /// Residual per-base error after consensus.
    pub fn consensus_error_rate(&self) -> f64 {
        if self.reference_bases == 0 { 0.0 } else { self.consensus_edit_sum as f64 / self.reference_bases as f64 }
    }
}

const BUCKETS: [(usize, usize); 5] = [(1, 5), (6, 10), (11, 20), (21, 40), (41, usize::MAX)];

/// Score [`build_consensus`] on `clusters`.
pub fn evaluate_clusters(clusters: &[ReadCluster]) -> RealDataReport {
    evaluate_with(clusters, |reads, _| build_consensus(reads).map(|r| r.sequence))
}

/// Score any consensus strategy. The callback receives the cluster's reads
/// and the reference length (a decoder knows its own strand length) and
/// returns its reconstruction, or `None` to report a failure.
pub fn evaluate_with<F>(clusters: &[ReadCluster], mut consensus: F) -> RealDataReport
where
    F: FnMut(&[DnaStrand], usize) -> Option<DnaStrand>,
{
    let mut buckets: Vec<BucketStats> = BUCKETS
        .iter()
        .map(|&(min_reads, max_reads)| BucketStats { min_reads, max_reads, ..Default::default() })
        .collect();
    let mut report = RealDataReport {
        clusters: 0,
        empty_clusters: 0,
        reads: 0,
        exact: 0,
        consensus_edit_sum: 0,
        reference_bases: 0,
        raw_read_error_rate: 0.0,
        buckets: Vec::new(),
    };
    let mut raw_edits = 0usize;
    let mut raw_bases = 0usize;

    for cluster in clusters {
        if cluster.reads.is_empty() {
            report.empty_clusters += 1;
            continue;
        }
        let reference = cluster.reference.bases();
        report.clusters += 1;
        report.reads += cluster.reads.len();
        report.reference_bases += reference.len();
        for read in &cluster.reads {
            raw_edits += edit_distance(read.bases(), reference);
            raw_bases += reference.len();
        }

        let dist = match consensus(&cluster.reads, reference.len()) {
            Some(seq) => edit_distance(seq.bases(), reference),
            None => reference.len(),
        };
        report.consensus_edit_sum += dist;
        if dist == 0 {
            report.exact += 1;
        }
        if let Some(b) = buckets
            .iter_mut()
            .find(|b| (b.min_reads..=b.max_reads).contains(&cluster.reads.len()))
        {
            b.clusters += 1;
            b.edit_sum += dist;
            if dist == 0 {
                b.exact += 1;
            }
        }
    }

    report.raw_read_error_rate = if raw_bases == 0 { 0.0 } else { raw_edits as f64 / raw_bases as f64 };
    report.buckets = buckets;
    report
}

/// Result of decoding real reads with their cluster labels thrown away.
#[derive(Debug, Clone, PartialEq)]
pub struct BlindReport {
    /// Reads pooled and shuffled.
    pub reads: usize,
    /// References that had at least one read.
    pub references: usize,
    /// Clusters [`cluster_reads`](crate::cluster::cluster_reads) produced.
    pub clusters: usize,
    /// Clusters with at least two reads (enough to vote).
    pub multi_read_clusters: usize,
    /// Fraction of reads whose cluster's majority reference is their own.
    pub purity: f64,
    /// Distinct references some cluster's consensus reproduced exactly.
    pub recovered_exact: usize,
}

impl BlindReport {
    pub fn recovery_rate(&self) -> f64 {
        if self.references == 0 { 0.0 } else { self.recovered_exact as f64 / self.references as f64 }
    }
}

impl fmt::Display for BlindReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "reads pooled and shuffled: {}", self.reads)?;
        writeln!(f, "clusters found:            {} ({} with 2+ reads) for {} references", self.clusters, self.multi_read_clusters, self.references)?;
        writeln!(f, "cluster purity:            {:.2}%", self.purity * 100.0)?;
        writeln!(f, "references recovered:      {} / {} ({:.2}%)", self.recovered_exact, self.references, self.recovery_rate() * 100.0)
    }
}

/// Reads used for one cluster's consensus in [`evaluate_blind`].
pub const MAX_CONSENSUS_READS: usize = 64;

/// Pool every read, shuffle (seeded), cluster without labels, take a
/// length-aware consensus of each cluster (a lone read stands for itself),
/// and count how many references come back exactly: the whole decode path
/// a real run needs, scored against ground truth.
pub fn evaluate_blind(data: &[ReadCluster], config: &crate::cluster::ClusterConfig, seed: u64) -> BlindReport {
    // A stream that never changes, so the scores published for a seed
    // stay reproducible (see `nucle_codec::stable_rng`).
    use nucle_codec::stable_rng::StableRng;

    let mut pooled: Vec<(usize, DnaStrand)> = data
        .iter()
        .enumerate()
        .flat_map(|(label, c)| c.reads.iter().cloned().map(move |r| (label, r)))
        .collect();
    let mut rng = StableRng::seed_from_u64(seed);
    for i in (1..pooled.len()).rev() {
        let j = rng.gen_range(0..i + 1);
        pooled.swap(i, j);
    }
    let reads: Vec<DnaStrand> = pooled.iter().map(|(_, r)| r.clone()).collect();
    let clusters = crate::cluster::cluster_reads(&reads, config);

    let mut majority_hits = 0usize;
    for c in &clusters {
        let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for &i in c {
            *counts.entry(pooled[i].0).or_insert(0) += 1;
        }
        majority_hits += counts.values().copied().max().unwrap_or(0);
    }

    let references: std::collections::HashSet<&DnaStrand> =
        data.iter().filter(|c| !c.reads.is_empty()).map(|c| &c.reference).collect();
    let mut recovered: std::collections::HashSet<DnaStrand> = std::collections::HashSet::new();
    let multi = clusters.iter().filter(|c| c.len() >= 2).count();
    let len = modal_reference_length(data);
    for c in &clusters {
        // A single read is its own best guess (most clean short reads are
        // error-free); larger clusters vote, on at most
        // `MAX_CONSENSUS_READS` reads, since agreement saturates long
        // before a merged cluster of thousands.
        let group: Vec<DnaStrand> = c.iter().take(MAX_CONSENSUS_READS).map(|&i| reads[i].clone()).collect();
        if let Some(r) = crate::consensus::build_consensus_with_length(&group, len) {
            if references.contains(&r.sequence) {
                recovered.insert(r.sequence);
            }
        }
    }

    BlindReport {
        reads: reads.len(),
        references: references.len(),
        clusters: clusters.len(),
        multi_read_clusters: multi,
        purity: if reads.is_empty() { 0.0 } else { majority_hits as f64 / reads.len() as f64 },
        recovered_exact: recovered.len(),
    }
}

/// The strand length a decoder would be configured with: the most common
/// reference length.
fn modal_reference_length(data: &[ReadCluster]) -> usize {
    let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for c in data {
        *counts.entry(c.reference.len()).or_insert(0) += 1;
    }
    counts.into_iter().max_by_key(|&(len, n)| (n, len)).map(|(len, _)| len).unwrap_or(0)
}

impl fmt::Display for RealDataReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "clusters scored:        {} ({} empty, {} reads)", self.clusters, self.empty_clusters, self.reads)?;
        writeln!(f, "raw read error rate:    {:.2}%", self.raw_read_error_rate * 100.0)?;
        writeln!(f, "consensus error rate:   {:.3}%", self.consensus_error_rate() * 100.0)?;
        writeln!(f, "exact reconstructions:  {} / {} ({:.2}%)", self.exact, self.clusters, self.exact_rate() * 100.0)?;
        writeln!(f, "reads/cluster  clusters  exact%   errors/strand")?;
        for b in self.buckets.iter().filter(|b| b.clusters > 0) {
            let range = if b.max_reads == usize::MAX {
                format!("{}+", b.min_reads)
            } else {
                format!("{}-{}", b.min_reads, b.max_reads)
            };
            writeln!(
                f,
                "{:>13}  {:>8}  {:>6.2}  {:>13.3}",
                range,
                b.clusters,
                b.exact_rate() * 100.0,
                b.edit_sum as f64 / b.clusters as f64
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> DnaStrand {
        DnaStrand::from_str(x).unwrap()
    }

    #[test]
    fn parses_blocks_in_reference_order_and_keeps_empty_ones() {
        let centers = "ACGTACGT\nTTTTCCCC\nGGGGAAAA\n";
        let clusters = "=====\nACGTACGT\nACGAACGT\n=====\n=====\nGGGGAAAA\n";
        let data = parse_cnr(centers, clusters).unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].reads.len(), 2);
        assert!(data[1].reads.is_empty());
        assert_eq!(data[2].reads[0].to_string(), "GGGGAAAA");
    }

    #[test]
    fn rejects_misaligned_files() {
        let err = parse_cnr("ACGT\nACGT\n", "=====\nACGT\n").unwrap_err();
        assert!(matches!(err, RealDataError::CountMismatch { references: 2, clusters: 1 }));
        let err = parse_cnr("ACGT\n", "ACGT\n=====\n").unwrap_err();
        assert!(matches!(err, RealDataError::ReadBeforeSeparator { line: 1 }));
        let err = parse_cnr("ACGN\n", "=====\n").unwrap_err();
        assert!(matches!(err, RealDataError::BadBase { line: 1, .. }));
    }

    #[test]
    fn edit_distance_counts_substitutions_and_indels() {
        assert_eq!(edit_distance(s("ACGT").bases(), s("ACGT").bases()), 0);
        assert_eq!(edit_distance(s("ACGT").bases(), s("AGGT").bases()), 1);
        assert_eq!(edit_distance(s("ACGT").bases(), s("ACT").bases()), 1);
        assert_eq!(edit_distance(s("ACGT").bases(), s("ACGGT").bases()), 1);
        assert_eq!(edit_distance(s("AAAA").bases(), s("TTTTTT").bases()), 6);
    }

    #[test]
    fn report_separates_raw_channel_from_consensus_and_buckets_by_coverage() {
        let data = parse_cnr(
            "ACGTACGTAC\nTTGGCCAATT\n",
            "=====\nACGTACGTAC\nACGTACGTAC\nACGAACGTAC\n=====\n",
        )
        .unwrap();
        let report = evaluate_clusters(&data);
        assert_eq!(report.clusters, 1);
        assert_eq!(report.empty_clusters, 1);
        assert_eq!(report.exact, 1);
        assert!((report.raw_read_error_rate - 1.0 / 30.0).abs() < 1e-12);
        assert_eq!(report.consensus_error_rate(), 0.0);
        assert_eq!(report.buckets[0].clusters, 1);
        assert!(format!("{report}").contains("exact reconstructions:  1 / 1"));
    }
}
