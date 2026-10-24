//! # Storage Decay
//!
//! Stored DNA breaks. Each phosphodiester bond fails as a first-order
//! process whose rate rises with temperature following the Arrhenius law
//!
//! ```text
//! k(T) = k_ref · exp(−Ea/R · (1/T − 1/T_ref))
//! ```
//!
//! where `k` is breaks per nucleotide per year. A stored strand is readable
//! only while intact (PCR needs both primer sites on one molecule), so a
//! strand of `L` nucleotides survives `t` years with probability
//! `exp(−k · L · t)`. A sequence is lost once every physical copy of it
//! has broken; a file is lost once more sequences are gone than its
//! Reed-Solomon parity can rebuild.
//!
//! The built-in media carry published kinetics, not fitted guesses:
//!
//! - [`StorageMedium::Encapsulated`]: purified DNA sealed dry and anoxic in
//!   stainless-steel capsules. Coudy et al., "Long term conservation of DNA
//!   at ambient temperature. Implications for DNA data storage", PLoS ONE
//!   16(11): e0259868 (2021): Ea = 197 kJ/mol and 3.82 × 10⁻¹⁵ breaks per
//!   nucleotide per second at 25 °C, measured by accelerated aging at
//!   100–140 °C.
//! - [`StorageMedium::Bone`]: DNA in buried bone, the natural reference.
//!   Allentoft et al., "The half-life of DNA in bone: measuring decay
//!   kinetics in 158 dated fossils", Proc. R. Soc. B 279: 4724–4733
//!   (2012): 5.50 × 10⁻⁶ breaks per nucleotide per year at an effective
//!   13.1 °C, with Ea = 127 kJ/mol taken from in vitro depurination.
//! - [`StorageMedium::Silica`]: DNA encapsulated in silica particles. Grass
//!   et al., "Robust chemical preservation of digital information on DNA
//!   in silica with error-correcting codes", Angew. Chem. Int. Ed. 54:
//!   2552–2555 (2015) aged 158 nt strands in silica at 70 °C and 50%
//!   relative humidity and report Ea = 155 kJ/mol. Their week at 70 °C was
//!   four strand half-lives (Heckel, Mikutis and Grass, Sci. Rep. 9: 9663
//!   (2019)), which is 2.9 × 10⁻⁸ breaks per nucleotide per second; W. D.
//!   Chen's 2019 ETH Zurich thesis tabulates the same data as 4.37 × 10⁻⁶
//!   s⁻¹ for a 150 nt strand. Independent readings of Grass's data (Coudy
//!   et al. 2021, Fig. 6; Doricchi et al., ACS Nano 2022, Table 1) agree
//!   within a factor of 3, which is about how far to trust the rate.
//!
//! All three are extrapolations below the temperatures they were measured
//! at, as their authors state; [`StorageMedium::Custom`] takes any other
//! measured kinetics.

use serde::{Deserialize, Serialize};

const GAS_CONSTANT_J_PER_MOL_K: f64 = 8.314_462_618;
const SECONDS_PER_YEAR: f64 = 365.25 * 24.0 * 3600.0;
const KELVIN_OFFSET: f64 = 273.15;

/// Where and how the DNA is kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StorageMedium {
    /// Dry, anoxic encapsulation (Coudy et al. 2021).
    Encapsulated,
    /// Buried bone (Allentoft et al. 2012).
    Bone,
    /// Silica particles (Grass et al. 2015).
    Silica,
    /// Caller-supplied kinetics.
    Custom {
        /// Activation energy, kJ/mol.
        activation_energy_kj_mol: f64,
        /// Breaks per nucleotide per year at `reference_celsius`.
        rate_per_nt_year: f64,
        /// Temperature the rate was measured at, °C.
        reference_celsius: f64,
    },
}

/// The Arrhenius parameters behind a medium.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Kinetics {
    pub activation_energy_kj_mol: f64,
    pub rate_per_nt_year: f64,
    pub reference_celsius: f64,
}

impl StorageMedium {
    /// Parse a CLI-style name: `encapsulated` (aliases `capsule`, `dnashell`),
    /// `bone` (alias `fossil`) or `silica` (alias `sio2`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "encapsulated" | "capsule" | "dnashell" => Some(Self::Encapsulated),
            "bone" | "fossil" => Some(Self::Bone),
            "silica" | "sio2" => Some(Self::Silica),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Encapsulated => "encapsulated",
            Self::Bone => "bone",
            Self::Silica => "silica",
            Self::Custom { .. } => "custom",
        }
    }

    /// Where the kinetics come from.
    pub fn citation(&self) -> &'static str {
        match self {
            Self::Encapsulated => "Coudy et al., PLoS ONE 16(11): e0259868 (2021)",
            Self::Bone => "Allentoft et al., Proc. R. Soc. B 279: 4724-4733 (2012)",
            Self::Silica => "Grass et al., Angew. Chem. Int. Ed. 54: 2552-2555 (2015); rate via Heckel et al., Sci. Rep. 9: 9663 (2019)",
            Self::Custom { .. } => "user-supplied kinetics",
        }
    }

    pub fn kinetics(&self) -> Kinetics {
        match *self {
            Self::Encapsulated => Kinetics {
                activation_energy_kj_mol: 197.0,
                rate_per_nt_year: 3.82e-15 * SECONDS_PER_YEAR,
                reference_celsius: 25.0,
            },
            Self::Bone => Kinetics {
                activation_energy_kj_mol: 127.0,
                rate_per_nt_year: 5.50e-6,
                reference_celsius: 13.1,
            },
            // ln 2 / (one week / 4 half-lives) / 158 nt, at 70 °C.
            Self::Silica => Kinetics {
                activation_energy_kj_mol: 155.0,
                rate_per_nt_year: 2.90e-8 * SECONDS_PER_YEAR,
                reference_celsius: 70.0,
            },
            Self::Custom { activation_energy_kj_mol, rate_per_nt_year, reference_celsius } => {
                Kinetics { activation_energy_kj_mol, rate_per_nt_year, reference_celsius }
            }
        }
    }

    /// Breaks per nucleotide per year at `celsius`.
    pub fn break_rate_per_nt_year(&self, celsius: f64) -> f64 {
        let k = self.kinetics();
        let t = celsius + KELVIN_OFFSET;
        let t_ref = k.reference_celsius + KELVIN_OFFSET;
        let ea = k.activation_energy_kj_mol * 1000.0;
        k.rate_per_nt_year * (-(ea / GAS_CONSTANT_J_PER_MOL_K) * (1.0 / t - 1.0 / t_ref)).exp()
    }
}

/// Probability that one molecule of `strand_len` nucleotides is still
/// intact after `years` at break rate `rate_per_nt_year`.
pub fn strand_survival(rate_per_nt_year: f64, strand_len: usize, years: f64) -> f64 {
    (-rate_per_nt_year * strand_len as f64 * years).exp()
}

/// Years until half of the molecules of this length have broken.
pub fn strand_half_life_years(rate_per_nt_year: f64, strand_len: usize) -> f64 {
    std::f64::consts::LN_2 / (rate_per_nt_year * strand_len as f64)
}

/// Everything needed to ask "how long does this archive stay readable?".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShelfLifeQuery {
    pub medium: StorageMedium,
    pub celsius: f64,
    /// Nucleotides per strand, primers included.
    pub strand_len: usize,
    /// Data strands in the file's Reed-Solomon stripe.
    pub data_strands: usize,
    /// Parity strands; up to this many lost sequences can be rebuilt.
    pub parity_strands: usize,
    /// Physical molecules of each sequence in the stored sample. A sequence
    /// is lost only when every copy has broken.
    pub copies_per_strand: u32,
    /// Required probability that the whole file is recoverable, e.g. 0.999.
    pub target_recovery: f64,
}

impl ShelfLifeQuery {
    /// Probability that one sequence has no intact copy left after `years`.
    pub fn sequence_loss_probability(&self, years: f64) -> f64 {
        let rate = self.medium.break_rate_per_nt_year(self.celsius);
        let broken = 1.0 - strand_survival(rate, self.strand_len, years);
        broken.powi(self.copies_per_strand.max(1) as i32)
    }

    /// Probability the file is still recoverable after `years`: no more
    /// than `parity_strands` of its `data + parity` sequences lost.
    pub fn recovery_probability(&self, years: f64) -> f64 {
        let n = self.data_strands + self.parity_strands;
        binomial_cdf(n, self.parity_strands, self.sequence_loss_probability(years))
    }

    /// Years until [`Self::recovery_probability`] falls below
    /// `target_recovery`. Returns `f64::INFINITY` if it never does within
    /// 10¹⁵ years.
    pub fn shelf_life_years(&self) -> f64 {
        if self.recovery_probability(0.0) < self.target_recovery {
            return 0.0;
        }
        let mut hi = 1.0;
        while self.recovery_probability(hi) >= self.target_recovery {
            hi *= 2.0;
            if hi > 1e15 {
                return f64::INFINITY;
            }
        }
        let mut lo = hi / 2.0;
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if self.recovery_probability(mid) >= self.target_recovery {
                lo = mid;
            } else {
                hi = mid;
            }
            if (hi - lo) <= 1e-6 * hi {
                break;
            }
        }
        lo
    }
}

/// P(X ≤ k) for X ~ Binomial(n, p), summed in log space so large stripes
/// and tiny probabilities stay accurate.
pub(crate) fn binomial_cdf(n: usize, k: usize, p: f64) -> f64 {
    if p <= 0.0 {
        return 1.0;
    }
    if p >= 1.0 {
        return if k >= n { 1.0 } else { 0.0 };
    }
    if k >= n {
        return 1.0;
    }
    let (ln_p, ln_q) = (p.ln(), (1.0 - p).ln());
    let mut ln_choose = 0.0;
    let mut total = 0.0;
    for i in 0..=k {
        if i > 0 {
            ln_choose += ((n - i + 1) as f64).ln() - (i as f64).ln();
        }
        total += (ln_choose + i as f64 * ln_p + (n - i) as f64 * ln_q).exp();
    }
    total.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    #[test]
    fn bone_rates_reproduce_allentofts_published_table() {
        // Table 1 of Allentoft et al. 2012, rounded to two figures there.
        let bone = StorageMedium::Bone;
        assert!(close(bone.break_rate_per_nt_year(13.1), 5.50e-6, 1e-9));
        for (celsius, published) in [(25.0, 4.5e-5), (15.0, 7.6e-6), (5.0, 1.1e-6), (-5.0, 1.5e-7)] {
            let k = bone.break_rate_per_nt_year(celsius);
            assert!(close(k, published, 0.06), "{celsius} °C: {k:e} vs {published:e}");
        }
    }

    #[test]
    fn encapsulated_half_life_matches_coudys_150_nt_figure() {
        // "38 000 years of half-life for a 150-nucleotide long DNA fragment" at 25 °C.
        let rate = StorageMedium::Encapsulated.break_rate_per_nt_year(25.0);
        let half_life = strand_half_life_years(rate, 150);
        assert!(close(half_life, 38_000.0, 0.02), "{half_life}");
    }

    #[test]
    fn silica_reproduces_grass_and_the_independent_readings_of_it() {
        let silica = StorageMedium::Silica;
        // Grass's week at 70 °C was four half-lives of a 158 nt strand.
        let week_years = 7.0 / 365.25;
        let half_life = strand_half_life_years(silica.break_rate_per_nt_year(70.0), 158);
        assert!(close(half_life, week_years / 4.0, 0.01), "{half_life}");
        // ...and, at Ea = 155 kJ/mol, as much decay as about 2,000 years at 9.4 °C.
        let equivalent = week_years * silica.break_rate_per_nt_year(70.0) / silica.break_rate_per_nt_year(9.4);
        assert!((1_900.0..2_500.0).contains(&equivalent), "{equivalent}");
        // Coudy et al. read 8 years at 25 °C for a 150 nt fragment, Doricchi et
        // al. 540 years at 10 °C: both within the factor of 3 the rate carries.
        let at = |celsius| strand_half_life_years(silica.break_rate_per_nt_year(celsius), 150);
        assert!((8.0 / 3.0..8.0 * 3.0).contains(&at(25.0)), "{}", at(25.0));
        assert!((540.0 / 3.0..540.0 * 3.0).contains(&at(10.0)), "{}", at(10.0));
        // Sealed anoxic capsules outlast silica by three orders of magnitude
        // at room temperature (Coudy's Fig. 6: 38,000 years against 8).
        let k25 = |m: StorageMedium| m.break_rate_per_nt_year(25.0);
        assert!(k25(silica) > 1_000.0 * k25(StorageMedium::Encapsulated));
    }

    #[test]
    fn colder_storage_is_slower_and_higher_ea_is_more_temperature_sensitive() {
        for m in [StorageMedium::Encapsulated, StorageMedium::Bone, StorageMedium::Silica] {
            assert!(m.break_rate_per_nt_year(-20.0) < m.break_rate_per_nt_year(4.0));
            assert!(m.break_rate_per_nt_year(4.0) < m.break_rate_per_nt_year(25.0));
        }
        let ratio = |m: StorageMedium| m.break_rate_per_nt_year(25.0) / m.break_rate_per_nt_year(-20.0);
        assert!(ratio(StorageMedium::Encapsulated) > ratio(StorageMedium::Bone));
    }

    #[test]
    fn custom_kinetics_at_the_reference_temperature_return_the_reference_rate() {
        let m = StorageMedium::Custom { activation_energy_kj_mol: 150.0, rate_per_nt_year: 2e-7, reference_celsius: 20.0 };
        assert!(close(m.break_rate_per_nt_year(20.0), 2e-7, 1e-12));
    }

    #[test]
    fn survival_and_half_life_agree() {
        let rate = 1e-6;
        let h = strand_half_life_years(rate, 200);
        assert!(close(strand_survival(rate, 200, h), 0.5, 1e-12));
    }

    #[test]
    fn binomial_cdf_matches_small_exact_cases() {
        assert!(close(binomial_cdf(3, 0, 0.5), 0.125, 1e-12));
        assert!(close(binomial_cdf(3, 1, 0.5), 0.5, 1e-12));
        assert!(close(binomial_cdf(4, 2, 0.1), 0.9963, 1e-9));
        assert_eq!(binomial_cdf(10, 10, 0.3), 1.0);
        assert_eq!(binomial_cdf(10, 3, 0.0), 1.0);
    }

    fn query(medium: StorageMedium, celsius: f64) -> ShelfLifeQuery {
        ShelfLifeQuery {
            medium,
            celsius,
            strand_len: 150,
            data_strands: 100,
            parity_strands: 20,
            copies_per_strand: 10,
            target_recovery: 0.999,
        }
    }

    #[test]
    fn shelf_life_is_the_point_recovery_crosses_the_target() {
        let q = query(StorageMedium::Bone, 13.1);
        let life = q.shelf_life_years();
        assert!(life.is_finite() && life > 0.0);
        assert!(q.recovery_probability(life * 0.999) >= 0.999);
        assert!(q.recovery_probability(life * 1.001) < 0.999);
    }

    #[test]
    fn more_parity_more_copies_and_colder_storage_all_last_longer() {
        let base = query(StorageMedium::Encapsulated, 25.0).shelf_life_years();
        let more_parity = ShelfLifeQuery { parity_strands: 40, ..query(StorageMedium::Encapsulated, 25.0) };
        let more_copies = ShelfLifeQuery { copies_per_strand: 100, ..query(StorageMedium::Encapsulated, 25.0) };
        let colder = query(StorageMedium::Encapsulated, 4.0);
        assert!(more_parity.shelf_life_years() > base);
        assert!(more_copies.shelf_life_years() > base);
        assert!(colder.shelf_life_years() > base);
    }

    #[test]
    fn no_parity_and_one_copy_fails_quickly() {
        let fragile = ShelfLifeQuery { parity_strands: 0, copies_per_strand: 1, ..query(StorageMedium::Bone, 25.0) };
        let robust = query(StorageMedium::Bone, 25.0);
        assert!(fragile.shelf_life_years() < robust.shelf_life_years() / 10.0);
    }

    #[test]
    fn parses_cli_names() {
        assert_eq!(StorageMedium::parse("DNAshell"), Some(StorageMedium::Encapsulated));
        assert_eq!(StorageMedium::parse("bone"), Some(StorageMedium::Bone));
        assert_eq!(StorageMedium::parse("SiO2"), Some(StorageMedium::Silica));
        assert_eq!(StorageMedium::parse("tape"), None);
    }
}
