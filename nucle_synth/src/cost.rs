//! # Cost, Density and Mass
//!
//! What it takes to write and read an archive, from its real strand counts.
//! Prices are assumptions the caller can override, because published
//! figures disagree by more than an order of magnitude: the US GAO put
//! synthetic DNA storage at about $3,500 per megabyte in 2022, while a 2026
//! trade analysis (Blocks & Files) cites about $100 per megabyte. The
//! defaults below are per-nucleotide list-price-scale assumptions, stated as
//! such in every report, not quotes.
//!
//! Mass uses 330 g/mol per nucleotide of single-stranded DNA and counts
//! every physical copy kept, which is what bounds how small an archive can
//! physically be.
//!
//! Carbon has three parts, each built from published figures and each with
//! its own scope:
//!
//! - Reading: Illumina's streamlined life-cycle assessments of its reagent
//!   kits give kg CO2e per gigabase sequenced (2025 Corporate Responsibility
//!   Report, p. 36; run by the vendor, not third-party reviewed). See
//!   [`Sequencer`]. A small archive on an unshared run still uses a whole
//!   kit, so these per-gigabase figures are a floor.
//! - The whole life cycle: Nguyen et al., "Architecting Datacenters for
//!   Sustainability: Greener Data Storage using Synthetic DNA", Electronics
//!   Goes Green 2020, modelled one terabyte kept for a year with 10% of it
//!   read, cradle to grave, and give the result only as a chart (Figure 5,
//!   in units of 0.1 kg CO2e per TB). The values here are readings of that
//!   chart. See [`Chemistry`].
//!
//! - Writing: no source reports CO2e per synthesized nucleotide, so it is
//!   built bottom-up from one lab-scale run of light-directed array
//!   synthesis. Antkowiak et al., "Low cost DNA data storage using
//!   photolithographic synthesis and advanced information reconstruction
//!   and error correction", Nat. Commun. 11, 5345 (2020), Supplementary
//!   Table 2, lists the reagents of one run. Each is weighed at a
//!   published cradle-to-gate emission factor, or at its chemical class's
//!   where it has none, and the light source at its measured power on
//!   Austria's grid (see [`array_run_kg_co2e`]): about 0.82 kg CO2e per
//!   run, two thirds of it acetonitrile and ethanol. Spread over that run's
//!   16,383 sequences of 60 nt, or over the 196,596 the same chip held for
//!   their largest file, that is 0.84 down to 0.070 mg CO2e per designed
//!   nucleotide. The synthesizer's fluidics, micromirror array and
//!   computer have no published power draw and no phosphoramidite has an
//!   emission factor of its own, so this is still a floor. See
//!   [`synthesis_kg_co2e_per_nt_range`].

use serde::{Deserialize, Serialize};

const AVOGADRO: f64 = 6.022_140_76e23;
const GRAMS_PER_MOL_PER_NT: f64 = 330.0;

/// Sequencers with a published kg CO2e per gigabase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sequencer {
    MiSeq,
    MiSeqI100,
    NovaSeq6000,
    NovaSeqX,
}

impl Sequencer {
    pub const ALL: [Sequencer; 4] = [Self::MiSeq, Self::MiSeqI100, Self::NovaSeq6000, Self::NovaSeqX];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name.to_ascii_lowercase())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::MiSeq => "miseq",
            Self::MiSeqI100 => "miseq-i100",
            Self::NovaSeq6000 => "novaseq-6000",
            Self::NovaSeqX => "novaseq-x",
        }
    }

    /// Illumina, 2025 Corporate Responsibility Report, p. 36.
    pub fn kg_co2e_per_gbase(&self) -> f64 {
        match self {
            Self::MiSeq => 3.81,
            Self::MiSeqI100 => 2.46,
            Self::NovaSeq6000 => 0.09,
            Self::NovaSeqX => 0.04,
        }
    }
}

/// DNA synthesis chemistries in Nguyen et al. 2020's life-cycle assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chemistry {
    /// Phosphoramidite synthesis with fossil-based acetonitrile.
    Phosphoramidite,
    /// Phosphoramidite synthesis with bio-based acetonitrile.
    BioAcetonitrile,
    /// Enzymatic (TdT) synthesis in water.
    Enzymatic,
}

impl Chemistry {
    pub const ALL: [Chemistry; 3] = [Self::Phosphoramidite, Self::BioAcetonitrile, Self::Enzymatic];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name.to_ascii_lowercase())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Phosphoramidite => "phosphoramidite",
            Self::BioAcetonitrile => "bio-acetonitrile",
            Self::Enzymatic => "enzymatic",
        }
    }

    /// kg CO2e for one terabyte kept a year with 10% read, read from
    /// Nguyen et al. 2020, Figure 5a (phosphoramidite's error bar spans
    /// about 3.1 to 11.2).
    pub fn kg_co2e_per_tb_year(&self) -> f64 {
        match self {
            Self::Phosphoramidite => 7.1,
            Self::BioAcetonitrile => 6.2,
            Self::Enzymatic => 1.6,
        }
    }
}

/// The reagents of one light-directed array synthesis run, in grams
/// (Antkowiak et al. 2020, Supplementary Table 2; the toluene from their
/// Methods), each with a cradle-to-gate emission factor in kg CO2e per kg:
///
/// - acetonitrile (the activator and cleavable-dT solutions counted as
///   acetonitrile) and ethanol: Amelio et al., Green Chem. 16, 3045
///   (2014), Table 4, means (ranges 2.27-3.70 and 1.03-1.39);
/// - DMSO, toluene, tetrahydrofuran and dichloromethane: the ACS GCI
///   Pharmaceutical Roundtable's PMI-LCA tool v07.2P (ecoinvent v2.2,
///   IPCC 2007), and its averaged organic solvent for imidazole, which has
///   no factor of its own;
/// - ethylenediamine, pyridine and the silanizing reagent: Idemat
///   2026RevB2's averages for their chemical classes (amines, pyridines,
///   organosilicon compounds), none having a factor of its own;
/// - iodine: Roche, Muhl and Finkbeiner, Int. J. Life Cycle Assess. 28,
///   1132 (2023), Chilean caliche with economic allocation;
/// - the four Bz-NPPOC phosphoramidites: the ACS tool's default for
///   complex reagents with protecting groups, since no phosphoramidite has
///   a published factor (Becker et al., ChemSusChem 2023, say the same).
///
/// The 9 mL of oxidizer is split by the composition the same group gives
/// in Sack et al. 2016: THF/water/pyridine/iodine 90.54/9.05/0.41/0.43
/// (v/v/v/w). The 80 g of exposure solvent is 1% imidazole in DMSO, and
/// the 0.03 g of beta-carotene is a 1% solution in dichloromethane.
const ARRAY_RUN_REAGENTS: [(&str, f64, f64); 12] = [
    ("acetonitrile, with activator and cleavable dT", (175.0 + 21.0 + 0.25) * 0.786, 2.96),
    ("ethanol", 90.0 * 0.789, 1.19),
    ("DMSO", 80.0 * 0.99, 1.302),
    ("imidazole", 80.0 * 0.01, 2.624),
    ("ethylenediamine", 20.0 * 0.899, 2.844),
    ("toluene", 20.0 * 0.867, 1.499),
    ("tetrahydrofuran", 9.0 * 0.9054 * 0.889, 5.720),
    ("pyridine", 9.0 * 0.0041 * 0.982, 9.743),
    ("iodine", 9.0 * 0.0043, 14.8),
    ("dichloromethane", 0.03 * 99.0, 3.393),
    ("silanizing reagent", 0.66, 4.561),
    ("Bz-NPPOC phosphoramidites", 0.08 + 0.05 + 0.07 + 0.06, 97.3),
];

/// Electricity of the run's light source, kWh. Hölz, Lietard and Somoza
/// (ACS Sustainable Chem. Eng. 5, 828, 2017) measured the LED system at 40
/// W lit and 20 W dark, with about a minute of chemistry per cycle after
/// the exposure; Antkowiak's 187 cycles expose for 35 s each.
const ARRAY_RUN_LIGHT_KWH: f64 = 187.0 * (35.0 * 40.0 + 60.0 * 20.0) / 3.6e6;

/// kg CO2e per kWh of Austria's consumption mix (Idemat 2026RevB2, 0.0381
/// per MJ); the run was made at the University of Vienna.
const AUSTRIA_KG_CO2E_PER_KWH: f64 = 0.038_080_585 * 3.6;

/// Designed nucleotides on that run (16,383 sequences of 60 nt) and on the
/// same chip at the 196,596 sequences of the authors' largest file, whose
/// reported cost per megabyte implies the same reagents per run.
const ARRAY_RUN_NT_FIRST: f64 = 16_383.0 * 60.0;
const ARRAY_RUN_NT_FULL: f64 = 196_596.0 * 60.0;

/// kg CO2e of one light-directed array synthesis run: every reagent in
/// Antkowiak et al.'s Supplementary Table 2 at its emission factor, and the
/// light source's electricity on Austria's grid.
pub fn array_run_kg_co2e() -> f64 {
    let reagents: f64 = ARRAY_RUN_REAGENTS.iter().map(|(_, grams, factor)| grams / 1000.0 * factor).sum();
    reagents + ARRAY_RUN_LIGHT_KWH * AUSTRIA_KG_CO2E_PER_KWH
}

/// kg CO2e per designed nucleotide, (full chip, first run): the low and
/// high end of write-side carbon.
pub fn synthesis_kg_co2e_per_nt_range() -> (f64, f64) {
    let run = array_run_kg_co2e();
    (run / ARRAY_RUN_NT_FULL, run / ARRAY_RUN_NT_FIRST)
}

/// The same chart's figures for the media DNA was compared against, kg
/// CO2e per terabyte-year (Nguyen et al. 2020, Figure 5b).
pub const HDD_KG_CO2E_PER_TB_YEAR: f64 = 42.0;
pub const TAPE_KG_CO2E_PER_TB_YEAR: f64 = 5.0;

/// Prices and physical assumptions for [`estimate`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostAssumptions {
    /// USD per synthesized nucleotide.
    pub synthesis_usd_per_nt: f64,
    /// USD per billion sequenced bases.
    pub sequencing_usd_per_gbase: f64,
    /// Sequencing reads per stored strand when the archive is read back.
    pub read_coverage: f64,
    /// Physical molecules kept per sequence.
    pub copies_per_strand: f64,
    /// Instrument the archive is read back on, for read-side carbon.
    pub sequencer: Sequencer,
    /// Synthesis chemistry, for life-cycle carbon.
    pub chemistry: Chemistry,
    /// kg CO2e per synthesized nucleotide, replacing the bottom-up range.
    #[serde(default)]
    pub synthesis_kg_co2e_per_nt: Option<f64>,
}

impl Default for CostAssumptions {
    /// Array oligo-pool synthesis at $0.00015/nt (the figure the CLI's
    /// benchmark already uses for Twist-scale chip synthesis) and short-read
    /// sequencing at $10 per gigabase, read at 10x on a NovaSeq X and kept
    /// at 10 copies, written by phosphoramidite synthesis.
    fn default() -> Self {
        Self {
            synthesis_usd_per_nt: 0.00015,
            sequencing_usd_per_gbase: 10.0,
            read_coverage: 10.0,
            copies_per_strand: 10.0,
            sequencer: Sequencer::NovaSeqX,
            chemistry: Chemistry::Phosphoramidite,
            synthesis_kg_co2e_per_nt: None,
        }
    }
}

/// The archive being costed, from a real encoding.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ArchiveShape {
    /// Payload size in bytes.
    pub payload_bytes: usize,
    /// Nucleotides synthesized for one copy of every strand, parity and
    /// primers included.
    pub nucleotides: usize,
    /// Strands (data + parity).
    pub strands: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostReport {
    pub synthesis_usd: f64,
    pub sequencing_usd: f64,
    pub write_usd_per_mb: f64,
    pub read_usd_per_mb: f64,
    /// Payload bits per synthesized nucleotide, all overhead included.
    pub bits_per_nt: f64,
    /// Mass of every copy of every strand, grams.
    pub mass_grams: f64,
    /// Payload bytes per gram of DNA at the stated copy number.
    pub bytes_per_gram: f64,
    /// kg CO2e of synthesis solvents for one copy of every strand, low and
    /// high end ([`synthesis_kg_co2e_per_nt_range`]); equal when the caller
    /// supplied a per-nucleotide figure.
    pub synthesis_kg_co2e_low: f64,
    pub synthesis_kg_co2e_high: f64,
    /// kg CO2e of the sequencing kits for one read-back.
    pub sequencing_kg_co2e: f64,
    /// kg CO2e per year kept, whole life cycle, scaled from Nguyen et al.'s
    /// terabyte-year by payload size.
    pub lifecycle_kg_co2e_per_year: f64,
    pub assumptions: CostAssumptions,
}

/// Cost, density and mass of storing `shape` under `assumptions`.
pub fn estimate(shape: ArchiveShape, assumptions: CostAssumptions) -> CostReport {
    let mb = shape.payload_bytes as f64 / 1_000_000.0;
    let nt = shape.nucleotides as f64;
    let synthesis_usd = nt * assumptions.synthesis_usd_per_nt;
    let gbases_read = nt * assumptions.read_coverage / 1e9;
    let (synthesis_low, synthesis_high) = match assumptions.synthesis_kg_co2e_per_nt {
        Some(per_nt) => (per_nt, per_nt),
        None => synthesis_kg_co2e_per_nt_range(),
    };
    let sequencing_usd = gbases_read * assumptions.sequencing_usd_per_gbase;
    let mass_grams = nt * assumptions.copies_per_strand * GRAMS_PER_MOL_PER_NT / AVOGADRO;
    let per_mb = |usd: f64| if mb > 0.0 { usd / mb } else { 0.0 };
    CostReport {
        synthesis_usd,
        sequencing_usd,
        write_usd_per_mb: per_mb(synthesis_usd),
        read_usd_per_mb: per_mb(sequencing_usd),
        bits_per_nt: if nt > 0.0 { shape.payload_bytes as f64 * 8.0 / nt } else { 0.0 },
        mass_grams,
        bytes_per_gram: if mass_grams > 0.0 { shape.payload_bytes as f64 / mass_grams } else { 0.0 },
        synthesis_kg_co2e_low: nt * synthesis_low,
        synthesis_kg_co2e_high: nt * synthesis_high,
        sequencing_kg_co2e: gbases_read * assumptions.sequencer.kg_co2e_per_gbase(),
        lifecycle_kg_co2e_per_year: shape.payload_bytes as f64 / 1e12 * assumptions.chemistry.kg_co2e_per_tb_year(),
        assumptions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> ArchiveShape {
        // 1 MB payload in 5.5 M nucleotides: about 1.45 bits/nt.
        ArchiveShape { payload_bytes: 1_000_000, nucleotides: 5_500_000, strands: 36_667 }
    }

    #[test]
    fn costs_scale_with_nucleotides_and_coverage() {
        let r = estimate(shape(), CostAssumptions::default());
        assert!((r.synthesis_usd - 825.0).abs() < 1e-6);
        assert!((r.write_usd_per_mb - 825.0).abs() < 1e-6);
        // 5.5 Mnt x 10 reads = 0.055 Gbase x $10.
        assert!((r.sequencing_usd - 0.55).abs() < 1e-9);
        let deeper = estimate(shape(), CostAssumptions { read_coverage: 20.0, ..CostAssumptions::default() });
        assert!((deeper.sequencing_usd - 1.1).abs() < 1e-9);
    }

    #[test]
    fn density_and_mass_follow_from_the_encoding() {
        let r = estimate(shape(), CostAssumptions::default());
        assert!((r.bits_per_nt - 8.0 / 5.5).abs() < 1e-12);
        // 5.5e7 nt x 330 g/mol / N_A.
        let expected = 5.5e6 * 10.0 * 330.0 / 6.022_140_76e23;
        assert!((r.mass_grams - expected).abs() < expected * 1e-12);
        assert!(r.bytes_per_gram > 1e17, "a megabyte weighs femtograms: {}", r.bytes_per_gram);
    }

    #[test]
    fn carbon_follows_bases_read_and_bytes_kept() {
        let r = estimate(shape(), CostAssumptions::default());
        // 0.055 Gbase on a NovaSeq X at 0.04 kg CO2e per Gbase.
        assert!((r.sequencing_kg_co2e - 0.0022).abs() < 1e-12);
        let miseq = estimate(shape(), CostAssumptions { sequencer: Sequencer::MiSeq, ..CostAssumptions::default() });
        assert!((miseq.sequencing_kg_co2e / r.sequencing_kg_co2e - 3.81 / 0.04).abs() < 1e-9);
        // 1 MB is a millionth of Nguyen et al.'s terabyte-year.
        assert!((r.lifecycle_kg_co2e_per_year - 7.1e-6).abs() < 1e-15);
        let enzymatic = estimate(shape(), CostAssumptions { chemistry: Chemistry::Enzymatic, ..CostAssumptions::default() });
        assert!(enzymatic.lifecycle_kg_co2e_per_year < r.lifecycle_kg_co2e_per_year);
        // Per terabyte-year, phosphoramidite DNA sits between tape and hard disk.
        let per_tb = r.lifecycle_kg_co2e_per_year * 1e6;
        assert!(TAPE_KG_CO2E_PER_TB_YEAR < per_tb && per_tb < HDD_KG_CO2E_PER_TB_YEAR);
        assert_eq!(Sequencer::parse("NovaSeq-X"), Some(Sequencer::NovaSeqX));
        assert_eq!(Chemistry::parse("enzymatic"), Some(Chemistry::Enzymatic));
        assert_eq!(Sequencer::parse("hiseq"), None);
    }

    #[test]
    fn write_side_carbon_is_the_array_run_spread_over_its_nucleotides() {
        // 154.25 g of acetonitrile at 2.96 and 71.01 g of ethanol at 1.19
        // were the whole estimate before; they are still two thirds of it.
        let run = array_run_kg_co2e();
        let solvents = 0.154_25 * 2.96 + 0.071_01 * 1.19;
        assert!((run - 0.8225).abs() < 0.001, "{run}");
        assert!((0.6..0.7).contains(&(solvents / run)), "{}", solvents / run);
        // The light source: 0.135 kWh at 0.137 kg CO2e per kWh.
        assert!((ARRAY_RUN_LIGHT_KWH - 0.135).abs() < 0.001);
        assert!((AUSTRIA_KG_CO2E_PER_KWH - 0.1371).abs() < 0.0001);
        let (low, high) = synthesis_kg_co2e_per_nt_range();
        assert!((low - 6.97e-8).abs() < 0.01e-8, "{low:e}");
        assert!((high - 8.37e-7).abs() < 0.01e-7, "{high:e}");
        let r = estimate(shape(), CostAssumptions::default());
        assert!((r.synthesis_kg_co2e_low - 5.5e6 * low).abs() < 1e-12);
        assert!((r.synthesis_kg_co2e_high - 5.5e6 * high).abs() < 1e-12);
        let given = estimate(shape(), CostAssumptions { synthesis_kg_co2e_per_nt: Some(1e-7), ..CostAssumptions::default() });
        assert!((given.synthesis_kg_co2e_low - 0.55).abs() < 1e-12);
        assert_eq!(given.synthesis_kg_co2e_low, given.synthesis_kg_co2e_high);
    }

    #[test]
    fn empty_payload_does_not_divide_by_zero() {
        let r = estimate(ArchiveShape { payload_bytes: 0, nucleotides: 0, strands: 0 }, CostAssumptions::default());
        assert_eq!(r.write_usd_per_mb, 0.0);
        assert_eq!(r.bits_per_nt, 0.0);
    }
}
