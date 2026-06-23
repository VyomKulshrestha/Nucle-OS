//! # Consensus Sequencing Engine
//!
//! When the same DNA strand is sequenced multiple times (coverage depth),
//! each copy has independent errors. By aligning multiple noisy copies
//! and taking a majority vote at each position, the consensus sequence
//! is dramatically more accurate than any individual read.
//!
//! Typical coverage: 5–20× per strand in DNA storage systems.

use nucle_codec::base::{DnaStrand, Nucleotide};
use std::collections::HashMap;

/// Result of consensus calling for a single strand.
#[derive(Debug, Clone)]
pub struct ConsensusResult {
    /// The consensus sequence (majority-voted).
    pub sequence: DnaStrand,
    /// Number of reads that contributed to this consensus.
    pub coverage: usize,
    /// Per-position confidence (fraction of reads agreeing with consensus).
    pub confidence: Vec<f64>,
    /// Average confidence across all positions.
    pub avg_confidence: f64,
}

/// Build a consensus sequence from multiple noisy copies of the same strand.
///
/// # Algorithm
///
/// 1. Align reads by position (assumes reads are pre-aligned / same length)
/// 2. At each position, count nucleotide frequencies
/// 3. Pick the most frequent base (majority vote)
/// 4. Record confidence as fraction of reads agreeing
///
/// For reads of different lengths (due to indels), we use the median
/// length and truncate/pad as needed.
pub fn build_consensus(reads: &[DnaStrand]) -> Option<ConsensusResult> {
    if reads.is_empty() {
        return None;
    }

    if reads.len() == 1 {
        let len = reads[0].len();
        return Some(ConsensusResult {
            sequence: reads[0].clone(),
            coverage: 1,
            confidence: vec![1.0; len],
            avg_confidence: 1.0,
        });
    }

    // Use the median length as the consensus length
    let mut lengths: Vec<usize> = reads.iter().map(|r| r.len()).collect();
    lengths.sort();
    let consensus_len = lengths[lengths.len() / 2];

    let mut consensus_bases: Vec<Nucleotide> = Vec::with_capacity(consensus_len);
    let mut confidence: Vec<f64> = Vec::with_capacity(consensus_len);

    for pos in 0..consensus_len {
        // Count nucleotide frequencies at this position
        let mut counts: HashMap<Nucleotide, usize> = HashMap::new();
        let mut total = 0;

        for read in reads {
            if let Some(base) = read.get(pos) {
                *counts.entry(base).or_insert(0) += 1;
                total += 1;
            }
        }

        if total == 0 {
            // No reads cover this position — use A as placeholder
            consensus_bases.push(Nucleotide::A);
            confidence.push(0.0);
            continue;
        }

        // Pick the most frequent base
        let (&best_base, &best_count) = counts.iter()
            .max_by_key(|(_, &count)| count)
            .unwrap();

        consensus_bases.push(best_base);
        confidence.push(best_count as f64 / total as f64);
    }

    let avg_conf = if confidence.is_empty() {
        0.0
    } else {
        confidence.iter().sum::<f64>() / confidence.len() as f64
    };

    Some(ConsensusResult {
        sequence: DnaStrand::new(consensus_bases),
        coverage: reads.len(),
        confidence,
        avg_confidence: avg_conf,
    })
}

/// Build consensus for groups of reads, where each group corresponds
/// to copies of the same original strand.
///
/// `read_groups`: each inner Vec contains multiple noisy copies of one strand.
/// Returns one consensus sequence per group.
pub fn build_consensus_batch(read_groups: &[Vec<DnaStrand>]) -> Vec<Option<ConsensusResult>> {
    read_groups.iter().map(|group| build_consensus(group)).collect()
}

/// Determine if more coverage is needed based on confidence threshold.
///
/// Returns the positions where confidence is below the threshold.
pub fn low_confidence_positions(result: &ConsensusResult, threshold: f64) -> Vec<usize> {
    result.confidence.iter()
        .enumerate()
        .filter(|(_, &conf)| conf < threshold)
        .map(|(pos, _)| pos)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_read_consensus() {
        let read = DnaStrand::from_str("ATCG").unwrap();
        let result = build_consensus(&[read.clone()]).unwrap();

        assert_eq!(result.sequence, read);
        assert_eq!(result.coverage, 1);
        assert_eq!(result.avg_confidence, 1.0);
    }

    #[test]
    fn test_perfect_consensus() {
        // All reads identical — perfect consensus
        let reads = vec![
            DnaStrand::from_str("ATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATCG").unwrap(),
        ];

        let result = build_consensus(&reads).unwrap();
        assert_eq!(result.sequence.to_string(), "ATCGATCG");
        assert_eq!(result.coverage, 3);
        assert_eq!(result.avg_confidence, 1.0);
    }

    #[test]
    fn test_majority_voting() {
        // 2 out of 3 agree at each position
        let reads = vec![
            DnaStrand::from_str("ATCG").unwrap(), // Original
            DnaStrand::from_str("ATCG").unwrap(), // Original
            DnaStrand::from_str("GCAT").unwrap(), // All different
        ];

        let result = build_consensus(&reads).unwrap();
        // Majority at each position should match the original
        assert_eq!(result.sequence.to_string(), "ATCG");
        // Confidence should be 2/3 at each position
        for &conf in &result.confidence {
            assert!((conf - 2.0 / 3.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_empty_reads() {
        assert!(build_consensus(&[]).is_none());
    }

    #[test]
    fn test_low_confidence_detection() {
        let result = ConsensusResult {
            sequence: DnaStrand::from_str("ATCG").unwrap(),
            coverage: 5,
            confidence: vec![1.0, 0.6, 0.4, 1.0],
            avg_confidence: 0.75,
        };

        let low = low_confidence_positions(&result, 0.8);
        assert_eq!(low, vec![1, 2]); // Positions 1 and 2 are below 80%
    }

    #[test]
    fn test_consensus_batch() {
        let groups = vec![
            vec![
                DnaStrand::from_str("AAAA").unwrap(),
                DnaStrand::from_str("AAAA").unwrap(),
            ],
            vec![
                DnaStrand::from_str("TTTT").unwrap(),
                DnaStrand::from_str("TTTT").unwrap(),
            ],
        ];

        let results = build_consensus_batch(&groups);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap().sequence.to_string(), "AAAA");
        assert_eq!(results[1].as_ref().unwrap().sequence.to_string(), "TTTT");
    }
}
