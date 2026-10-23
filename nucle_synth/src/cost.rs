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
//! Carbon comes from the only published figures, each with its own scope:
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
//! No source gives CO2e per synthesized nucleotide for array oligo pools,
//! so synthesis on its own has no carbon figure.

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
    fn empty_payload_does_not_divide_by_zero() {
        let r = estimate(ArchiveShape { payload_bytes: 0, nucleotides: 0, strands: 0 }, CostAssumptions::default());
        assert_eq!(r.write_usd_per_mb, 0.0);
        assert_eq!(r.bits_per_nt, 0.0);
    }
}
