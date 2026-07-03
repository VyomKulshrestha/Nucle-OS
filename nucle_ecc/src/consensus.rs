//! # Consensus Sequencing Engine
//!
//! When the same DNA strand is sequenced multiple times (coverage depth),
//! each copy has independent errors. By aligning multiple noisy copies
//! and taking a majority vote at each position, the consensus sequence
//! is dramatically more accurate than any individual read.
//!
//! Typical coverage: 5–20× per strand in DNA storage systems.
//!
//! Reads of the same length as the reference are voted positionally with
//! no alignment cost -- correct and cheap for substitution-only noise
//! (Illumina). A read of a *different* length is the signature of an
//! insertion or deletion, which shifts every base after it out of position;
//! voting it positionally would compare unrelated bases and produce
//! garbage. Those reads are first globally aligned to the reference
//! (Needleman-Wunsch) so a frame-shifted read still contributes its votes
//! to the right column, which is what makes this work under Nanopore's
//! indel-dominant noise too, not just Illumina's substitution-dominant noise.

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
/// 1. Pick a reference read (the one whose length is closest to the
///    group's median -- a reasonable "typical" pick without the cost of
///    computing full pairwise distances between every read).
/// 2. For each other read: if it's the same length as the reference,
///    vote positionally (cheap, correct when errors are substitutions
///    only). If it's a different length, globally align it to the
///    reference first (Needleman-Wunsch) so an insertion or deletion
///    doesn't throw off every vote after it.
/// 3. At each reference position, tally the aligned votes and pick the
///    most frequent base.
/// 4. Record confidence as the fraction of covering reads that agreed.
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

    let mut lengths: Vec<usize> = reads.iter().map(|r| r.len()).collect();
    lengths.sort_unstable();
    let median_len = lengths[lengths.len() / 2];
    let reference = reads.iter()
        .min_by_key(|r| (r.len() as i64 - median_len as i64).abs())
        .unwrap();
    let ref_bases = reference.bases().to_vec();
    let ref_len = ref_bases.len();

    if ref_len == 0 {
        return Some(ConsensusResult {
            sequence: DnaStrand::new(Vec::new()),
            coverage: reads.len(),
            confidence: Vec::new(),
            avg_confidence: 0.0,
        });
    }

    let mut votes: Vec<HashMap<Nucleotide, usize>> = vec![HashMap::new(); ref_len];
    let mut coverage_per_pos: Vec<usize> = vec![0; ref_len];

    for read in reads {
        let read_bases = read.bases();

        if read_bases.len() == ref_len {
            for (pos, &base) in read_bases.iter().enumerate() {
                *votes[pos].entry(base).or_insert(0) += 1;
                coverage_per_pos[pos] += 1;
            }
            continue;
        }

        // Different length than the reference: an insertion or deletion
        // happened somewhere in this read. Align it to the reference so
        // its votes land on the reference position they actually
        // correspond to, instead of whatever position they happen to fall
        // on after the shift.
        let (aligned_ref, aligned_read) = needleman_wunsch_align(&ref_bases, read_bases);
        let mut ref_pos = 0;
        for (r, q) in aligned_ref.iter().zip(aligned_read.iter()) {
            match (r, q) {
                (Some(_), Some(base)) => {
                    *votes[ref_pos].entry(*base).or_insert(0) += 1;
                    coverage_per_pos[ref_pos] += 1;
                    ref_pos += 1;
                }
                (Some(_), None) => {
                    // Deletion relative to the reference: this read has
                    // nothing to vote with at this position.
                    ref_pos += 1;
                }
                (None, Some(_)) => {
                    // Insertion relative to the reference: an extra base
                    // with no reference position to attribute it to --
                    // discarded, same as noise.
                }
                (None, None) => unreachable!("an alignment column is never a gap on both sides"),
            }
        }
    }

    let mut consensus_bases: Vec<Nucleotide> = Vec::with_capacity(ref_len);
    let mut confidence: Vec<f64> = Vec::with_capacity(ref_len);
    for pos in 0..ref_len {
        if coverage_per_pos[pos] == 0 {
            // Every read had a deletion at this position -- fall back to
            // the reference's own base rather than an arbitrary placeholder.
            consensus_bases.push(ref_bases[pos]);
            confidence.push(0.0);
            continue;
        }
        let (&best_base, &best_count) = votes[pos].iter()
            .max_by_key(|(_, &count)| count)
            .unwrap();
        consensus_bases.push(best_base);
        confidence.push(best_count as f64 / coverage_per_pos[pos] as f64);
    }

    let avg_conf = confidence.iter().sum::<f64>() / confidence.len() as f64;

    Some(ConsensusResult {
        sequence: DnaStrand::new(consensus_bases),
        coverage: reads.len(),
        confidence,
        avg_confidence: avg_conf,
    })
}

/// Global (Needleman-Wunsch) pairwise alignment between two nucleotide
/// sequences. Returns both sequences padded with gaps (`None`) so they're
/// the same length and column-aligned -- position `i` of the two returned
/// vectors is always either a match, a mismatch, or one side being a gap,
/// never a gap on both sides.
fn needleman_wunsch_align(a: &[Nucleotide], b: &[Nucleotide]) -> (Vec<Option<Nucleotide>>, Vec<Option<Nucleotide>>) {
    const MATCH: i32 = 2;
    const MISMATCH: i32 = -1;
    const GAP: i32 = -2;

    let n = a.len();
    let m = b.len();

    let mut dp = vec![vec![0i32; m + 1]; n + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i as i32 * GAP;
    }
    for j in 0..=m {
        dp[0][j] = j as i32 * GAP;
    }
    for i in 1..=n {
        for j in 1..=m {
            let diag = dp[i - 1][j - 1] + if a[i - 1] == b[j - 1] { MATCH } else { MISMATCH };
            let up = dp[i - 1][j] + GAP;
            let left = dp[i][j - 1] + GAP;
            dp[i][j] = diag.max(up).max(left);
        }
    }

    let mut aligned_a = Vec::with_capacity(n.max(m));
    let mut aligned_b = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            let diag = dp[i - 1][j - 1] + if a[i - 1] == b[j - 1] { MATCH } else { MISMATCH };
            if dp[i][j] == diag {
                aligned_a.push(Some(a[i - 1]));
                aligned_b.push(Some(b[j - 1]));
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && dp[i][j] == dp[i - 1][j] + GAP {
            aligned_a.push(Some(a[i - 1]));
            aligned_b.push(None);
            i -= 1;
            continue;
        }
        aligned_a.push(None);
        aligned_b.push(Some(b[j - 1]));
        j -= 1;
    }
    aligned_a.reverse();
    aligned_b.reverse();
    (aligned_a, aligned_b)
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

    #[test]
    fn test_alignment_places_gap_at_the_insertion() {
        let reference = [Nucleotide::A, Nucleotide::T, Nucleotide::C, Nucleotide::G];
        // Same as `reference` but with an extra G inserted after position 2.
        let inserted = [Nucleotide::A, Nucleotide::T, Nucleotide::C, Nucleotide::G, Nucleotide::G];
        let (aligned_ref, aligned_read) = needleman_wunsch_align(&reference, &inserted);

        // Every reference base should still find its match; exactly one
        // extra base in `inserted` should land on a reference-side gap.
        assert_eq!(aligned_ref.iter().filter(|b| b.is_some()).count(), 4);
        assert_eq!(aligned_read.iter().filter(|b| b.is_some()).count(), 5);
        assert_eq!(aligned_ref.iter().filter(|b| b.is_none()).count(), 1);
        // Reconstructing the reference from the non-gap columns on its side
        // should reproduce it exactly, in order.
        let recovered: Vec<Nucleotide> = aligned_ref.iter().filter_map(|b| *b).collect();
        assert_eq!(recovered, reference);
    }

    #[test]
    fn test_consensus_corrects_frame_shifting_indels() {
        // A majority-of-reads-affected scenario: only 2 of 5 copies are
        // exact, the other 3 each have a *different* single indel. Plain
        // positional voting (comparing raw index i across all 5 reads)
        // would be dominated by three different frame-shifted reads past
        // the indel point and would not reliably reconstruct the original.
        // Alignment-based voting anchors every read to the reference
        // strand's own coordinates first, so the two exact copies plus the
        // correctly-realigned bases from the other three still agree.
        let original = "ATCGATCGTACGATCG";
        let exact_a = DnaStrand::from_str(original).unwrap();
        let exact_b = DnaStrand::from_str(original).unwrap();
        // Deletion: drop the 'T' at index 8.
        let deletion = DnaStrand::from_str("ATCGATCGACGATCG").unwrap();
        // Insertion: an extra 'A' after index 8.
        let insertion = DnaStrand::from_str("ATCGATCGTAACGATCG").unwrap();
        // A second, independent deletion: drop the 'G' at index 3.
        let deletion2 = DnaStrand::from_str("ATCATCGTACGATCG").unwrap();

        let reads = vec![exact_a, exact_b, deletion, insertion, deletion2];
        let result = build_consensus(&reads).expect("non-empty group must produce a consensus");
        assert_eq!(
            result.sequence.to_string(),
            original,
            "alignment-anchored voting should recover the original sequence \
             even though 3 of 5 reads are frame-shifted by an indel"
        );
    }

    #[test]
    fn test_consensus_handles_different_length_groups_without_panicking() {
        // Regression guard: reads of wildly different lengths (e.g. a
        // severely truncated Nanopore read) must not panic the aligner.
        let reads = vec![
            DnaStrand::from_str("ATCGATCGATCG").unwrap(),
            DnaStrand::from_str("AT").unwrap(),
            DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap(),
        ];
        let result = build_consensus(&reads);
        assert!(result.is_some());
    }
}
