//! # Core Nucleotide Types and DNA Strand Primitives
//!
//! Foundation types used by every layer of the Nucle-OS stack.
//! Defines `Nucleotide` (A, T, G, C), `DnaStrand` (sequence of nucleotides),
//! and all conversion traits between binary data and DNA representations.

use std::fmt;
use serde::{Serialize, Deserialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during nucleotide/strand operations.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum DnaError {
    #[error("invalid nucleotide character: '{0}'")]
    InvalidNucleotide(char),

    #[error("empty strand")]
    EmptyStrand,

    #[error("invalid strand string: '{0}'")]
    InvalidStrandString(String),

    #[error("strand length {actual} does not match expected {expected}")]
    LengthMismatch { expected: usize, actual: usize },

    #[error("encoding error: {0}")]
    EncodingError(String),

    #[error("decoding error: {0}")]
    DecodingError(String),
}

// ---------------------------------------------------------------------------
// Nucleotide
// ---------------------------------------------------------------------------

/// A single DNA nucleotide base.
///
/// The four bases of DNA:
/// - **A** (Adenine) — pairs with T
/// - **T** (Thymine) — pairs with A
/// - **G** (Guanine) — pairs with C
/// - **C** (Cytosine) — pairs with G
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nucleotide {
    A, // Adenine
    T, // Thymine
    G, // Guanine
    C, // Cytosine
}

impl Nucleotide {
    /// All four nucleotides in canonical order.
    pub const ALL: [Nucleotide; 4] = [
        Nucleotide::A,
        Nucleotide::T,
        Nucleotide::G,
        Nucleotide::C,
    ];

    /// Convert a character to a nucleotide.
    pub fn from_char(c: char) -> Result<Self, DnaError> {
        match c {
            'A' | 'a' => Ok(Nucleotide::A),
            'T' | 't' => Ok(Nucleotide::T),
            'G' | 'g' => Ok(Nucleotide::G),
            'C' | 'c' => Ok(Nucleotide::C),
            _ => Err(DnaError::InvalidNucleotide(c)),
        }
    }

    /// Convert nucleotide to its character representation.
    pub fn to_char(self) -> char {
        match self {
            Nucleotide::A => 'A',
            Nucleotide::T => 'T',
            Nucleotide::G => 'G',
            Nucleotide::C => 'C',
        }
    }

    /// Return the Watson-Crick complement of this base.
    ///
    /// A ↔ T, G ↔ C
    pub fn complement(self) -> Self {
        match self {
            Nucleotide::A => Nucleotide::T,
            Nucleotide::T => Nucleotide::A,
            Nucleotide::G => Nucleotide::C,
            Nucleotide::C => Nucleotide::G,
        }
    }

    /// Returns true if this is a GC base (Guanine or Cytosine).
    ///
    /// GC bases form 3 hydrogen bonds (stronger than AT's 2 bonds),
    /// affecting thermal stability and synthesis fidelity.
    pub fn is_gc(self) -> bool {
        matches!(self, Nucleotide::G | Nucleotide::C)
    }

    /// Map a 2-bit value to a nucleotide.
    ///
    /// Mapping: 0 → A, 1 → T, 2 → G, 3 → C
    pub fn from_bits(bits: u8) -> Result<Self, DnaError> {
        match bits {
            0 => Ok(Nucleotide::A),
            1 => Ok(Nucleotide::T),
            2 => Ok(Nucleotide::G),
            3 => Ok(Nucleotide::C),
            _ => Err(DnaError::EncodingError(format!(
                "invalid 2-bit value: {}",
                bits
            ))),
        }
    }

    /// Map a nucleotide to its 2-bit representation.
    ///
    /// Mapping: A → 0, T → 1, G → 2, C → 3
    pub fn to_bits(self) -> u8 {
        match self {
            Nucleotide::A => 0,
            Nucleotide::T => 1,
            Nucleotide::G => 2,
            Nucleotide::C => 3,
        }
    }

    /// Map a ternary digit (0, 1, 2) to a nucleotide, excluding
    /// the `previous` base to prevent homopolymer runs.
    ///
    /// This is the core of Goldman et al.'s rotating cipher:
    /// given the previous nucleotide, map trit → one of the
    /// three remaining bases in a deterministic order.
    pub fn from_trit(trit: u8, previous: Nucleotide) -> Result<Self, DnaError> {
        if trit > 2 {
            return Err(DnaError::EncodingError(format!(
                "invalid ternary digit: {}",
                trit
            )));
        }
        // The three bases that are NOT the previous base, in canonical order
        let candidates: Vec<Nucleotide> = Nucleotide::ALL
            .iter()
            .copied()
            .filter(|&n| n != previous)
            .collect();
        Ok(candidates[trit as usize])
    }

    /// Inverse of `from_trit`: given this nucleotide and the previous one,
    /// return the ternary digit (0, 1, or 2).
    pub fn to_trit(self, previous: Nucleotide) -> Result<u8, DnaError> {
        let candidates: Vec<Nucleotide> = Nucleotide::ALL
            .iter()
            .copied()
            .filter(|&n| n != previous)
            .collect();
        candidates
            .iter()
            .position(|&n| n == self)
            .map(|p| p as u8)
            .ok_or_else(|| {
                DnaError::DecodingError(format!(
                    "nucleotide {:?} same as previous {:?}",
                    self, previous
                ))
            })
    }
}

impl fmt::Display for Nucleotide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

// ---------------------------------------------------------------------------
// DnaStrand
// ---------------------------------------------------------------------------

/// A sequence of nucleotides representing a single DNA strand (oligo).
///
/// This is the fundamental data unit in the DNA storage stack.
/// Each strand typically contains:
/// - Optional primer regions (for addressing/retrieval)
/// - A data payload (encoded binary data)
/// - Optional index/seed information
///
/// Typical strand lengths are 150–300 nucleotides, limited by
/// synthesis technology fidelity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DnaStrand {
    /// The nucleotide sequence.
    bases: Vec<Nucleotide>,
}

impl DnaStrand {
    /// Create a new strand from a vector of nucleotides.
    pub fn new(bases: Vec<Nucleotide>) -> Self {
        Self { bases }
    }

    /// Create an empty strand.
    pub fn empty() -> Self {
        Self { bases: Vec::new() }
    }

    /// Parse a strand from a string of nucleotide characters.
    ///
    /// Accepts both uppercase and lowercase (A/a, T/t, G/g, C/c).
    ///
    /// # Example
    /// ```
    /// use nucle_codec::base::DnaStrand;
    /// let strand = DnaStrand::from_str("ATCGATCG").unwrap();
    /// assert_eq!(strand.len(), 8);
    /// ```
    pub fn from_str(s: &str) -> Result<Self, DnaError> {
        if s.is_empty() {
            return Err(DnaError::EmptyStrand);
        }
        let bases: Result<Vec<Nucleotide>, _> =
            s.chars().map(Nucleotide::from_char).collect();
        Ok(Self { bases: bases? })
    }

    /// Return the nucleotide sequence as a slice.
    pub fn bases(&self) -> &[Nucleotide] {
        &self.bases
    }

    /// Return the nucleotide sequence as a mutable slice.
    pub fn bases_mut(&mut self) -> &mut Vec<Nucleotide> {
        &mut self.bases
    }

    /// Number of nucleotides in this strand.
    pub fn len(&self) -> usize {
        self.bases.len()
    }

    /// Whether this strand has zero nucleotides.
    pub fn is_empty(&self) -> bool {
        self.bases.is_empty()
    }

    /// Convert the strand to its string representation.
    pub fn to_string(&self) -> String {
        self.bases.iter().map(|n| n.to_char()).collect()
    }

    /// Get the Watson-Crick reverse complement of this strand.
    ///
    /// Reverses the sequence and complements each base:
    /// 5'-ATCG-3' → 3'-TAGC-5' → 5'-CGAT-3'
    pub fn reverse_complement(&self) -> Self {
        let bases: Vec<Nucleotide> = self
            .bases
            .iter()
            .rev()
            .map(|n| n.complement())
            .collect();
        Self { bases }
    }

    /// Calculate the GC content as a fraction (0.0 to 1.0).
    ///
    /// GC content must be 40–60% for reliable synthesis and sequencing.
    pub fn gc_content(&self) -> f64 {
        if self.bases.is_empty() {
            return 0.0;
        }
        let gc_count = self.bases.iter().filter(|n| n.is_gc()).count();
        gc_count as f64 / self.bases.len() as f64
    }

    /// Find the longest homopolymer run (consecutive identical bases).
    ///
    /// Returns (nucleotide, run_length). Max allowed is typically 3.
    pub fn max_homopolymer_run(&self) -> (Option<Nucleotide>, usize) {
        if self.bases.is_empty() {
            return (None, 0);
        }

        let mut max_base = self.bases[0];
        let mut max_run = 1usize;
        let mut current_run = 1usize;

        for i in 1..self.bases.len() {
            if self.bases[i] == self.bases[i - 1] {
                current_run += 1;
                if current_run > max_run {
                    max_run = current_run;
                    max_base = self.bases[i];
                }
            } else {
                current_run = 1;
            }
        }

        (Some(max_base), max_run)
    }

    /// Append a nucleotide to the end of this strand.
    pub fn push(&mut self, base: Nucleotide) {
        self.bases.push(base);
    }

    /// Append all nucleotides from another strand.
    pub fn extend(&mut self, other: &DnaStrand) {
        self.bases.extend_from_slice(&other.bases);
    }

    /// Extract a sub-strand (slice) from start to end (exclusive).
    pub fn slice(&self, start: usize, end: usize) -> DnaStrand {
        DnaStrand::new(self.bases[start..end].to_vec())
    }

    /// Get nucleotide at position, if in bounds.
    pub fn get(&self, index: usize) -> Option<Nucleotide> {
        self.bases.get(index).copied()
    }
}

impl fmt::Display for DnaStrand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for base in &self.bases {
            write!(f, "{}", base)?;
        }
        Ok(())
    }
}

impl IntoIterator for DnaStrand {
    type Item = Nucleotide;
    type IntoIter = std::vec::IntoIter<Nucleotide>;

    fn into_iter(self) -> Self::IntoIter {
        self.bases.into_iter()
    }
}

impl<'a> IntoIterator for &'a DnaStrand {
    type Item = &'a Nucleotide;
    type IntoIter = std::slice::Iter<'a, Nucleotide>;

    fn into_iter(self) -> Self::IntoIter {
        self.bases.iter()
    }
}

impl FromIterator<Nucleotide> for DnaStrand {
    fn from_iter<I: IntoIterator<Item = Nucleotide>>(iter: I) -> Self {
        DnaStrand::new(iter.into_iter().collect())
    }
}

// ---------------------------------------------------------------------------
// StrandCollection — a set of strands representing encoded data
// ---------------------------------------------------------------------------

/// A collection of DNA strands, typically representing one encoded file
/// or data block.
///
/// In DNA storage, data is split across many short strands (oligos).
/// This structure holds all strands for a single logical unit of data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrandCollection {
    /// The strands in this collection.
    pub strands: Vec<DnaStrand>,
    /// Total bytes of original data encoded in these strands.
    pub original_size: usize,
}

impl StrandCollection {
    /// Create a new empty collection.
    pub fn new(original_size: usize) -> Self {
        Self {
            strands: Vec::new(),
            original_size,
        }
    }

    /// Create a collection from existing strands.
    pub fn from_strands(strands: Vec<DnaStrand>, original_size: usize) -> Self {
        Self {
            strands,
            original_size,
        }
    }

    /// Number of strands in this collection.
    pub fn strand_count(&self) -> usize {
        self.strands.len()
    }

    /// Total number of nucleotides across all strands.
    pub fn total_nucleotides(&self) -> usize {
        self.strands.iter().map(|s| s.len()).sum()
    }

    /// Information density: original bytes / total nucleotides.
    /// Higher is better. Theoretical max is 0.25 bytes/nt (2 bits/nt).
    pub fn density(&self) -> f64 {
        let total_nt = self.total_nucleotides();
        if total_nt == 0 {
            return 0.0;
        }
        self.original_size as f64 / total_nt as f64
    }

    /// Bits per nucleotide achieved by this encoding.
    /// Theoretical max is 2.0 bits/nt.
    pub fn bits_per_nucleotide(&self) -> f64 {
        self.density() * 8.0
    }

    /// Add a strand to the collection.
    pub fn push(&mut self, strand: DnaStrand) {
        self.strands.push(strand);
    }

    /// Average GC content across all strands.
    pub fn avg_gc_content(&self) -> f64 {
        if self.strands.is_empty() {
            return 0.0;
        }
        let total: f64 = self.strands.iter().map(|s| s.gc_content()).sum();
        total / self.strands.len() as f64
    }

    /// Maximum homopolymer run across all strands.
    pub fn max_homopolymer(&self) -> usize {
        self.strands
            .iter()
            .map(|s| s.max_homopolymer_run().1)
            .max()
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Codec Trait — interface all encoders/decoders must implement
// ---------------------------------------------------------------------------

/// Trait that all DNA codecs must implement.
///
/// A codec converts between raw binary data and DNA strands,
/// handling the encoding constraints and data segmentation.
pub trait DnaCodec {
    /// Name of this codec (e.g., "ternary", "fountain").
    fn name(&self) -> &str;

    /// Encode raw bytes into a collection of DNA strands.
    fn encode(&self, data: &[u8]) -> Result<StrandCollection, DnaError>;

    /// Decode a collection of DNA strands back to raw bytes.
    fn decode(&self, strands: &StrandCollection) -> Result<Vec<u8>, DnaError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nucleotide_from_char() {
        assert_eq!(Nucleotide::from_char('A').unwrap(), Nucleotide::A);
        assert_eq!(Nucleotide::from_char('t').unwrap(), Nucleotide::T);
        assert_eq!(Nucleotide::from_char('G').unwrap(), Nucleotide::G);
        assert_eq!(Nucleotide::from_char('c').unwrap(), Nucleotide::C);
        assert!(Nucleotide::from_char('X').is_err());
    }

    #[test]
    fn test_nucleotide_complement() {
        assert_eq!(Nucleotide::A.complement(), Nucleotide::T);
        assert_eq!(Nucleotide::T.complement(), Nucleotide::A);
        assert_eq!(Nucleotide::G.complement(), Nucleotide::C);
        assert_eq!(Nucleotide::C.complement(), Nucleotide::G);
    }

    #[test]
    fn test_nucleotide_bits_roundtrip() {
        for n in Nucleotide::ALL {
            let bits = n.to_bits();
            assert_eq!(Nucleotide::from_bits(bits).unwrap(), n);
        }
    }

    #[test]
    fn test_nucleotide_trit_roundtrip() {
        for prev in Nucleotide::ALL {
            for trit in 0..3u8 {
                let n = Nucleotide::from_trit(trit, prev).unwrap();
                assert_ne!(n, prev, "trit mapping must avoid previous base");
                assert_eq!(n.to_trit(prev).unwrap(), trit);
            }
        }
    }

    #[test]
    fn test_strand_from_str() {
        let strand = DnaStrand::from_str("ATCGATCG").unwrap();
        assert_eq!(strand.len(), 8);
        assert_eq!(strand.to_string(), "ATCGATCG");
    }

    #[test]
    fn test_strand_gc_content() {
        // GGCC = 100% GC
        let strand = DnaStrand::from_str("GGCC").unwrap();
        assert!((strand.gc_content() - 1.0).abs() < f64::EPSILON);

        // AATT = 0% GC
        let strand = DnaStrand::from_str("AATT").unwrap();
        assert!(strand.gc_content().abs() < f64::EPSILON);

        // ATGC = 50% GC
        let strand = DnaStrand::from_str("ATGC").unwrap();
        assert!((strand.gc_content() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_strand_homopolymer() {
        let strand = DnaStrand::from_str("ATAAAGC").unwrap();
        let (base, run) = strand.max_homopolymer_run();
        assert_eq!(base, Some(Nucleotide::A));
        assert_eq!(run, 3);

        let strand = DnaStrand::from_str("ATCG").unwrap();
        let (_, run) = strand.max_homopolymer_run();
        assert_eq!(run, 1);
    }

    #[test]
    fn test_strand_reverse_complement() {
        let strand = DnaStrand::from_str("ATCG").unwrap();
        let rc = strand.reverse_complement();
        assert_eq!(rc.to_string(), "CGAT");

        // Double reverse complement should give back original
        assert_eq!(rc.reverse_complement(), strand);
    }

    #[test]
    fn test_strand_collection_metrics() {
        let s1 = DnaStrand::from_str("ATCGATCG").unwrap(); // 8 nt
        let s2 = DnaStrand::from_str("GCGCATAT").unwrap(); // 8 nt
        let collection = StrandCollection::from_strands(vec![s1, s2], 2);

        assert_eq!(collection.strand_count(), 2);
        assert_eq!(collection.total_nucleotides(), 16);
        assert!((collection.density() - 0.125).abs() < 0.001); // 2 bytes / 16 nt
        assert!((collection.bits_per_nucleotide() - 1.0).abs() < 0.001); // 16 bits / 16 nt
    }

    #[test]
    fn test_strand_slice_and_extend() {
        let strand = DnaStrand::from_str("ATCGATCG").unwrap();
        let sub = strand.slice(2, 5);
        assert_eq!(sub.to_string(), "CGA");

        let mut s1 = DnaStrand::from_str("AT").unwrap();
        let s2 = DnaStrand::from_str("CG").unwrap();
        s1.extend(&s2);
        assert_eq!(s1.to_string(), "ATCG");
    }

    #[test]
    fn test_strand_iterator() {
        let strand = DnaStrand::from_str("ATCG").unwrap();
        let collected: Vec<Nucleotide> = strand.into_iter().collect();
        assert_eq!(collected, vec![
            Nucleotide::A,
            Nucleotide::T,
            Nucleotide::C,
            Nucleotide::G,
        ]);
    }
}
