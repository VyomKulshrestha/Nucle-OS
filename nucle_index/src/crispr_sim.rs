//! # CRISPR Random Access Simulation
//!
//! Simulates selective amplification of DNA strands using primer-based
//! targeting, modeling the key aspects of CRISPR-Cas9 random access:
//!
//! - **Specific amplification**: Target strands matching the query primer
//! - **Cross-talk**: Non-specific amplification of similar sequences
//! - **PCR bias**: Amplification efficiency varies by strand composition
//! - **Strand loss**: Some target strands may not amplify

use crate::primer::{PrimerPair, PrimerLibrary};
use nucle_codec::base::{DnaStrand, Nucleotide};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Serialize, Deserialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the CRISPR random access simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrisprConfig {
    /// Probability of successfully amplifying a target strand.
    /// 1.0 = perfect amplification, lower = some targets missed.
    pub amplification_efficiency: f64,

    /// Probability of amplifying a non-target strand (cross-talk).
    /// 0.0 = no cross-talk, higher = more noise.
    pub cross_talk_rate: f64,

    /// Number of PCR cycles to simulate.
    /// More cycles = more copies but also more bias.
    pub pcr_cycles: u32,

    /// Minimum Hamming distance fraction for a primer to "match" a strand.
    /// Lower = more permissive matching = more cross-talk.
    pub match_threshold: f64,

    /// PRNG seed.
    pub seed: u64,
}

impl Default for CrisprConfig {
    fn default() -> Self {
        Self {
            amplification_efficiency: 0.95,
            cross_talk_rate: 0.01,
            pcr_cycles: 1,
            match_threshold: 0.8,
            seed: 42,
        }
    }
}

impl CrisprConfig {
    /// Ideal configuration with no errors.
    pub fn ideal() -> Self {
        Self {
            amplification_efficiency: 1.0,
            cross_talk_rate: 0.0,
            pcr_cycles: 1,
            match_threshold: 1.0,
            seed: 42,
        }
    }

    /// Realistic configuration with moderate cross-talk.
    pub fn realistic() -> Self {
        Self {
            amplification_efficiency: 0.90,
            cross_talk_rate: 0.02,
            pcr_cycles: 1,
            match_threshold: 0.75,
            seed: 42,
        }
    }
}

// ---------------------------------------------------------------------------
// Retrieval Result
// ---------------------------------------------------------------------------

/// Result of a CRISPR retrieval operation.
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    /// Strands that were successfully retrieved (matching the target primer).
    pub target_strands: Vec<DnaStrand>,
    /// Strands retrieved due to cross-talk (non-specific amplification).
    pub crosstalk_strands: Vec<DnaStrand>,
    /// Number of target strands that were in the pool.
    pub total_targets: usize,
    /// Number of target strands successfully amplified.
    pub amplified_targets: usize,
    /// Retrieval precision: targets / (targets + crosstalk).
    pub precision: f64,
    /// Retrieval recall: amplified_targets / total_targets.
    pub recall: f64,
}

impl fmt::Display for RetrievalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "┌─ CRISPR Retrieval Results ──────────")?;
        writeln!(f, "│ Total targets:   {}", self.total_targets)?;
        writeln!(f, "│ Amplified:       {}", self.amplified_targets)?;
        writeln!(f, "│ Cross-talk:      {}", self.crosstalk_strands.len())?;
        writeln!(f, "│ Precision:       {:.1}%", self.precision * 100.0)?;
        writeln!(f, "│ Recall:          {:.1}%", self.recall * 100.0)?;
        write!(f, "└──────────────────────────────────────")
    }
}

// ---------------------------------------------------------------------------
// CRISPR Simulator
// ---------------------------------------------------------------------------

/// Simulates CRISPR-Cas9 random access to DNA storage.
pub struct CrisprSimulator {
    config: CrisprConfig,
}

impl CrisprSimulator {
    /// Create a new simulator.
    pub fn new(config: CrisprConfig) -> Self {
        Self { config }
    }

    /// Create with default settings.
    pub fn default_sim() -> Self {
        Self::new(CrisprConfig::default())
    }

    /// Retrieve strands matching a target primer pair from a pool.
    ///
    /// `pool`: all tagged strands in the storage system.
    /// `target`: the primer pair to select for.
    pub fn retrieve(
        &self,
        pool: &[DnaStrand],
        target: &PrimerPair,
    ) -> RetrievalResult {
        let mut rng = StdRng::seed_from_u64(self.config.seed);
        let mut target_strands = Vec::new();
        let mut crosstalk_strands = Vec::new();
        let mut total_targets = 0;
        let mut amplified_targets = 0;

        for strand in pool {
            let is_target = target.matches_forward(strand);

            if is_target {
                total_targets += 1;
                // Target strand — amplify with configured efficiency
                if rng.gen::<f64>() < self.config.amplification_efficiency {
                    target_strands.push(strand.clone());
                    amplified_targets += 1;
                }
            } else {
                // Non-target — check for cross-talk
                let similarity = self.primer_similarity(strand, target);
                let crosstalk_prob = if similarity > self.config.match_threshold {
                    self.config.cross_talk_rate * similarity
                } else {
                    self.config.cross_talk_rate * 0.01 // Very low background
                };

                if rng.gen::<f64>() < crosstalk_prob {
                    crosstalk_strands.push(strand.clone());
                }
            }
        }

        let total_retrieved = amplified_targets + crosstalk_strands.len();
        let precision = if total_retrieved > 0 {
            amplified_targets as f64 / total_retrieved as f64
        } else {
            1.0
        };
        let recall = if total_targets > 0 {
            amplified_targets as f64 / total_targets as f64
        } else {
            1.0
        };

        RetrievalResult {
            target_strands,
            crosstalk_strands,
            total_targets,
            amplified_targets,
            precision,
            recall,
        }
    }

    /// Compute similarity between a strand's prefix and a primer.
    /// Returns fraction of matching bases (0.0 to 1.0).
    fn primer_similarity(&self, strand: &DnaStrand, primer: &PrimerPair) -> f64 {
        let primer_bases = primer.forward.bases();
        let strand_bases = strand.bases();
        let len = primer_bases.len().min(strand_bases.len());

        if len == 0 {
            return 0.0;
        }

        let matches = primer_bases[..len]
            .iter()
            .zip(strand_bases[..len].iter())
            .filter(|(a, b)| a == b)
            .count();

        matches as f64 / len as f64
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_pool() -> (PrimerLibrary, Vec<DnaStrand>) {
        let library = PrimerLibrary::generate(3, 20, 42);
        let data = DnaStrand::from_str("ATCGATCGATCG").unwrap();

        let mut pool = Vec::new();
        // Add 5 strands for primer 0
        for _ in 0..5 {
            pool.push(library.primers[0].tag_strand(&data));
        }
        // Add 3 strands for primer 1
        for _ in 0..3 {
            pool.push(library.primers[1].tag_strand(&data));
        }
        // Add 2 strands for primer 2
        for _ in 0..2 {
            pool.push(library.primers[2].tag_strand(&data));
        }

        (library, pool)
    }

    #[test]
    fn test_ideal_retrieval() {
        let (library, pool) = make_test_pool();
        let sim = CrisprSimulator::new(CrisprConfig::ideal());

        let result = sim.retrieve(&pool, &library.primers[0]);

        assert_eq!(result.total_targets, 5);
        assert_eq!(result.amplified_targets, 5);
        assert!(result.crosstalk_strands.is_empty());
        assert_eq!(result.precision, 1.0);
        assert_eq!(result.recall, 1.0);
    }

    #[test]
    fn test_partial_amplification() {
        let (library, pool) = make_test_pool();
        let sim = CrisprSimulator::new(CrisprConfig {
            amplification_efficiency: 0.5,
            cross_talk_rate: 0.0,
            ..CrisprConfig::default()
        });

        let result = sim.retrieve(&pool, &library.primers[0]);

        assert_eq!(result.total_targets, 5);
        assert!(result.amplified_targets <= 5);
        assert!(result.crosstalk_strands.is_empty());
    }

    #[test]
    fn test_retrieval_correct_file() {
        let (library, pool) = make_test_pool();
        let sim = CrisprSimulator::new(CrisprConfig::ideal());

        // Retrieve file 1 (3 strands)
        let result = sim.retrieve(&pool, &library.primers[1]);
        assert_eq!(result.total_targets, 3);
        assert_eq!(result.amplified_targets, 3);

        // Retrieve file 2 (2 strands)
        let result = sim.retrieve(&pool, &library.primers[2]);
        assert_eq!(result.total_targets, 2);
        assert_eq!(result.amplified_targets, 2);
    }

    #[test]
    fn test_display() {
        let (library, pool) = make_test_pool();
        let sim = CrisprSimulator::default_sim();
        let result = sim.retrieve(&pool, &library.primers[0]);
        let display = format!("{}", result);
        assert!(display.contains("CRISPR"));
        assert!(display.contains("Precision"));
    }

    #[test]
    fn test_empty_pool() {
        let library = PrimerLibrary::generate(1, 20, 42);
        let sim = CrisprSimulator::default_sim();
        let result = sim.retrieve(&[], &library.primers[0]);

        assert_eq!(result.total_targets, 0);
        assert_eq!(result.amplified_targets, 0);
    }
}
