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

use serde::{Deserialize, Serialize};

const AVOGADRO: f64 = 6.022_140_76e23;
const GRAMS_PER_MOL_PER_NT: f64 = 330.0;

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
}

impl Default for CostAssumptions {
    /// Array oligo-pool synthesis at $0.00015/nt (the figure the CLI's
    /// benchmark already uses for Twist-scale chip synthesis) and short-read
    /// sequencing at $10 per gigabase, read at 10x and kept at 10 copies.
    fn default() -> Self {
        Self { synthesis_usd_per_nt: 0.00015, sequencing_usd_per_gbase: 10.0, read_coverage: 10.0, copies_per_strand: 10.0 }
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
    pub assumptions: CostAssumptions,
}

/// Cost, density and mass of storing `shape` under `assumptions`.
pub fn estimate(shape: ArchiveShape, assumptions: CostAssumptions) -> CostReport {
    let mb = shape.payload_bytes as f64 / 1_000_000.0;
    let nt = shape.nucleotides as f64;
    let synthesis_usd = nt * assumptions.synthesis_usd_per_nt;
    let sequencing_usd = nt * assumptions.read_coverage / 1e9 * assumptions.sequencing_usd_per_gbase;
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
    fn empty_payload_does_not_divide_by_zero() {
        let r = estimate(ArchiveShape { payload_bytes: 0, nucleotides: 0, strands: 0 }, CostAssumptions::default());
        assert_eq!(r.write_usd_per_mb, 0.0);
        assert_eq!(r.bits_per_nt, 0.0);
    }
}
