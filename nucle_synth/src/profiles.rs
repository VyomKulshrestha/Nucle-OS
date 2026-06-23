//! # Hardware Profiles for DNA Synthesizers and Sequencers
//!
//! Pre-configured error profiles that mimic real-world hardware:
//!
//! | Platform | Dominant Error | Per-base Rate |
//! |----------|---------------|---------------|
//! | Illumina | Substitutions | ~0.1% |
//! | Oxford Nanopore | Indels (homopolymers) | ~3-5% |
//! | Twist Bioscience | Deletions | ~0.03% |
//! | Custom | User-defined | Variable |
//!
//! Each profile is a pre-configured `StandardErrorModel` with rates
//! calibrated from published error characterization studies.

use crate::errors::{ErrorRates, StandardErrorModel};
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
}

impl HardwareProfile {
    /// Get the error rates for this hardware profile.
    pub fn error_rates(&self) -> ErrorRates {
        match self {
            HardwareProfile::Illumina => ErrorRates {
                substitution: 0.001,    // ~0.1% (Q30)
                insertion: 0.0001,      // ~0.01%
                deletion: 0.0001,       // ~0.01%
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
        }
    }

    /// Create a StandardErrorModel configured for this hardware.
    pub fn to_error_model(&self) -> StandardErrorModel {
        let model = StandardErrorModel::new(self.name(), self.error_rates());

        // Illumina has position-dependent quality degradation
        if *self == HardwareProfile::Illumina {
            model.with_position_dependence(2.0)
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
