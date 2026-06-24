//! # Yin-Yang Codec (Ping et al., 2022)
//!
//! Encodes binary data into DNA sequences using two **complementary mapping
//! rules** ("Yin" and "Yang") that achieve GC balance by construction.
//!
//! ## Algorithm
//!
//! Each nucleotide encodes **2 bits** (one from Segment A, one from Segment B):
//!
//! 1. Split the binary stream into two parallel segments (A and B)
//! 2. **Yang rule**: bit from Segment A selects a GC partition:
//!    `0 → {A, T}` (AT bases), `1 → {C, G}` (GC bases)
//! 3. **Yin rule**: bit from Segment B + previous nucleotide selects within
//!    the partition (context-dependent to reduce homopolymers)
//! 4. The **intersection** of Yang and Yin sets yields exactly one nucleotide
//!
//! ## Properties
//!
//! - **Density**: 2.0 bits/nt theoretical, ~1.7 bits/nt effective (with headers)
//! - **GC balance**: Structural guarantee from Yang rule — GC% mirrors bit
//!   distribution of Segment A (≈50% for most real data)
//! - **Homopolymers**: Reduced by Yin rule's context-dependency (not eliminated)
//!
//! ## Reference
//!
//! Ping, Z., et al. (2022). "Towards practical and robust DNA-based data
//! archiving using the yin-yang codec system." Nature Computational Science,
//! 2, 234–242.

use crate::base::{DnaCodec, DnaError, DnaStrand, Nucleotide, StrandCollection};
use crate::constraints::{ConstraintConfig, ConstraintValidator};
use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the Yin-Yang codec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YinYangConfig {
    /// Maximum payload nucleotides per strand (excluding index header).
    /// Typical: 100–200 nt.
    pub strand_length: usize,

    /// Whether to run biological constraint screening on output strands.
    /// Yin-Yang achieves GC balance by construction, but screening catches
    /// edge cases (palindromes, residual homopolymer runs).
    pub screen_constraints: bool,

    /// Constraint configuration for optional screening.
    pub constraint_config: ConstraintConfig,
}

impl Default for YinYangConfig {
    fn default() -> Self {
        Self {
            strand_length: 150,
            screen_constraints: false, // GC balance by construction
            constraint_config: ConstraintConfig::default(),
        }
    }
}

impl YinYangConfig {
    /// Strict configuration with constraint screening enabled.
    pub fn strict() -> Self {
        Self {
            screen_constraints: true,
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Yin-Yang Mapping Rules
// ---------------------------------------------------------------------------

/// Yang rule: maps one bit → a pair of nucleotides partitioned by GC content.
/// `0 → {A, T}` (weak/AT bases), `1 → {C, G}` (strong/GC bases).
///
/// This is the GC-balancing mechanism: for roughly balanced input bits,
/// the output is roughly 50% GC by construction.
fn yang_set(bit: u8) -> [Nucleotide; 2] {
    if bit == 0 {
        [Nucleotide::A, Nucleotide::T]
    } else {
        [Nucleotide::C, Nucleotide::G]
    }
}

/// Yin rule: maps one bit + previous nucleotide → a pair of nucleotides.
///
/// The context-dependency on the previous base helps break homopolymer runs.
/// Each Yin set **must** contain exactly 1 AT base and 1 GC base so that
/// the intersection with any Yang set ({A,T} or {C,G}) yields exactly 1
/// nucleotide.
///
/// Rule table (designed so that bit=0 maps away from prev, reducing homopolymers):
///
/// | Prev | bit=0         | bit=1         |
/// |------|---------------|---------------|
/// | A    | {T, G}        | {A, C}        |
/// | T    | {A, C}        | {T, G}        |
/// | C    | {T, G}        | {A, C}        |
/// | G    | {A, C}        | {T, G}        |
fn yin_set(bit: u8, prev: Nucleotide) -> [Nucleotide; 2] {
    match prev {
        Nucleotide::A => {
            if bit == 0 { [Nucleotide::T, Nucleotide::G] }
            else        { [Nucleotide::A, Nucleotide::C] }
        }
        Nucleotide::T => {
            if bit == 0 { [Nucleotide::A, Nucleotide::C] }
            else        { [Nucleotide::T, Nucleotide::G] }
        }
        Nucleotide::C => {
            if bit == 0 { [Nucleotide::T, Nucleotide::G] }
            else        { [Nucleotide::A, Nucleotide::C] }
        }
        Nucleotide::G => {
            if bit == 0 { [Nucleotide::A, Nucleotide::C] }
            else        { [Nucleotide::T, Nucleotide::G] }
        }
    }
}

/// Intersect Yang and Yin sets to get exactly one nucleotide.
///
/// By construction, the Yang set (2 elements) and Yin set (2 elements)
/// always share exactly 1 element from {A, T, C, G}.
fn intersect(yang: &[Nucleotide; 2], yin: &[Nucleotide; 2]) -> Result<Nucleotide, DnaError> {
    for &y in yang {
        for &x in yin {
            if y == x {
                return Ok(y);
            }
        }
    }
    Err(DnaError::EncodingError(
        "yin-yang intersection empty (rule inconsistency)".into(),
    ))
}

/// Reverse Yang rule: given a nucleotide, recover bit_a.
/// If the nucleotide is A or T → 0, if C or G → 1.
fn yang_decode(nt: Nucleotide) -> u8 {
    if nt.is_gc() { 1 } else { 0 }
}

/// Reverse Yin rule: given a nucleotide and previous base, recover bit_b.
fn yin_decode(nt: Nucleotide, prev: Nucleotide) -> u8 {
    let set0 = yin_set(0, prev);
    if set0.contains(&nt) { 0 } else { 1 }
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// Yin-Yang dual-rule codec.
///
/// Achieves GC balance by construction using the Yang rule's AT/GC partition.
/// Context-dependent Yin rule reduces homopolymer formation.
/// Encodes 2 bits per nucleotide.
pub struct YinYangCodec {
    config: YinYangConfig,
}

impl YinYangCodec {
    /// Create a new Yin-Yang codec with the given configuration.
    pub fn new(config: YinYangConfig) -> Self {
        Self { config }
    }

    /// Create a codec with default settings.
    pub fn default_codec() -> Self {
        Self::new(YinYangConfig::default())
    }

    /// Access the configuration.
    pub fn config(&self) -> &YinYangConfig {
        &self.config
    }

    /// Encode a data buffer into nucleotides using the Yin-Yang rules.
    ///
    /// The data is treated as a bitstream. We split it into two parallel
    /// streams: bits at even positions → Segment A, odd positions → Segment B.
    /// Each (bit_a, bit_b) pair encodes one nucleotide.
    fn encode_bits(data: &[u8], prev_base: Nucleotide) -> Result<Vec<Nucleotide>, DnaError> {
        let mut nucleotides = Vec::with_capacity(data.len() * 4);
        let mut prev = prev_base;

        for &byte in data {
            // Each byte = 8 bits = 4 nucleotides (2 bits each)
            for shift in (0..8).step_by(2) {
                let pair = (byte >> (6 - shift)) & 0b11;
                let bit_a = (pair >> 1) & 1; // high bit → Yang (GC selection)
                let bit_b = pair & 1;        // low bit → Yin (context selection)

                let y_set = yang_set(bit_a);
                let x_set = yin_set(bit_b, prev);
                let nt = intersect(&y_set, &x_set)?;

                nucleotides.push(nt);
                prev = nt;
            }
        }

        Ok(nucleotides)
    }

    /// Decode nucleotides back to data bytes.
    fn decode_bits(
        nucleotides: &[Nucleotide],
        prev_base: Nucleotide,
    ) -> Result<Vec<u8>, DnaError> {
        if nucleotides.len() % 4 != 0 {
            return Err(DnaError::DecodingError(format!(
                "nucleotide count {} not a multiple of 4",
                nucleotides.len()
            )));
        }

        let mut bytes = Vec::with_capacity(nucleotides.len() / 4);
        let mut prev = prev_base;

        for chunk in nucleotides.chunks(4) {
            let mut byte: u8 = 0;
            for (i, &nt) in chunk.iter().enumerate() {
                let bit_a = yang_decode(nt);
                let bit_b = yin_decode(nt, prev);
                let pair = (bit_a << 1) | bit_b;
                byte |= pair << (6 - i * 2);
                prev = nt;
            }
            bytes.push(byte);
        }

        Ok(bytes)
    }
}

/// The virtual initial base used for the Yin rule's context.
const VIRTUAL_BASE: Nucleotide = Nucleotide::A;

impl DnaCodec for YinYangCodec {
    fn name(&self) -> &str {
        "yin-yang"
    }

    fn encode(&self, data: &[u8]) -> Result<StrandCollection, DnaError> {
        if data.is_empty() {
            return Err(DnaError::EncodingError("empty input data".into()));
        }

        // Step 1: Prepend a 4-byte (u32) length header
        let len = data.len() as u32;
        let mut payload = len.to_be_bytes().to_vec();
        payload.extend_from_slice(data);

        // Step 2: Encode the entire payload to nucleotides
        let all_nucs = Self::encode_bits(&payload, VIRTUAL_BASE)?;

        // Step 3: Segment into strands with index headers
        // Strand structure: [2-byte index → 8 nt (direct 2-bit map)] [payload nucleotides]
        let strand_payload_len = if self.config.strand_length > 8 {
            self.config.strand_length - 8
        } else {
            4 // minimum
        };

        let validator = if self.config.screen_constraints {
            Some(ConstraintValidator::new(self.config.constraint_config.clone()))
        } else {
            None
        };

        let mut collection = StrandCollection::new(data.len());

        for (idx, chunk) in all_nucs.chunks(strand_payload_len).enumerate() {
            // Encode strand index as 2 bytes → 8 nucleotides using direct encoding
            let idx_u16 = idx as u16;
            let idx_bytes = idx_u16.to_be_bytes();
            let idx_nucs = Self::encode_bits(&idx_bytes, VIRTUAL_BASE)?;

            // Assemble strand: [index nt][payload nt]
            let mut strand_nucs = Vec::with_capacity(8 + chunk.len());
            strand_nucs.extend(&idx_nucs);
            strand_nucs.extend_from_slice(chunk);

            let strand = DnaStrand::new(strand_nucs);

            // Optional constraint screening (advisory — include strand regardless)
            if let Some(ref v) = validator {
                let _ = v.is_valid(&strand);
            }

            collection.push(strand);
        }

        Ok(collection)
    }

    fn decode(&self, strands: &StrandCollection) -> Result<Vec<u8>, DnaError> {
        if strands.strands.is_empty() {
            return Err(DnaError::DecodingError("no strands to decode".into()));
        }

        // Step 1: Parse each strand — extract index and payload nucleotides
        let mut indexed_payloads: Vec<(usize, Vec<Nucleotide>)> = Vec::new();

        for strand in &strands.strands {
            let bases = strand.bases();

            if bases.len() < 12 {
                return Err(DnaError::DecodingError(
                    "strand too short for yin-yang header".into(),
                ));
            }

            // Decode 8-nt index header → 2 bytes
            let idx_nucs = &bases[0..8];
            let idx_bytes = Self::decode_bits(idx_nucs, VIRTUAL_BASE)?;
            let idx = u16::from_be_bytes([idx_bytes[0], idx_bytes[1]]) as usize;

            // Payload is everything after the 8-nt header
            let payload = bases[8..].to_vec();
            indexed_payloads.push((idx, payload));
        }

        // Step 2: Sort by index and concatenate nucleotides
        indexed_payloads.sort_by_key(|(idx, _)| *idx);

        let mut all_nucs: Vec<Nucleotide> = Vec::new();
        for (_idx, payload) in &indexed_payloads {
            all_nucs.extend(payload);
        }

        // Step 3: Decode nucleotides back to bytes
        // Ensure nucleotide count is a multiple of 4 (trim padding if needed)
        let usable_len = (all_nucs.len() / 4) * 4;
        let all_bytes = Self::decode_bits(&all_nucs[..usable_len], VIRTUAL_BASE)?;

        // Step 4: Extract length header and return original data
        if all_bytes.len() < 4 {
            return Err(DnaError::DecodingError(
                "decoded data too short for length header".into(),
            ));
        }

        let original_len = u32::from_be_bytes([
            all_bytes[0],
            all_bytes[1],
            all_bytes[2],
            all_bytes[3],
        ]) as usize;

        if all_bytes.len() < 4 + original_len {
            return Err(DnaError::DecodingError(format!(
                "decoded {} bytes but header says {} expected",
                all_bytes.len() - 4,
                original_len
            )));
        }

        Ok(all_bytes[4..4 + original_len].to_vec())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yang_gc_partition() {
        // Yang bit=0 → AT bases, bit=1 → GC bases
        let at = yang_set(0);
        assert!(at.contains(&Nucleotide::A));
        assert!(at.contains(&Nucleotide::T));

        let gc = yang_set(1);
        assert!(gc.contains(&Nucleotide::C));
        assert!(gc.contains(&Nucleotide::G));
    }

    #[test]
    fn test_intersection_always_unique() {
        // For every combination of (bit_a, bit_b, prev_base),
        // the intersection should yield exactly one nucleotide
        for &prev in &Nucleotide::ALL {
            for bit_a in 0..=1u8 {
                for bit_b in 0..=1u8 {
                    let y = yang_set(bit_a);
                    let x = yin_set(bit_b, prev);
                    let result = intersect(&y, &x);
                    assert!(
                        result.is_ok(),
                        "empty intersection: prev={:?} bit_a={} bit_b={}",
                        prev, bit_a, bit_b
                    );
                }
            }
        }
    }

    #[test]
    fn test_encode_decode_roundtrip_all_bytes() {
        // Every single byte should survive a roundtrip
        for byte in 0..=255u8 {
            let data = vec![byte];
            let nucs = YinYangCodec::encode_bits(&data, VIRTUAL_BASE).unwrap();
            let decoded = YinYangCodec::decode_bits(&nucs, VIRTUAL_BASE).unwrap();
            assert_eq!(decoded, data, "roundtrip failed for byte {}", byte);
        }
    }

    #[test]
    fn test_yang_decode_inverse() {
        // yang_decode should invert yang_set
        for bit_a in 0..=1u8 {
            let set = yang_set(bit_a);
            for &nt in &set {
                assert_eq!(yang_decode(nt), bit_a, "yang_decode mismatch for {:?}", nt);
            }
        }
    }

    #[test]
    fn test_yin_decode_inverse() {
        // yin_decode should invert yin_set
        for &prev in &Nucleotide::ALL {
            for bit_b in 0..=1u8 {
                let set = yin_set(bit_b, prev);
                for &nt in &set {
                    assert_eq!(
                        yin_decode(nt, prev),
                        bit_b,
                        "yin_decode mismatch for {:?} prev={:?}",
                        nt,
                        prev
                    );
                }
            }
        }
    }

    #[test]
    fn test_encode_decode_text() {
        let codec = YinYangCodec::default_codec();
        let data = b"Hello, Yin-Yang DNA codec!";

        let encoded = codec.encode(data).unwrap();
        assert!(!encoded.strands.is_empty());

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data.to_vec());
    }

    #[test]
    fn test_encode_decode_binary() {
        let codec = YinYangCodec::default_codec();
        let data: Vec<u8> = (0..=255).collect();

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_all_zeros() {
        let codec = YinYangCodec::default_codec();
        let data = vec![0u8; 100];

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_all_ones() {
        let codec = YinYangCodec::default_codec();
        let data = vec![0xFFu8; 100];

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_gc_balance() {
        let codec = YinYangCodec::default_codec();
        // Random-ish data
        let data: Vec<u8> = (0..500).map(|i| (i * 37 + 13) as u8).collect();

        let encoded = codec.encode(&data).unwrap();

        let total_nt: usize = encoded.strands.iter().map(|s| s.len()).sum();
        let weighted_gc: f64 = encoded
            .strands
            .iter()
            .map(|s| s.gc_content() * s.len() as f64)
            .sum();
        let gc_frac = weighted_gc / total_nt as f64;
        // Yin-Yang should achieve GC between 35-65% on varied data
        assert!(
            gc_frac > 0.35 && gc_frac < 0.65,
            "GC content {:.1}% out of expected range",
            gc_frac * 100.0
        );
    }

    #[test]
    fn test_density() {
        let codec = YinYangCodec::default_codec();
        let data: Vec<u8> = (0..500u16).map(|i| i as u8).collect();

        let encoded = codec.encode(&data).unwrap();
        let bpn = encoded.bits_per_nucleotide();

        // Yin-Yang at 2 bits/nt theoretical should achieve >1.5 effective
        assert!(
            bpn > 1.5,
            "bits_per_nucleotide {:.3} too low for yin-yang",
            bpn
        );
    }

    #[test]
    fn test_large_data_roundtrip() {
        let codec = YinYangCodec::default_codec();
        let data: Vec<u8> = (0..2000).map(|i| (i % 256) as u8).collect();

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }
}
