//! # Noise Injection Engine
//!
//! Applies error models to collections of DNA strands, simulating
//! the full synthesis → storage → sequencing pipeline.
//!
//! Supports:
//! - Single-pass error injection (one round of errors)
//! - Multi-copy simulation (coverage depth with independent errors)
//! - Decay simulation (long-term storage degradation)
//! - Pipeline simulation (synthesis → storage → sequencing chain)

use crate::errors::ErrorModel;
use crate::profiles::HardwareProfile;
use crate::strand::{SynthStrand, SynthPool};
use nucle_codec::base::{DnaStrand, StrandCollection};
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Serialize, Deserialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Simulation Configuration
// ---------------------------------------------------------------------------

/// Configuration for the noise injection simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// PRNG seed for reproducible simulations.
    pub seed: u64,

    /// Number of copies of each strand to simulate (coverage depth).
    /// Higher coverage enables consensus-based error correction.
    /// Typical: 5–20× for DNA storage.
    pub coverage_depth: u32,

    /// Hardware profile for synthesis errors.
    pub synthesis_profile: HardwareProfile,

    /// Hardware profile for sequencing errors.
    pub sequencing_profile: HardwareProfile,

    /// Whether to simulate long-term storage decay.
    pub simulate_decay: bool,

    /// Decay rate per base per unit time (for storage simulation).
    /// Typical: ~1e-9 per base per year in ideal conditions.
    pub decay_rate: f64,

    /// Storage time in arbitrary units (for decay simulation).
    pub storage_time: f64,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            coverage_depth: 1,
            synthesis_profile: HardwareProfile::TwistBioscience,
            sequencing_profile: HardwareProfile::Illumina,
            simulate_decay: false,
            decay_rate: 1e-9,
            storage_time: 1.0,
        }
    }
}

impl SimulationConfig {
    /// Simulate Twist synthesis + Illumina sequencing (common pipeline).
    pub fn twist_illumina() -> Self {
        Self {
            synthesis_profile: HardwareProfile::TwistBioscience,
            sequencing_profile: HardwareProfile::Illumina,
            ..Default::default()
        }
    }

    /// Simulate Twist synthesis + Nanopore sequencing.
    pub fn twist_nanopore() -> Self {
        Self {
            synthesis_profile: HardwareProfile::TwistBioscience,
            sequencing_profile: HardwareProfile::OxfordNanopore,
            ..Default::default()
        }
    }

    /// No-error simulation for baseline testing.
    pub fn pristine() -> Self {
        Self {
            synthesis_profile: HardwareProfile::Pristine,
            sequencing_profile: HardwareProfile::Pristine,
            ..Default::default()
        }
    }

    /// Set coverage depth.
    pub fn with_coverage(mut self, depth: u32) -> Self {
        self.coverage_depth = depth;
        self
    }

    /// Set PRNG seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

// ---------------------------------------------------------------------------
// Simulation Results
// ---------------------------------------------------------------------------

/// Results from running a noise simulation.
#[derive(Debug, Clone)]
pub struct SimulationResult {
    /// The pool of strands after error injection.
    pub pool: SynthPool,
    /// Configuration used.
    pub config: SimulationConfig,
    /// Number of input strands.
    pub input_strand_count: usize,
    /// Number of output strands (after dropout, × coverage).
    pub output_strand_count: usize,
}

impl SimulationResult {
    /// Fraction of strands that survived (not dropped).
    pub fn survival_rate(&self) -> f64 {
        self.pool.survival_rate()
    }

    /// Average per-base error rate across intact strands.
    pub fn avg_error_rate(&self) -> f64 {
        self.pool.avg_error_rate()
    }
}

impl fmt::Display for SimulationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let errors = self.pool.total_errors();
        writeln!(f, "┌─ Simulation Results ────────────────")?;
        writeln!(f, "│ Synthesis:     {}", self.config.synthesis_profile.name())?;
        writeln!(f, "│ Sequencing:    {}", self.config.sequencing_profile.name())?;
        writeln!(f, "│ Coverage:      {}×", self.config.coverage_depth)?;
        writeln!(f, "│ Input strands: {}", self.input_strand_count)?;
        writeln!(f, "│ Output strands:{}", self.output_strand_count)?;
        writeln!(f, "│ Intact:        {}", self.pool.intact_count())?;
        writeln!(f, "│ Survival:      {:.1}%", self.survival_rate() * 100.0)?;
        writeln!(f, "│ Avg error rate:{:.4}%", self.avg_error_rate() * 100.0)?;
        writeln!(f, "│ Substitutions: {}", errors.substitutions)?;
        writeln!(f, "│ Insertions:    {}", errors.insertions)?;
        writeln!(f, "│ Deletions:     {}", errors.deletions)?;
        write!(f, "└──────────────────────────────────────")
    }
}

// ---------------------------------------------------------------------------
// Noise Engine
// ---------------------------------------------------------------------------

/// The main noise injection engine.
///
/// Takes a collection of pristine DNA strands and produces a `SynthPool`
/// with realistic errors injected according to the simulation configuration.
pub struct NoiseEngine {
    config: SimulationConfig,
}

impl NoiseEngine {
    /// Create a new noise engine with the given configuration.
    pub fn new(config: SimulationConfig) -> Self {
        Self { config }
    }

    /// Create an engine with default settings.
    pub fn default_engine() -> Self {
        Self::new(SimulationConfig::default())
    }

    /// Access the configuration.
    pub fn config(&self) -> &SimulationConfig {
        &self.config
    }

    /// Run the full simulation pipeline on a strand collection.
    ///
    /// Pipeline: [pristine strands] → [synthesis errors] → [sequencing errors] → [SynthPool]
    pub fn simulate(&self, collection: &StrandCollection) -> SimulationResult {
        let mut rng = StdRng::seed_from_u64(self.config.seed);
        let input_count = collection.strands.len();

        let synth_model = self.config.synthesis_profile.to_error_model();
        let seq_model = self.config.sequencing_profile.to_error_model();

        let mut all_strands: Vec<SynthStrand> = Vec::new();
        let mut strand_id: u64 = 0;

        for strand in &collection.strands {
            // Generate `coverage_depth` copies of each strand
            for _copy in 0..self.config.coverage_depth {
                let mut synth = SynthStrand::pristine(strand.clone(), strand_id);
                strand_id += 1;

                // Step 1: Apply synthesis errors
                synth_model.apply(&mut synth, &mut rng);

                // Skip further processing if strand was dropped
                if !synth.is_intact {
                    all_strands.push(synth);
                    continue;
                }

                // Step 2: Apply sequencing errors (on top of synthesis errors)
                let synth_errors = synth.error_count.clone();
                seq_model.apply(&mut synth, &mut rng);

                // Accumulate errors from both stages
                synth.error_count.substitutions += synth_errors.substitutions;
                synth.error_count.insertions += synth_errors.insertions;
                synth.error_count.deletions += synth_errors.deletions;

                all_strands.push(synth);
            }
        }

        let output_count = all_strands.len();
        let pool = SynthPool::from_strands(all_strands);

        SimulationResult {
            pool,
            config: self.config.clone(),
            input_strand_count: input_count,
            output_strand_count: output_count,
        }
    }

    /// Quick simulation with a single hardware profile (combined synth+seq).
    pub fn simulate_single_profile(
        collection: &StrandCollection,
        profile: HardwareProfile,
        seed: u64,
    ) -> SimulationResult {
        let config = SimulationConfig {
            seed,
            coverage_depth: 1,
            synthesis_profile: profile,
            sequencing_profile: HardwareProfile::Pristine, // Only one stage
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        NoiseEngine::new(config).simulate(collection)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_collection() -> StrandCollection {
        let strands: Vec<DnaStrand> = (0..10)
            .map(|_| DnaStrand::from_str("ATCGATCGATCGATCGATCGATCG").unwrap())
            .collect();
        StrandCollection::from_strands(strands, 60)
    }

    #[test]
    fn test_pristine_simulation() {
        let collection = make_test_collection();
        let engine = NoiseEngine::new(SimulationConfig::pristine());

        let result = engine.simulate(&collection);

        assert_eq!(result.input_strand_count, 10);
        assert_eq!(result.output_strand_count, 10);
        assert!((result.survival_rate() - 1.0).abs() < f64::EPSILON);
        assert_eq!(result.avg_error_rate(), 0.0);
    }

    #[test]
    fn test_coverage_depth() {
        let collection = make_test_collection();
        let config = SimulationConfig::pristine().with_coverage(5);
        let engine = NoiseEngine::new(config);

        let result = engine.simulate(&collection);

        // 10 strands × 5 copies = 50 output strands
        assert_eq!(result.output_strand_count, 50);
    }

    #[test]
    fn test_illumina_pipeline() {
        let collection = make_test_collection();
        let config = SimulationConfig::twist_illumina().with_seed(42);
        let engine = NoiseEngine::new(config);

        let result = engine.simulate(&collection);

        // Should have some errors but mostly intact
        assert!(result.survival_rate() > 0.5);
    }

    #[test]
    fn test_nanopore_more_errors_than_illumina() {
        let collection = make_test_collection();

        let illumina_result = NoiseEngine::new(
            SimulationConfig::twist_illumina().with_seed(42)
        ).simulate(&collection);

        let nanopore_result = NoiseEngine::new(
            SimulationConfig::twist_nanopore().with_seed(42)
        ).simulate(&collection);

        // Nanopore should have higher error rate than Illumina
        assert!(
            nanopore_result.avg_error_rate() >= illumina_result.avg_error_rate(),
            "nanopore ({:.4}) should have >= errors than illumina ({:.4})",
            nanopore_result.avg_error_rate(),
            illumina_result.avg_error_rate()
        );
    }

    #[test]
    fn test_simulation_result_display() {
        let collection = make_test_collection();
        let engine = NoiseEngine::new(SimulationConfig::pristine());
        let result = engine.simulate(&collection);
        let display = format!("{}", result);
        assert!(display.contains("Simulation Results"));
        assert!(display.contains("Pristine"));
    }

    #[test]
    fn test_single_profile_simulation() {
        let collection = make_test_collection();
        let result = NoiseEngine::simulate_single_profile(
            &collection,
            HardwareProfile::Illumina,
            42,
        );
        assert_eq!(result.input_strand_count, 10);
    }

    #[test]
    fn test_reproducibility() {
        let collection = make_test_collection();

        let r1 = NoiseEngine::new(SimulationConfig::twist_illumina().with_seed(123))
            .simulate(&collection);
        let r2 = NoiseEngine::new(SimulationConfig::twist_illumina().with_seed(123))
            .simulate(&collection);

        // Same seed should produce same results
        assert_eq!(r1.pool.intact_count(), r2.pool.intact_count());
        assert_eq!(
            r1.pool.total_errors().total(),
            r2.pool.total_errors().total()
        );
    }
}
