//! # Error Models for DNA Synthesis and Sequencing
//!
//! Defines the `ErrorModel` trait and implementations for injecting
//! realistic errors into DNA strands. Models the three fundamental
//! error types in molecular biology:
//!
//! - **Substitution**: one base replaced by another (most common in Illumina)
//! - **Insertion**: an extra base inserted (common in Nanopore)
//! - **Deletion**: a base dropped (most common in synthesis)

use nucle_codec::base::{DnaStrand, Nucleotide};
use crate::strand::{SynthStrand, ErrorCounts};
use rand::rngs::StdRng;
use rand::Rng;
use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Error Rate Parameters
// ---------------------------------------------------------------------------

/// Per-base error rates for a specific error channel.
///
/// These rates are specified as probabilities per nucleotide position.
/// For example, a substitution rate of 0.001 means ~0.1% of bases
/// will be substituted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRates {
    /// Probability of substituting one base for another per position.
    pub substitution: f64,
    /// Probability of inserting an extra base after a position.
    pub insertion: f64,
    /// Probability of deleting the base at a position.
    pub deletion: f64,
    /// Probability of the entire strand being dropped from the pool.
    pub strand_dropout: f64,
    /// Probability of the strand being truncated (incomplete synthesis).
    /// If truncated, a random suffix is removed.
    pub truncation: f64,
}

impl ErrorRates {
    /// Zero error rates (pristine).
    pub fn zero() -> Self {
        Self {
            substitution: 0.0,
            insertion: 0.0,
            deletion: 0.0,
            strand_dropout: 0.0,
            truncation: 0.0,
        }
    }

    /// Total per-base error rate (substitution + insertion + deletion).
    pub fn total_per_base(&self) -> f64 {
        self.substitution + self.insertion + self.deletion
    }
}

// ---------------------------------------------------------------------------
// ErrorModel Trait
// ---------------------------------------------------------------------------

/// Trait for error injection models.
///
/// An error model takes a pristine DNA strand and returns a potentially
/// corrupted version with realistic errors injected.
pub trait ErrorModel: Send + Sync {
    /// Name of this error model.
    fn name(&self) -> &str;

    /// Get the error rates for this model.
    fn error_rates(&self) -> &ErrorRates;

    /// Apply errors to a single strand.
    ///
    /// Takes a pristine `SynthStrand` and mutates it according to
    /// the error model's probability distributions.
    fn apply(&self, strand: &mut SynthStrand, rng: &mut StdRng);
}

// ---------------------------------------------------------------------------
// Base Error Model Implementation
// ---------------------------------------------------------------------------

/// Standard error model using configurable error rates.
///
/// Applies independent Bernoulli trials at each base position
/// for substitutions, insertions, and deletions. Also handles
/// strand-level events (dropout, truncation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardErrorModel {
    name: String,
    rates: ErrorRates,
    /// Whether errors are position-dependent (higher at 3' end).
    pub position_dependent: bool,
    /// Factor by which error rate increases at the 3' end (Illumina-like).
    /// 1.0 = uniform, 2.0 = double at the end.
    pub end_degradation_factor: f64,
}

impl StandardErrorModel {
    /// Create a new standard error model.
    pub fn new(name: &str, rates: ErrorRates) -> Self {
        Self {
            name: name.to_string(),
            rates,
            position_dependent: false,
            end_degradation_factor: 1.0,
        }
    }

    /// Enable position-dependent errors (Illumina-like quality degradation).
    pub fn with_position_dependence(mut self, factor: f64) -> Self {
        self.position_dependent = true;
        self.end_degradation_factor = factor;
        self
    }

    /// Calculate the position-adjusted error rate multiplier.
    fn position_factor(&self, pos: usize, total_len: usize) -> f64 {
        if !self.position_dependent || total_len == 0 {
            return 1.0;
        }
        let relative_pos = pos as f64 / total_len as f64;
        // Linear interpolation from 1.0 at start to end_degradation_factor at end
        1.0 + (self.end_degradation_factor - 1.0) * relative_pos
    }

    /// Substitute a nucleotide with a random different base.
    fn random_substitute(original: Nucleotide, rng: &mut StdRng) -> Nucleotide {
        let others: Vec<Nucleotide> = Nucleotide::ALL
            .iter()
            .copied()
            .filter(|&n| n != original)
            .collect();
        others[rng.gen_range(0..3)]
    }

    /// Generate a random nucleotide for insertions.
    fn random_nucleotide(rng: &mut StdRng) -> Nucleotide {
        Nucleotide::ALL[rng.gen_range(0..4)]
    }
}

impl ErrorModel for StandardErrorModel {
    fn name(&self) -> &str {
        &self.name
    }

    fn error_rates(&self) -> &ErrorRates {
        &self.rates
    }

    fn apply(&self, strand: &mut SynthStrand, rng: &mut StdRng) {
        // Check for strand dropout first
        if rng.gen::<f64>() < self.rates.strand_dropout {
            strand.is_intact = false;
            return;
        }

        // Check for truncation
        if rng.gen::<f64>() < self.rates.truncation {
            let bases = strand.sequence.bases_mut();
            if bases.len() > 10 {
                // Remove a random suffix (10-50% of the strand)
                let cut_fraction = rng.gen_range(0.1..0.5);
                let cut_point = (bases.len() as f64 * (1.0 - cut_fraction)) as usize;
                bases.truncate(cut_point.max(1));
                strand.is_truncated = true;
                strand.quality_scores.truncate(bases.len());
            }
        }

        // Apply per-base errors
        let original_bases = strand.sequence.bases().to_vec();
        let total_len = original_bases.len();
        let mut new_bases: Vec<Nucleotide> = Vec::with_capacity(total_len);
        let mut new_quality: Vec<u8> = Vec::with_capacity(total_len);
        let mut errors = ErrorCounts::default();

        for (pos, &base) in original_bases.iter().enumerate() {
            let factor = self.position_factor(pos, total_len);

            // Deletion: skip this base
            if rng.gen::<f64>() < self.rates.deletion * factor {
                errors.deletions += 1;
                continue;
            }

            // Substitution: replace with different base
            if rng.gen::<f64>() < self.rates.substitution * factor {
                let new_base = Self::random_substitute(base, rng);
                new_bases.push(new_base);
                // Lower quality score for substituted bases
                let q = if pos < strand.quality_scores.len() {
                    strand.quality_scores[pos].saturating_sub(20)
                } else {
                    10
                };
                new_quality.push(q);
                errors.substitutions += 1;
            } else {
                new_bases.push(base);
                let q = if pos < strand.quality_scores.len() {
                    strand.quality_scores[pos]
                } else {
                    30
                };
                new_quality.push(q);
            }

            // Insertion: add an extra random base after this position
            if rng.gen::<f64>() < self.rates.insertion * factor {
                new_bases.push(Self::random_nucleotide(rng));
                new_quality.push(5); // Very low quality for inserted bases
                errors.insertions += 1;
            }
        }

        strand.sequence = DnaStrand::new(new_bases);
        strand.quality_scores = new_quality;
        strand.error_count = errors;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn test_zero_error_model() {
        let model = StandardErrorModel::new("zero", ErrorRates::zero());
        let dna = DnaStrand::from_str("ATCGATCGATCGATCG").unwrap();
        let mut synth = SynthStrand::pristine(dna.clone(), 0);
        let mut rng = StdRng::seed_from_u64(42);

        model.apply(&mut synth, &mut rng);

        assert!(synth.is_intact);
        assert!(!synth.has_errors());
        assert_eq!(synth.sequence, dna);
    }

    #[test]
    fn test_high_substitution_rate() {
        let model = StandardErrorModel::new("high-sub", ErrorRates {
            substitution: 0.5, // 50% substitution rate
            insertion: 0.0,
            deletion: 0.0,
            strand_dropout: 0.0,
            truncation: 0.0,
        });

        let dna = DnaStrand::from_str("ATCGATCGATCGATCGATCGATCG").unwrap();
        let mut synth = SynthStrand::pristine(dna.clone(), 0);
        let mut rng = StdRng::seed_from_u64(42);

        model.apply(&mut synth, &mut rng);

        // With 50% sub rate on 24 bases, expect several errors
        assert!(synth.error_count.substitutions > 0);
        assert_eq!(synth.error_count.insertions, 0);
        assert_eq!(synth.error_count.deletions, 0);
        // Length should be preserved (subs don't change length)
        assert_eq!(synth.sequence.len(), dna.len());
    }

    #[test]
    fn test_deletion_shortens_strand() {
        let model = StandardErrorModel::new("high-del", ErrorRates {
            substitution: 0.0,
            insertion: 0.0,
            deletion: 0.3, // 30% deletion rate
            strand_dropout: 0.0,
            truncation: 0.0,
        });

        let dna = DnaStrand::from_str("ATCGATCGATCGATCGATCGATCG").unwrap();
        let original_len = dna.len();
        let mut synth = SynthStrand::pristine(dna, 0);
        let mut rng = StdRng::seed_from_u64(42);

        model.apply(&mut synth, &mut rng);

        assert!(synth.sequence.len() < original_len);
        assert!(synth.error_count.deletions > 0);
    }

    #[test]
    fn test_insertion_lengthens_strand() {
        let model = StandardErrorModel::new("high-ins", ErrorRates {
            substitution: 0.0,
            insertion: 0.3, // 30% insertion rate
            deletion: 0.0,
            strand_dropout: 0.0,
            truncation: 0.0,
        });

        let dna = DnaStrand::from_str("ATCGATCGATCGATCGATCGATCG").unwrap();
        let original_len = dna.len();
        let mut synth = SynthStrand::pristine(dna, 0);
        let mut rng = StdRng::seed_from_u64(42);

        model.apply(&mut synth, &mut rng);

        assert!(synth.sequence.len() > original_len);
        assert!(synth.error_count.insertions > 0);
    }

    #[test]
    fn test_strand_dropout() {
        let model = StandardErrorModel::new("dropout", ErrorRates {
            substitution: 0.0,
            insertion: 0.0,
            deletion: 0.0,
            strand_dropout: 1.0, // 100% dropout
            truncation: 0.0,
        });

        let dna = DnaStrand::from_str("ATCGATCG").unwrap();
        let mut synth = SynthStrand::pristine(dna, 0);
        let mut rng = StdRng::seed_from_u64(42);

        model.apply(&mut synth, &mut rng);

        assert!(!synth.is_intact);
    }

    #[test]
    fn test_position_dependent_errors() {
        let model = StandardErrorModel::new("pos-dep", ErrorRates {
            substitution: 0.1,
            insertion: 0.0,
            deletion: 0.0,
            strand_dropout: 0.0,
            truncation: 0.0,
        }).with_position_dependence(3.0);

        assert!(model.position_dependent);
        // Factor at start should be 1.0, at end should be 3.0
        assert!((model.position_factor(0, 100) - 1.0).abs() < 0.01);
        assert!((model.position_factor(100, 100) - 3.0).abs() < 0.01);
    }
}
