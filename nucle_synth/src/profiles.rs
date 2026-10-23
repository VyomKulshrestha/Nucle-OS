//! # Hardware Profiles for DNA Synthesizers and Sequencers
//!
//! Pre-configured error profiles that mimic real-world hardware:
//!
//! | Platform | Dominant Error | Per-base Rate |
//! |----------|---------------|---------------|
//! | Illumina | Substitutions | ~0.1% |
//! | Oxford Nanopore | Indels (homopolymers) | ~3-5% |
//! | Twist Bioscience | Deletions | ~0.03% |
//! | Array pool, high fidelity (measured) | Substitutions (G>T) | ~0.11% |
//! | Array pool, low fidelity (measured) | Transitions | ~1.5% |
//! | Custom | User-defined | Variable |
//!
//! Each profile is a pre-configured `StandardErrorModel` with rates
//! calibrated from published error characterization studies.
//!
//! The two array-pool profiles are measured end to end (synthesis, PCR and
//! Illumina iSeq 100 sequencing together) on study ERP173549 / PRJEB90546
//! (Gimpel et al. 2026, dt4dds benchmark pool): runs ERR15127739 (high
//! fidelity, 692,992 read pairs) and ERR15127744 (low fidelity, 1,041,806
//! pairs), read 1, the 10,833 non-repetitive 129 nt references. Besides
//! their per-base rates they carry the measured substitution spectrum,
//! position ramps, first-base spike, homopolymer indel factors and
//! log-normal coverage spread ([`HardwareProfile::coverage_log_sigma`]).
//! The same runs put Illumina's sequencing-attributed indels at
//! 2.8e-6 to 1.9e-5 per base, which is what the Illumina profile uses.

use crate::errors::{ErrorRates, MeasuredFeatures, StandardErrorModel};
use serde::{Serialize, Deserialize};

/// Enumeration of known hardware platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardwareProfile {
    /// Illumina short-read sequencing (e.g., NovaSeq, MiSeq).
    /// Low error rate, substitution-dominant, quality degrades at 3' end.
    Illumina,

    /// Oxford Nanopore long-read sequencing (R10.4 + SUP basecalling).
    /// Higher error rate, indel-dominant, especially in homopolymers.
    OxfordNanopore,

    /// Twist Bioscience silicon-chip oligo synthesis.
    /// Very low error rate, deletion-dominant.
    TwistBioscience,

    /// IDT oPools array-based synthesis.
    /// Low error rate, deletion-dominant.
    Idt,

    /// Traditional column-based phosphoramidite synthesis.
    /// Higher error rate, deletion + truncation dominant.
    ColumnSynthesis,

    /// No errors — pristine channel for baseline testing.
    Pristine,

    /// A high-fidelity array oligo pool, measured end to end on real reads
    /// (ERR15127739). Already includes sequencing.
    ArrayHighFidelity,

    /// A low-fidelity array oligo pool, measured end to end on real reads
    /// (ERR15127744): about 14x the high-fidelity error, transition-dominated
    /// and almost entirely in the molecules. Already includes sequencing.
    ArrayLowFidelity,
}

impl HardwareProfile {
    /// Get the error rates for this hardware profile.
    pub fn error_rates(&self) -> ErrorRates {
        match self {
            HardwareProfile::Illumina => ErrorRates {
                substitution: 0.001,    // ~0.1% (Q30); measured 2.8e-4 (R1) to 7.1e-4 (R2)
                insertion: 0.00002,     // measured sequencing-attributed 2.8e-6 to 1.9e-5
                deletion: 0.00002,      // measured sequencing-attributed 4.8e-6 to 1.9e-5
                strand_dropout: 0.02,   // ~2% strand loss
                truncation: 0.0,        // Not applicable (sequencing, not synthesis)
            },

            HardwareProfile::OxfordNanopore => ErrorRates {
                substitution: 0.03,     // ~3% (R10.4 + SUP)
                insertion: 0.02,        // ~2%
                deletion: 0.02,         // ~2%
                strand_dropout: 0.03,   // ~3%
                truncation: 0.0,
            },

            HardwareProfile::TwistBioscience => ErrorRates {
                substitution: 0.0001,   // ~0.01%
                insertion: 0.00005,     // ~0.005%
                deletion: 0.0003,       // ~0.03% (dominant error)
                strand_dropout: 0.01,   // ~1%
                truncation: 0.005,      // ~0.5% truncation
            },

            HardwareProfile::Idt => ErrorRates {
                substitution: 0.0002,   // ~0.02%
                insertion: 0.0001,      // ~0.01%
                deletion: 0.0005,       // ~0.05%
                strand_dropout: 0.015,  // ~1.5%
                truncation: 0.008,      // ~0.8%
            },

            HardwareProfile::ColumnSynthesis => ErrorRates {
                substitution: 0.002,    // ~0.2%
                insertion: 0.001,       // ~0.1%
                deletion: 0.005,        // ~0.5% (dominant)
                strand_dropout: 0.03,   // ~3%
                truncation: 0.02,       // ~2% (incomplete coupling)
            },

            HardwareProfile::Pristine => ErrorRates::zero(),

            // Run-of-1 indel rates; homopolymer factors raise longer runs.
            // Dropout = references with no reads at all.
            HardwareProfile::ArrayHighFidelity => ErrorRates {
                substitution: 7.47e-4,
                insertion: 5.52e-5,
                deletion: 2.63e-4,
                strand_dropout: 81.0 / 10_833.0,
                truncation: 0.0,
            },

            HardwareProfile::ArrayLowFidelity => ErrorRates {
                substitution: 9.82e-3,
                insertion: 7.28e-4,
                deletion: 4.23e-3,
                strand_dropout: 58.0 / 10_833.0,
                truncation: 0.0,
            },
        }
    }

    /// Standard deviation of ln(reads per strand / mean) this platform
    /// produces; 0 means every strand gets the same coverage. Measured
    /// 0.77 and 1.14 for the two array pools.
    pub fn coverage_log_sigma(&self) -> f64 {
        match self {
            HardwareProfile::ArrayHighFidelity => 0.77,
            HardwareProfile::ArrayLowFidelity => 1.14,
            _ => 0.0,
        }
    }

    /// Whether this profile was measured end to end, sequencing included,
    /// so no separate sequencing stage should be added on top.
    pub fn includes_sequencing(&self) -> bool {
        matches!(self, HardwareProfile::ArrayHighFidelity | HardwareProfile::ArrayLowFidelity)
    }

    /// The measured channel structure of the array-pool profiles.
    fn measured_features(&self) -> Option<MeasuredFeatures> {
        // Substitution rates per reference base, rows/columns A, T, G, C.
        match self {
            HardwareProfile::ArrayHighFidelity => Some(MeasuredFeatures {
                substitution: [
                    [0.0, 2.669e-4, 3.517e-4, 2.86e-5],
                    [1.405e-4, 0.0, 5.18e-5, 1.650e-4],
                    [2.563e-4, 6.942e-4, 0.0, 5.49e-5],
                    [4.136e-4, 4.785e-4, 7.86e-5, 0.0],
                ],
                first_base_sub_factor: 6.5,
                sub_ramp: (0.7, 1.3),
                ins_ramp: (1.0, 1.0),
                del_ramp: (1.0, 1.0),
                homopolymer_del: [1.0, 1.14, 1.10, 1.19, 1.11, 2.42],
                homopolymer_ins: [1.0, 1.03, 0.99, 1.02, 1.11, 3.11],
            }),
            HardwareProfile::ArrayLowFidelity => Some(MeasuredFeatures {
                substitution: [
                    [0.0, 5.075e-3, 7.048e-3, 8.698e-4],
                    [1.825e-3, 0.0, 5.759e-4, 7.713e-3],
                    [5.565e-3, 1.887e-3, 0.0, 6.505e-4],
                    [6.645e-4, 7.360e-3, 4.806e-4, 0.0],
                ],
                first_base_sub_factor: 1.33,
                sub_ramp: (0.85, 1.12),
                ins_ramp: (1.35, 0.62),
                del_ramp: (1.1, 0.95),
                homopolymer_del: [1.0, 1.04, 1.07, 1.10, 1.21, 1.75],
                homopolymer_ins: [1.0, 1.80, 2.06, 2.29, 2.35, 3.03],
            }),
            _ => None,
        }
    }

    /// Create a StandardErrorModel configured for this hardware.
    pub fn to_error_model(&self) -> StandardErrorModel {
        let model = StandardErrorModel::new(self.name(), self.error_rates());

        // Illumina has position-dependent quality degradation
        if *self == HardwareProfile::Illumina {
            model.with_position_dependence(2.0)
        } else if let Some(features) = self.measured_features() {
            model.with_measured(features)
        } else {
            model
        }
    }

    /// Human-readable name of this platform.
    pub fn name(&self) -> &str {
        match self {
            HardwareProfile::Illumina => "Illumina",
            HardwareProfile::OxfordNanopore => "Oxford Nanopore",
            HardwareProfile::TwistBioscience => "Twist Bioscience",
            HardwareProfile::Idt => "IDT oPools",
            HardwareProfile::ColumnSynthesis => "Column Synthesis",
            HardwareProfile::Pristine => "Pristine (no errors)",
            HardwareProfile::ArrayHighFidelity => "Array pool, high fidelity (measured)",
            HardwareProfile::ArrayLowFidelity => "Array pool, low fidelity (measured)",
        }
    }

    /// All available profiles.
    pub fn all() -> Vec<HardwareProfile> {
        vec![
            HardwareProfile::Illumina,
            HardwareProfile::OxfordNanopore,
            HardwareProfile::TwistBioscience,
            HardwareProfile::Idt,
            HardwareProfile::ColumnSynthesis,
            HardwareProfile::Pristine,
            HardwareProfile::ArrayHighFidelity,
            HardwareProfile::ArrayLowFidelity,
        ]
    }
}

/// Builder for custom hardware profiles with user-defined error rates.
#[derive(Debug, Clone)]
pub struct CustomProfileBuilder {
    name: String,
    rates: ErrorRates,
    position_dependent: bool,
    end_degradation: f64,
}

impl CustomProfileBuilder {
    /// Start building a custom profile with a name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            rates: ErrorRates::zero(),
            position_dependent: false,
            end_degradation: 1.0,
        }
    }

    /// Set substitution rate.
    pub fn substitution(mut self, rate: f64) -> Self {
        self.rates.substitution = rate;
        self
    }

    /// Set insertion rate.
    pub fn insertion(mut self, rate: f64) -> Self {
        self.rates.insertion = rate;
        self
    }

    /// Set deletion rate.
    pub fn deletion(mut self, rate: f64) -> Self {
        self.rates.deletion = rate;
        self
    }

    /// Set strand dropout rate.
    pub fn strand_dropout(mut self, rate: f64) -> Self {
        self.rates.strand_dropout = rate;
        self
    }

    /// Set truncation rate.
    pub fn truncation(mut self, rate: f64) -> Self {
        self.rates.truncation = rate;
        self
    }

    /// Enable position-dependent error rates.
    pub fn position_dependent(mut self, end_factor: f64) -> Self {
        self.position_dependent = true;
        self.end_degradation = end_factor;
        self
    }

    /// Build the error model.
    pub fn build(self) -> StandardErrorModel {
        let model = StandardErrorModel::new(&self.name, self.rates);
        if self.position_dependent {
            model.with_position_dependence(self.end_degradation)
        } else {
            model
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorModel;

    #[test]
    fn test_all_profiles_have_names() {
        for profile in HardwareProfile::all() {
            assert!(!profile.name().is_empty());
        }
    }

    #[test]
    fn test_illumina_profile() {
        let rates = HardwareProfile::Illumina.error_rates();
        // Illumina: substitution dominant, very low indel
        assert!(rates.substitution > rates.insertion);
        assert!(rates.substitution > rates.deletion);
        assert!(rates.total_per_base() < 0.01); // < 1% total
    }

    #[test]
    fn test_nanopore_profile() {
        let rates = HardwareProfile::OxfordNanopore.error_rates();
        // Nanopore: higher overall, indel-heavy
        assert!(rates.total_per_base() > 0.01); // > 1% total
        assert!(rates.insertion > 0.01);
        assert!(rates.deletion > 0.01);
    }

    #[test]
    fn test_pristine_profile() {
        let rates = HardwareProfile::Pristine.error_rates();
        assert_eq!(rates.total_per_base(), 0.0);
        assert_eq!(rates.strand_dropout, 0.0);
    }

    #[test]
    fn test_custom_profile_builder() {
        let model = CustomProfileBuilder::new("my-hardware")
            .substitution(0.005)
            .insertion(0.002)
            .deletion(0.003)
            .strand_dropout(0.01)
            .position_dependent(2.5)
            .build();

        assert_eq!(model.name(), "my-hardware");
        let rates = model.error_rates();
        assert!((rates.substitution - 0.005).abs() < f64::EPSILON);
        assert!(model.position_dependent);
    }

    #[test]
    fn test_illumina_position_dependent() {
        let model = HardwareProfile::Illumina.to_error_model();
        assert!(model.position_dependent);
    }

    #[test]
    fn test_twist_not_position_dependent() {
        let model = HardwareProfile::TwistBioscience.to_error_model();
        assert!(!model.position_dependent);
    }
}
