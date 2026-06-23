//! # Synthesized DNA Strand Types
//!
//! Wraps `DnaStrand` with metadata that tracks the strand's journey
//! through synthesis, storage, and sequencing — quality scores,
//! coverage depth, and provenance information.

use nucle_codec::base::{DnaStrand, Nucleotide};
use serde::{Serialize, Deserialize};

/// A DNA strand that has been through the synthesis/sequencing pipeline,
/// carrying metadata about its quality and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthStrand {
    /// The nucleotide sequence (possibly with errors from synthesis/sequencing).
    pub sequence: DnaStrand,

    /// The original error-free sequence (for validation/debugging).
    /// `None` if this strand was created from real sequencing data.
    pub original: Option<DnaStrand>,

    /// Per-base Phred quality scores (Q-scores).
    /// Q = -10 * log10(error_probability)
    /// Q30 ≈ 99.9% accuracy, Q20 ≈ 99%, Q10 ≈ 90%
    pub quality_scores: Vec<u8>,

    /// Number of times this strand was sequenced (coverage depth).
    /// Higher coverage enables consensus correction.
    pub coverage: u32,

    /// Unique identifier for this strand in the pool.
    pub strand_id: u64,

    /// Whether this strand survived the full pipeline (not dropped).
    pub is_intact: bool,

    /// Whether this strand was truncated during synthesis.
    pub is_truncated: bool,

    /// Number of errors introduced (for simulation tracking).
    pub error_count: ErrorCounts,
}

/// Counts of each error type introduced into a strand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorCounts {
    pub substitutions: usize,
    pub insertions: usize,
    pub deletions: usize,
}

impl ErrorCounts {
    pub fn total(&self) -> usize {
        self.substitutions + self.insertions + self.deletions
    }
}

impl SynthStrand {
    /// Create a pristine (error-free) synth strand from a raw DNA strand.
    pub fn pristine(sequence: DnaStrand, strand_id: u64) -> Self {
        let len = sequence.len();
        Self {
            original: Some(sequence.clone()),
            sequence,
            quality_scores: vec![40; len], // Q40 = very high quality
            coverage: 1,
            strand_id,
            is_intact: true,
            is_truncated: false,
            error_count: ErrorCounts::default(),
        }
    }

    /// Create a new synth strand with specified quality.
    pub fn new(
        sequence: DnaStrand,
        original: Option<DnaStrand>,
        quality_scores: Vec<u8>,
        strand_id: u64,
    ) -> Self {
        Self {
            sequence,
            original,
            quality_scores,
            coverage: 1,
            strand_id,
            is_intact: true,
            is_truncated: false,
            error_count: ErrorCounts::default(),
        }
    }

    /// Length of the (possibly mutated) sequence.
    pub fn len(&self) -> usize {
        self.sequence.len()
    }

    /// Whether the sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.sequence.is_empty()
    }

    /// Check if this strand has any errors compared to the original.
    pub fn has_errors(&self) -> bool {
        self.error_count.total() > 0
    }

    /// Compute the error rate (errors / original length).
    pub fn error_rate(&self) -> f64 {
        if let Some(ref orig) = self.original {
            if orig.len() == 0 {
                return 0.0;
            }
            self.error_count.total() as f64 / orig.len() as f64
        } else {
            0.0
        }
    }

    /// Average quality score.
    pub fn avg_quality(&self) -> f64 {
        if self.quality_scores.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.quality_scores.iter().map(|&q| q as u64).sum();
        sum as f64 / self.quality_scores.len() as f64
    }

    /// Check if a base at a given position matches the original.
    pub fn base_matches_original(&self, pos: usize) -> Option<bool> {
        let orig = self.original.as_ref()?;
        let orig_base = orig.get(pos)?;
        let curr_base = self.sequence.get(pos)?;
        Some(orig_base == curr_base)
    }
}

/// A collection of synthesized strands representing a pool of DNA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthPool {
    /// All strands in the pool (including errored/truncated ones).
    pub strands: Vec<SynthStrand>,
}

impl SynthPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            strands: Vec::new(),
        }
    }

    /// Create a pool from existing strands.
    pub fn from_strands(strands: Vec<SynthStrand>) -> Self {
        Self { strands }
    }

    /// Number of strands in the pool.
    pub fn len(&self) -> usize {
        self.strands.len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.strands.is_empty()
    }

    /// Number of intact (non-dropped, non-truncated) strands.
    pub fn intact_count(&self) -> usize {
        self.strands.iter().filter(|s| s.is_intact && !s.is_truncated).count()
    }

    /// Fraction of strands that survived intact.
    pub fn survival_rate(&self) -> f64 {
        if self.strands.is_empty() {
            return 0.0;
        }
        self.intact_count() as f64 / self.strands.len() as f64
    }

    /// Average error rate across all intact strands.
    pub fn avg_error_rate(&self) -> f64 {
        let intact: Vec<&SynthStrand> = self.strands.iter()
            .filter(|s| s.is_intact)
            .collect();
        if intact.is_empty() {
            return 0.0;
        }
        let total: f64 = intact.iter().map(|s| s.error_rate()).sum();
        total / intact.len() as f64
    }

    /// Total errors across all strands by type.
    pub fn total_errors(&self) -> ErrorCounts {
        let mut total = ErrorCounts::default();
        for s in &self.strands {
            total.substitutions += s.error_count.substitutions;
            total.insertions += s.error_count.insertions;
            total.deletions += s.error_count.deletions;
        }
        total
    }

    /// Extract intact strands back to a StrandCollection for decoding.
    pub fn to_strand_collection(&self, original_size: usize) -> nucle_codec::base::StrandCollection {
        let strands: Vec<DnaStrand> = self.strands
            .iter()
            .filter(|s| s.is_intact && !s.is_truncated)
            .map(|s| s.sequence.clone())
            .collect();
        nucle_codec::base::StrandCollection::from_strands(strands, original_size)
    }
}

impl Default for SynthPool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pristine_strand() {
        let dna = DnaStrand::from_str("ATCGATCG").unwrap();
        let synth = SynthStrand::pristine(dna.clone(), 0);

        assert_eq!(synth.len(), 8);
        assert!(synth.is_intact);
        assert!(!synth.is_truncated);
        assert!(!synth.has_errors());
        assert_eq!(synth.error_rate(), 0.0);
        assert_eq!(synth.avg_quality(), 40.0);
        assert_eq!(synth.original.unwrap(), dna);
    }

    #[test]
    fn test_synth_pool_stats() {
        let s1 = SynthStrand::pristine(DnaStrand::from_str("ATCG").unwrap(), 0);
        let mut s2 = SynthStrand::pristine(DnaStrand::from_str("GCTA").unwrap(), 1);
        s2.is_intact = false; // Dropped

        let pool = SynthPool::from_strands(vec![s1, s2]);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.intact_count(), 1);
        assert!((pool.survival_rate() - 0.5).abs() < f64::EPSILON);
    }
}
