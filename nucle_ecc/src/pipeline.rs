//! # Full Error Correction Pipeline
//!
//! Orchestrates the multi-stage error correction flow:
//!
//! ```text
//! Noisy reads → [Consensus] → [Inner decode] → [Outer RS decode] → Clean data
//! ```
//!
//! The pipeline is configurable — each stage can be enabled/disabled
//! independently depending on the error channel characteristics.

use crate::reed_solomon::{ReedSolomon, RsConfig, RsError};
use crate::consensus::{self, ConsensusResult};
use nucle_codec::base::{DnaStrand, Nucleotide};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Configuration for the repair pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Whether to run consensus sequencing (requires multiple copies).
    pub enable_consensus: bool,
    /// Minimum confidence threshold for consensus. Positions below this
    /// are flagged as uncertain.
    pub consensus_threshold: f64,
    /// Whether to apply Reed-Solomon outer code recovery.
    pub enable_reed_solomon: bool,
    /// Reed-Solomon configuration.
    pub rs_config: RsConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            enable_consensus: true,
            consensus_threshold: 0.8,
            enable_reed_solomon: true,
            rs_config: RsConfig::default(),
        }
    }
}

/// A recovery manifest tracking ECC and consensus details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecoveryManifest {
    pub observed_error_rate: f64,
    pub consensus_method: String,
    pub sequencing_profile: String,
    pub recovered_strands: usize,
    pub ecc_success: bool,
    /// Per-strand-position observed mismatch rate: (strand index, fraction
    /// of bases that differed between the pre-correction and post-correction
    /// strand at that position). Empty when there was nothing to compare
    /// (e.g. no parity strands were involved in recovery).
    pub observed_error_distribution: Vec<(usize, f64)>,
}

/// Compare strands before and after ECC correction to derive a real,
/// per-position observed error distribution — not a synthetic estimate.
/// Positions present in both slices are compared base-by-base; a position
/// only present in one slice (an erasure) is skipped, since there is no
/// "before" state to diff against.
pub fn compute_error_distribution(before: &[DnaStrand], after: &[DnaStrand]) -> Vec<(usize, f64)> {
    before
        .iter()
        .zip(after.iter())
        .enumerate()
        .filter_map(|(i, (b, a))| {
            let b_bases = b.bases();
            let a_bases = a.bases();
            let len = b_bases.len().min(a_bases.len());
            if len == 0 {
                return None;
            }
            let mismatches = b_bases
                .iter()
                .take(len)
                .zip(a_bases.iter().take(len))
                .filter(|(bb, ab)| bb != ab)
                .count();
            Some((i, mismatches as f64 / len as f64))
        })
        .collect()
}

/// Consensus-vote each group of coverage-copy reads, then Reed-Solomon
/// decode the consensus results.
///
/// `data_groups`/`parity_groups`: dense, one entry per logical strand
/// position (a position with zero surviving reads is an empty `Vec`, which
/// becomes an erasure for RS -- same as a strand that never arrived at
/// all). This is the one real fix for the standard DNA-storage failure
/// mode: RS alone only recovers a strand that's entirely missing, never one
/// that survived corrupted, but majority-voting across independent reads of
/// the SAME strand corrects most substitution errors regardless of which
/// read has them. Requires actual sequencing coverage (multiple reads per
/// group) to have anything to vote across -- a single read per group is
/// consensus voting on nothing, which is still safe (returns that read
/// unchanged) but provides no correction.
///
/// Used by both `nucle_vfs::syscall::dna_read` and the playground's
/// interactive benchmark, so there's one implementation of "how redundancy
/// actually helps under noise," not two that can drift apart.
pub fn consensus_then_rs_decode(
    data_groups: &[Vec<DnaStrand>],
    parity_groups: &[Vec<DnaStrand>],
    rs_config: RsConfig,
) -> Vec<DnaStrand> {
    let vote = |group: &[DnaStrand]| -> Option<DnaStrand> {
        consensus::build_consensus(group).map(|r| r.sequence)
    };
    let consensus_data: Vec<Option<DnaStrand>> = data_groups.iter().map(|g| vote(g)).collect();
    let consensus_parity: Vec<DnaStrand> = parity_groups.iter().filter_map(|g| vote(g)).collect();

    if consensus_parity.is_empty() {
        return consensus_data.into_iter().flatten().collect();
    }

    let rs = ReedSolomon::new(rs_config);
    let received: Vec<Option<Vec<u8>>> = consensus_data.iter()
        .map(|opt| opt.as_ref().map(|s| s.bases().iter().map(|n| n.to_bits()).collect()))
        .collect();
    let parity_bytes: Vec<Vec<u8>> = consensus_parity.iter()
        .map(|s| s.bases().iter().map(|n| n.to_bits()).collect())
        .collect();

    match rs.decode_block(&received, &parity_bytes) {
        Ok(recovered_bytes) => recovered_bytes.iter().map(|bytes| {
            let bases: Vec<_> = bytes.iter().filter_map(|&b| Nucleotide::from_bits(b).ok()).collect();
            DnaStrand::new(bases)
        }).collect(),
        Err(_) => consensus_data.into_iter().flatten().collect(),
    }
}

/// Statistics from running the repair pipeline.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineStats {
    /// Number of input read groups (or strands).
    pub input_count: usize,
    /// Number of strands after consensus.
    pub post_consensus_count: usize,
    /// Average consensus confidence.
    pub avg_confidence: f64,
    /// Number of strands recovered by RS.
    pub rs_recovered_count: usize,
    /// Number of low-confidence positions flagged.
    pub low_confidence_positions: usize,
    /// Whether the pipeline completed successfully.
    pub success: bool,
}

impl fmt::Display for PipelineStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "┌─ Repair Pipeline Stats ─────────────")?;
        writeln!(f, "│ Input groups:     {}", self.input_count)?;
        writeln!(f, "│ Post-consensus:   {}", self.post_consensus_count)?;
        writeln!(f, "│ Avg confidence:   {:.1}%", self.avg_confidence * 100.0)?;
        writeln!(f, "│ RS recovered:     {}", self.rs_recovered_count)?;
        writeln!(f, "│ Low-conf pos:     {}", self.low_confidence_positions)?;
        writeln!(f, "│ Status:           {}", if self.success { "✓ OK" } else { "✗ FAILED" })?;
        write!(f, "└──────────────────────────────────────")
    }
}

/// The full error correction pipeline.
pub struct RepairPipeline {
    config: PipelineConfig,
}

impl RepairPipeline {
    /// Create a new pipeline with the given configuration.
    pub fn new(config: PipelineConfig) -> Self {
        Self { config }
    }

    /// Create with default settings.
    pub fn default_pipeline() -> Self {
        Self::new(PipelineConfig::default())
    }

    /// Run the consensus stage on grouped reads.
    ///
    /// Input: groups of reads, where each group = multiple copies of one strand.
    /// Output: one consensus strand per group, plus results with confidence.
    pub fn run_consensus(
        &self,
        read_groups: &[Vec<DnaStrand>],
    ) -> (Vec<DnaStrand>, Vec<ConsensusResult>, PipelineStats) {
        let mut consensus_strands = Vec::new();
        let mut consensus_results = Vec::new();
        let mut total_confidence = 0.0;
        let mut total_low_conf = 0;

        for group in read_groups {
            if let Some(result) = consensus::build_consensus(group) {
                let low = consensus::low_confidence_positions(
                    &result,
                    self.config.consensus_threshold,
                );
                total_low_conf += low.len();
                total_confidence += result.avg_confidence;
                consensus_strands.push(result.sequence.clone());
                consensus_results.push(result);
            }
        }

        let avg_conf = if consensus_results.is_empty() {
            0.0
        } else {
            total_confidence / consensus_results.len() as f64
        };

        let stats = PipelineStats {
            input_count: read_groups.len(),
            post_consensus_count: consensus_strands.len(),
            avg_confidence: avg_conf,
            rs_recovered_count: 0,
            low_confidence_positions: total_low_conf,
            success: !consensus_strands.is_empty(),
        };

        (consensus_strands, consensus_results, stats)
    }

    /// Run Reed-Solomon recovery on strand data.
    ///
    /// `strand_data`: Some(bytes) for present strands, None for missing.
    /// `parity_data`: the parity strands.
    ///
    /// Returns recovered data strands.
    pub fn run_rs_recovery(
        &self,
        strand_data: &[Option<Vec<u8>>],
        parity_data: &[Vec<u8>],
    ) -> Result<(Vec<Vec<u8>>, usize), RsError> {
        let rs = ReedSolomon::new(self.config.rs_config.clone());

        let erased_count = strand_data.iter().filter(|s| s.is_none()).count();
        let recovered = rs.decode_block(strand_data, parity_data)?;

        Ok((recovered, erased_count))
    }

    /// Run the full pipeline: consensus → RS recovery.
    ///
    /// `read_groups`: grouped reads (multiple copies per strand).
    /// `parity_data`: RS parity strands (if RS is enabled).
    /// `expected_count`: expected number of data strands.
    pub fn run_full(
        &self,
        read_groups: &[Vec<DnaStrand>],
        parity_data: Option<&[Vec<u8>]>,
    ) -> (Vec<DnaStrand>, PipelineStats) {
        // Stage 1: Consensus
        let (consensus_strands, _results, mut stats) = if self.config.enable_consensus {
            self.run_consensus(read_groups)
        } else {
            // No consensus — take first read from each group
            let strands: Vec<DnaStrand> = read_groups.iter()
                .filter_map(|g| g.first().cloned())
                .collect();
            let stats = PipelineStats {
                input_count: read_groups.len(),
                post_consensus_count: strands.len(),
                avg_confidence: 1.0,
                rs_recovered_count: 0,
                low_confidence_positions: 0,
                success: !strands.is_empty(),
            };
            (strands, Vec::new(), stats)
        };

        // Stage 2: RS recovery (if enabled and parity available)
        if self.config.enable_reed_solomon {
            if let Some(parity) = parity_data {
                // Convert consensus strands to byte vectors
                let strand_bytes: Vec<Option<Vec<u8>>> = consensus_strands.iter()
                    .map(|s| {
                        Some(s.bases().iter().map(|n| n.to_bits()).collect::<Vec<u8>>())
                    })
                    .collect();

                if let Ok((recovered, count)) = self.run_rs_recovery(&strand_bytes, parity) {
                    stats.rs_recovered_count = count;
                    // Convert back to DnaStrands
                    let recovered_strands: Vec<DnaStrand> = recovered.iter()
                        .map(|bytes| {
                            let bases: Vec<_> = bytes.iter()
                                .filter_map(|&b| nucle_codec::base::Nucleotide::from_bits(b).ok())
                                .collect();
                            DnaStrand::new(bases)
                        })
                        .collect();
                    return (recovered_strands, stats);
                }
            }
        }

        (consensus_strands, stats)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_distribution_detects_mismatch() {
        let before = vec![DnaStrand::from_str("ATCG").unwrap(), DnaStrand::from_str("GGGG").unwrap()];
        let after = vec![DnaStrand::from_str("ATCC").unwrap(), DnaStrand::from_str("GGGG").unwrap()];
        let dist = compute_error_distribution(&before, &after);
        assert_eq!(dist.len(), 2);
        assert_eq!(dist[0].0, 0);
        assert!((dist[0].1 - 0.25).abs() < 1e-9); // 1 of 4 bases differs
        assert_eq!(dist[1].0, 1);
        assert_eq!(dist[1].1, 0.0); // identical strand
    }

    #[test]
    fn test_error_distribution_empty_on_no_overlap() {
        let before: Vec<DnaStrand> = vec![];
        let after = vec![DnaStrand::from_str("ATCG").unwrap()];
        assert!(compute_error_distribution(&before, &after).is_empty());
    }

    #[test]
    fn test_consensus_pipeline() {
        let pipeline = RepairPipeline::default_pipeline();

        let groups = vec![
            vec![
                DnaStrand::from_str("ATCG").unwrap(),
                DnaStrand::from_str("ATCG").unwrap(),
                DnaStrand::from_str("ATCG").unwrap(),
            ],
            vec![
                DnaStrand::from_str("GCTA").unwrap(),
                DnaStrand::from_str("GCTA").unwrap(),
            ],
        ];

        let (strands, _results, stats) = pipeline.run_consensus(&groups);

        assert_eq!(strands.len(), 2);
        assert_eq!(strands[0].to_string(), "ATCG");
        assert_eq!(strands[1].to_string(), "GCTA");
        assert!(stats.success);
        assert_eq!(stats.avg_confidence, 1.0);
    }

    #[test]
    fn test_full_pipeline_no_rs() {
        let config = PipelineConfig {
            enable_consensus: true,
            enable_reed_solomon: false,
            ..PipelineConfig::default()
        };
        let pipeline = RepairPipeline::new(config);

        let groups = vec![
            vec![DnaStrand::from_str("AAAA").unwrap()],
            vec![DnaStrand::from_str("TTTT").unwrap()],
        ];

        let (strands, stats) = pipeline.run_full(&groups, None);

        assert_eq!(strands.len(), 2);
        assert!(stats.success);
    }

    #[test]
    fn test_pipeline_stats_display() {
        let stats = PipelineStats {
            input_count: 10,
            post_consensus_count: 10,
            avg_confidence: 0.95,
            rs_recovered_count: 2,
            low_confidence_positions: 5,
            success: true,
        };
        let display = format!("{}", stats);
        assert!(display.contains("Repair Pipeline"));
        assert!(display.contains("95.0%"));
    }

    #[test]
    fn test_rs_recovery_stage() {
        let pipeline = RepairPipeline::default_pipeline();

        // Create data and encode with RS
        let rs = ReedSolomon::new(RsConfig::new(2));
        let data = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let parity = rs.encode_block(&data).unwrap();

        // Erase one strand
        let received = vec![
            Some(vec![1, 2, 3]),
            None, // Missing!
            Some(vec![7, 8, 9]),
        ];

        let (recovered, count) = pipeline.run_rs_recovery(&received, &parity).unwrap();
        assert_eq!(count, 1); // One erasure
        assert_eq!(recovered[1], vec![4, 5, 6]); // Recovered!
    }

    #[test]
    fn test_consensus_then_rs_decode_corrects_substitution_with_no_rs_needed() {
        // Two logical strands, each read 3 times; one read of the first
        // strand has a single substitution error. No parity involved --
        // consensus alone should out-vote the corrupted copy.
        let data_groups = vec![
            vec![
                DnaStrand::from_str("ATCG").unwrap(),
                DnaStrand::from_str("ATCG").unwrap(),
                DnaStrand::from_str("ATCC").unwrap(), // last base flipped
            ],
            vec![
                DnaStrand::from_str("GCTA").unwrap(),
                DnaStrand::from_str("GCTA").unwrap(),
            ],
        ];
        let recovered = consensus_then_rs_decode(&data_groups, &[], RsConfig::new(0));
        assert_eq!(recovered[0].to_string(), "ATCG", "majority vote should out-vote the single corrupted read");
        assert_eq!(recovered[1].to_string(), "GCTA");
    }

    #[test]
    fn test_consensus_then_rs_decode_recovers_full_erasure_via_rs() {
        // Strand 1 has zero surviving reads (fully dropped) -- consensus
        // has nothing to vote on, so RS must reconstruct it from parity.
        let rs = ReedSolomon::new(RsConfig::new(2));
        let strands = vec![
            DnaStrand::from_str("AAAA").unwrap(),
            DnaStrand::from_str("CCCC").unwrap(),
            DnaStrand::from_str("GGGG").unwrap(),
        ];
        let strand_bytes: Vec<Vec<u8>> = strands.iter()
            .map(|s| s.bases().iter().map(|n| n.to_bits()).collect())
            .collect();
        let parity_bytes = rs.encode_block(&strand_bytes).unwrap();
        let parity_strands: Vec<DnaStrand> = parity_bytes.iter()
            .map(|p| DnaStrand::new(p.iter().filter_map(|&b| Nucleotide::from_bits(b).ok()).collect()))
            .collect();

        let data_groups = vec![
            vec![strands[0].clone()],
            vec![], // dropped entirely
            vec![strands[2].clone()],
        ];
        let parity_groups: Vec<Vec<DnaStrand>> = parity_strands.iter().map(|p| vec![p.clone()]).collect();

        let recovered = consensus_then_rs_decode(&data_groups, &parity_groups, RsConfig::new(2));
        assert_eq!(recovered.len(), 3);
        assert_eq!(recovered[1].to_string(), "CCCC", "RS should reconstruct the fully-dropped strand");
    }
}
