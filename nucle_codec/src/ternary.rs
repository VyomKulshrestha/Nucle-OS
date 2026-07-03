//! # Ternary Rotating Cipher Codec (Goldman et al., 2013)
//!
//! Encodes binary data into DNA sequences using a ternary (base-3)
//! intermediate representation with a rotating cipher that
//! **eliminates all homopolymer runs by construction**.
//!
//! ## Algorithm
//!
//! 1. Convert binary data to base-3 (ternary) digits
//! 2. Map each ternary digit to a nucleotide using a rotating rule:
//!    given the previous base, choose from the 3 remaining bases
//! 3. This guarantees no two consecutive bases are the same
//!
//! ## Properties
//!
//! - **Density**: ~1.58 bits/nucleotide (log₂(3))
//! - **Homopolymers**: Impossible by construction
//! - **Overlapping segments**: Optional redundancy via overlapping encoding
//!
//! ## Reference
//!
//! Goldman, N., et al. (2013). "Towards practical, high-capacity,
//! low-maintenance information storage in synthesized DNA."
//! Nature, 494(7435), 77-80.

use crate::base::{DnaCodec, DnaError, DnaStrand, Nucleotide, StrandCollection};
use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the ternary rotating cipher codec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TernaryConfig {
    /// Maximum nucleotides per strand (payload only, excluding primers).
    /// Typical: 100–200 nt.
    pub strand_length: usize,

    /// Number of nucleotides of overlap between consecutive segments.
    /// Higher overlap = more redundancy = better error tolerance.
    /// Goldman et al. used 75% overlap (4× coverage).
    /// Set to 0 for no overlap.
    pub overlap: usize,

    /// Initial "seed" nucleotide for the rotating cipher.
    /// The first trit is mapped relative to this base.
    pub seed_base: Nucleotide,
}

impl Default for TernaryConfig {
    fn default() -> Self {
        Self {
            strand_length: 150,
            overlap: 75, // 50% overlap for 2× redundancy
            seed_base: Nucleotide::A,
        }
    }
}

impl TernaryConfig {
    /// No overlap — each data segment encoded once.
    pub fn no_overlap() -> Self {
        Self {
            strand_length: 150,
            overlap: 0,
            seed_base: Nucleotide::A,
        }
    }

    /// High redundancy — 75% overlap (4× coverage like Goldman).
    pub fn high_redundancy() -> Self {
        Self {
            strand_length: 150,
            overlap: 112, // 75% of 150
            seed_base: Nucleotide::A,
        }
    }
}

// ---------------------------------------------------------------------------
// Ternary Codec
// ---------------------------------------------------------------------------

/// Ternary rotating cipher codec.
///
/// Implements Goldman et al.'s encoding scheme:
/// binary → ternary → DNA (with rotating cipher to avoid homopolymers).
pub struct TernaryCodec {
    config: TernaryConfig,
}

impl TernaryCodec {
    /// Create a new ternary codec with the given configuration.
    pub fn new(config: TernaryConfig) -> Self {
        Self { config }
    }

    /// Create a codec with default settings.
    pub fn default_codec() -> Self {
        Self::new(TernaryConfig::default())
    }

    /// Access the configuration.
    pub fn config(&self) -> &TernaryConfig {
        &self.config
    }

    /// Convert a byte slice to a vector of ternary (base-3) digits.
    ///
    /// Each byte (0–255) is converted to base-3 representation.
    /// A byte requires ceil(log₃(256)) = 6 ternary digits.
    ///
    /// We prepend the original data length as a 4-byte (u32) header
    /// so decoding knows where the real data ends.
    fn bytes_to_trits(data: &[u8]) -> Vec<u8> {
        let mut trits = Vec::new();

        // Encode data length as a 4-byte header (u32 big-endian)
        let len = data.len() as u32;
        let len_bytes = len.to_be_bytes();
        for &byte in &len_bytes {
            Self::byte_to_trits(byte, &mut trits);
        }

        // Encode each data byte
        for &byte in data {
            Self::byte_to_trits(byte, &mut trits);
        }

        trits
    }

    /// Convert a single byte to 6 ternary digits (base-3).
    /// 3^6 = 729 > 256, so 6 trits can represent any byte.
    fn byte_to_trits(byte: u8, trits: &mut Vec<u8>) {
        let mut value = byte as u16;
        let mut trit_buf = [0u8; 6];
        for i in (0..6).rev() {
            trit_buf[i] = (value % 3) as u8;
            value /= 3;
        }
        trits.extend_from_slice(&trit_buf);
    }

    /// Convert ternary digits back to bytes.
    /// Every 6 trits = 1 byte.
    fn trits_to_bytes(trits: &[u8]) -> Result<Vec<u8>, DnaError> {
        if trits.len() % 6 != 0 {
            return Err(DnaError::DecodingError(format!(
                "trit count {} is not a multiple of 6",
                trits.len()
            )));
        }

        let mut bytes = Vec::new();
        for chunk in trits.chunks(6) {
            let mut value: u16 = 0;
            for &trit in chunk {
                if trit > 2 {
                    return Err(DnaError::DecodingError(format!(
                        "invalid ternary digit: {}",
                        trit
                    )));
                }
                value = value * 3 + trit as u16;
            }
            if value > 255 {
                return Err(DnaError::DecodingError(format!(
                    "trit sequence decodes to value {} > 255",
                    value
                )));
            }
            bytes.push(value as u8);
        }

        Ok(bytes)
    }

    /// Encode ternary digits into nucleotides using the rotating cipher.
    ///
    /// Given the previous nucleotide, each trit (0, 1, 2) maps to one
    /// of the three remaining nucleotides. This guarantees no two
    /// consecutive nucleotides are the same.
    fn trits_to_nucleotides(
        trits: &[u8],
        seed: Nucleotide,
    ) -> Result<Vec<Nucleotide>, DnaError> {
        let mut nucleotides = Vec::with_capacity(trits.len());
        let mut prev = seed;

        for &trit in trits {
            let nt = Nucleotide::from_trit(trit, prev)?;
            nucleotides.push(nt);
            prev = nt;
        }

        Ok(nucleotides)
    }

    /// Decode nucleotides back to ternary digits using the rotating cipher.
    fn nucleotides_to_trits(
        nucleotides: &[Nucleotide],
        seed: Nucleotide,
    ) -> Result<Vec<u8>, DnaError> {
        let mut trits = Vec::with_capacity(nucleotides.len());
        let mut prev = seed;

        for &nt in nucleotides {
            let trit = nt.to_trit(prev)?;
            trits.push(trit);
            prev = nt;
        }

        Ok(trits)
    }

    /// Split ternary data into overlapping segments for strand encoding.
    fn segment_trits(&self, trits: &[u8]) -> Vec<Vec<u8>> {
        let strand_len = self.config.strand_length;
        let step = if self.config.overlap >= strand_len {
            1 // Prevent zero or negative step
        } else {
            strand_len - self.config.overlap
        };

        let mut segments = Vec::new();

        if trits.is_empty() {
            return segments;
        }

        let mut start = 0;
        while start < trits.len() {
            let end = (start + strand_len).min(trits.len());
            let mut segment = trits[start..end].to_vec();

            // Pad the last segment to full strand length. This filler is
            // discarded on decode (only the length-prefixed real data
            // survives), so its *value* doesn't matter -- but a constant
            // trit here is a real bug, not a harmless simplification: the
            // rotating cipher maps a fixed trit to a fixed rotation
            // relative to whatever base precedes it, and a fixed rotation
            // applied repeatedly cycles through a short, fixed set of
            // bases (e.g. trit 0 degenerates into a "TATATATA..." repeat
            // once the cipher's running state lands on T or A). That
            // self-inflicted tandem repeat is exactly the kind of region
            // that's hardest for any downstream consensus/alignment
            // scheme to recover correctly under indel noise, for reasons
            // that have nothing to do with the real data -- confirmed as
            // the actual cause of residual Nanopore consensus errors that
            // looked, at first, like a fundamental alignment limitation.
            //
            // A cheap deterministic PRNG (xorshift32, fixed seed so
            // encoding stays fully reproducible) breaks the periodicity.
            // It has to fill in whole `byte_to_trits`-shaped groups of 6,
            // not raw random trits one at a time: `trits_to_bytes` insists
            // every 6-trit group decode to a valid byte (<=255) and only
            // ~256 of the 3^6=729 possible 6-trit combinations qualify, a
            // constraint the old constant-zero filler satisfied by luck
            // (0 always decodes to byte 0) that a per-trit random filler
            // would violate most of the time.
            let mut filler_state: u32 = 0x9e37_79b9;
            while segment.len() < strand_len {
                filler_state ^= filler_state << 13;
                filler_state ^= filler_state >> 17;
                filler_state ^= filler_state << 5;
                let mut filler_trits = Vec::new();
                Self::byte_to_trits((filler_state % 256) as u8, &mut filler_trits);
                let remaining = strand_len - segment.len();
                segment.extend(filler_trits.into_iter().take(remaining));
            }

            segments.push(segment);

            if end >= trits.len() {
                break;
            }
            start += step;
        }

        segments
    }

    /// A deterministic, position-addressable pseudo-random trit: the same
    /// `position` always yields the same value, computed independently of
    /// any other position (no sequential state to advance). That's the
    /// property whitening needs here: overlapping segments each contain
    /// the *same* absolute trit position more than once, and a strand's
    /// own position range is only known (from its index header) one
    /// strand at a time during decode, in whatever order retrieval
    /// happens to return them -- a stateful "advance once per trit"
    /// generator can't be replayed correctly under either condition, only
    /// one addressed purely by position can. (A splitmix64-style mixing
    /// round; not cryptographic, just a cheap decorrelator.)
    fn whiten_value_at(position: usize) -> u8 {
        let mut x = (position as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        (x % 3) as u8
    }

    /// XOR-add (mod 3) the position-addressable whitening stream onto
    /// `trits`, starting at absolute position `base_pos`, in place.
    ///
    /// The rotating cipher maps a constant trit value to a constant
    /// rotation relative to whatever base precedes it, so any long run of
    /// a single trit value degenerates into a short, fixed-period base
    /// cycle once fed through it (e.g. a run of trit `0` becomes a
    /// "TATATATA..." repeat) -- a self-inflicted tandem repeat with
    /// nothing to do with the real data, and exactly the kind of region
    /// that's hardest for any downstream consensus/alignment scheme to
    /// recover under indel noise. Such runs aren't a corner case: the
    /// 4-byte big-endian length header this codec prepends to every
    /// payload has at least one, usually several, leading zero *bytes*
    /// for any file under 16MB, and a zero byte is 6 trits of value 0 --
    /// so every encoded file starts with a real (if short) instance of
    /// this bug, not just unlucky inputs.
    ///
    /// This is applied to a whole segment -- padding included, not just
    /// the real data before it -- and reversed the same way per-strand at
    /// decode (using that strand's own known index to recover its
    /// `base_pos`), rather than applied once globally before segmentation
    /// and reversed once after reassembly: decode doesn't know where any
    /// given strand's real-data/padding boundary falls until *after*
    /// decoding the length header, so unwhitening has to work without
    /// needing that boundary, which per-segment/per-strand positioning
    /// gives for free.
    fn whiten_segment(segment: &mut [u8], base_pos: usize, reverse: bool) {
        for (j, t) in segment.iter_mut().enumerate() {
            let r = Self::whiten_value_at(base_pos + j);
            *t = if reverse { (*t + 3 - r) % 3 } else { (*t + r) % 3 };
        }
    }
}

impl DnaCodec for TernaryCodec {
    fn name(&self) -> &str {
        "ternary-rotating-cipher"
    }

    fn encode(&self, data: &[u8]) -> Result<StrandCollection, DnaError> {
        if data.is_empty() {
            return Err(DnaError::EncodingError("empty input data".into()));
        }

        // Step 1: Convert bytes → ternary digits (with length header)
        let trits = Self::bytes_to_trits(data);

        // Step 2: Segment the trits into strand-length chunks
        let mut segments = self.segment_trits(&trits);

        // Whiten every segment (including its padding, if any) before the
        // rotating cipher ever sees it -- see `whiten_segment`'s doc
        // comment for why a plain length-header + data trit stream is not
        // safe to feed it directly.
        let step = if self.config.overlap >= self.config.strand_length {
            1
        } else {
            self.config.strand_length - self.config.overlap
        };
        for (idx, segment) in segments.iter_mut().enumerate() {
            Self::whiten_segment(segment, idx * step, false);
        }

        // Step 3: Encode each segment into nucleotides
        let mut collection = StrandCollection::new(data.len());

        for (idx, segment) in segments.iter().enumerate() {
            let nucleotides =
                Self::trits_to_nucleotides(segment, self.config.seed_base)?;

            // Prepend a 4-trit strand index header for ordering during decode
            let mut strand_data = Vec::new();
            // Encode index as 4 trits (supports up to 3^4 = 81 strands without overlap)
            let idx_trits = [
                ((idx / 27) % 3) as u8,
                ((idx / 9) % 3) as u8,
                ((idx / 3) % 3) as u8,
                (idx % 3) as u8,
            ];
            let idx_nucs =
                Self::trits_to_nucleotides(&idx_trits, self.config.seed_base)?;
            strand_data.extend(idx_nucs);
            strand_data.extend(nucleotides);

            collection.push(DnaStrand::new(strand_data));
        }

        Ok(collection)
    }

    fn decode(&self, strands: &StrandCollection) -> Result<Vec<u8>, DnaError> {
        if strands.strands.is_empty() {
            return Err(DnaError::DecodingError("no strands to decode".into()));
        }

        // Step 1: Extract strand indices and payload nucleotides
        let mut indexed_payloads: Vec<(usize, Vec<Nucleotide>)> = Vec::new();

        for strand in &strands.strands {
            let bases = strand.bases();
            if bases.len() < 5 {
                return Err(DnaError::DecodingError(
                    "strand too short for header".into(),
                ));
            }

            // Decode the 4-trit index header
            let idx_nucs = &bases[0..4];
            let idx_trits =
                Self::nucleotides_to_trits(idx_nucs, self.config.seed_base)?;
            let idx = idx_trits[0] as usize * 27
                + idx_trits[1] as usize * 9
                + idx_trits[2] as usize * 3
                + idx_trits[3] as usize;

            // Payload is everything after the 4-trit header
            let payload = bases[4..].to_vec();
            indexed_payloads.push((idx, payload));
        }

        // Step 2: Sort by index and reconstruct
        indexed_payloads.sort_by_key(|(idx, _)| *idx);

        let step = if self.config.overlap >= self.config.strand_length {
            1
        } else {
            self.config.strand_length - self.config.overlap
        };

        // Step 3: Use the first strand set (no overlap reconstruction needed
        // for basic decoding — overlap provides redundancy for error correction)
        if self.config.overlap == 0 {
            // No overlap: concatenate all payloads in order
            let mut all_trits = Vec::new();

            for (idx, payload) in &indexed_payloads {
                let mut trits = Self::nucleotides_to_trits(
                    payload,
                    self.config.seed_base,
                )?;
                // Reverse the per-segment whitening `encode` applied,
                // using this strand's own index for its absolute
                // position -- see `whiten_segment`'s doc comment for why
                // this has to happen per-strand rather than once after
                // reassembly.
                Self::whiten_segment(&mut trits, *idx * step, true);
                all_trits.extend(trits);
            }

            // Decode trits to bytes
            // First 24 trits (4 bytes) are the length header
            if all_trits.len() < 24 {
                return Err(DnaError::DecodingError(
                    "insufficient data for length header".into(),
                ));
            }

            let all_bytes = Self::trits_to_bytes(&all_trits)?;

            // Extract original length from header
            if all_bytes.len() < 4 {
                return Err(DnaError::DecodingError(
                    "insufficient bytes for length header".into(),
                ));
            }

            let original_len = u32::from_be_bytes([
                all_bytes[0],
                all_bytes[1],
                all_bytes[2],
                all_bytes[3],
            ]) as usize;

            // Extract the original data
            let data_start = 4;
            let data_end = data_start + original_len;

            if data_end > all_bytes.len() {
                return Err(DnaError::DecodingError(format!(
                    "claimed length {} exceeds decoded data ({} bytes available)",
                    original_len,
                    all_bytes.len() - data_start
                )));
            }

            Ok(all_bytes[data_start..data_end].to_vec())
        } else {
            // With overlap: use the non-overlapping portions of each strand
            let mut all_trits = Vec::new();

            for (i, (idx, payload)) in indexed_payloads.iter().enumerate() {
                let mut trits = Self::nucleotides_to_trits(
                    payload,
                    self.config.seed_base,
                )?;
                Self::whiten_segment(&mut trits, *idx * step, true);

                if i == indexed_payloads.len() - 1 {
                    // Last strand: take all remaining trits
                    all_trits.extend(&trits);
                } else {
                    // Take only the non-overlapping portion
                    let take = step.min(trits.len());
                    all_trits.extend(&trits[..take]);
                }
            }

            // Ensure trit count is a multiple of 6 for byte conversion
            let usable_len = (all_trits.len() / 6) * 6;
            let all_bytes = Self::trits_to_bytes(&all_trits[..usable_len])?;

            if all_bytes.len() < 4 {
                return Err(DnaError::DecodingError(
                    "insufficient bytes for length header".into(),
                ));
            }

            let original_len = u32::from_be_bytes([
                all_bytes[0],
                all_bytes[1],
                all_bytes[2],
                all_bytes[3],
            ]) as usize;

            let data_start = 4;
            let data_end = data_start + original_len;

            if data_end > all_bytes.len() {
                return Err(DnaError::DecodingError(format!(
                    "claimed length {} exceeds decoded data ({} bytes available)",
                    original_len,
                    all_bytes.len() - data_start
                )));
            }

            Ok(all_bytes[data_start..data_end].to_vec())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_to_trits_roundtrip() {
        for byte in 0..=255u8 {
            let mut trits = Vec::new();
            TernaryCodec::byte_to_trits(byte, &mut trits);
            assert_eq!(trits.len(), 6);

            // Verify all trits are 0, 1, or 2
            for &t in &trits {
                assert!(t <= 2, "trit {} out of range for byte {}", t, byte);
            }

            // Roundtrip
            let bytes = TernaryCodec::trits_to_bytes(&trits).unwrap();
            assert_eq!(bytes.len(), 1);
            assert_eq!(bytes[0], byte, "roundtrip failed for byte {}", byte);
        }
    }

    #[test]
    fn test_trits_to_nucleotides_no_homopolymers() {
        let trits: Vec<u8> = vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 0];
        let nucs =
            TernaryCodec::trits_to_nucleotides(&trits, Nucleotide::A).unwrap();

        // Verify no two consecutive nucleotides are the same
        for i in 1..nucs.len() {
            assert_ne!(
                nucs[i], nucs[i - 1],
                "homopolymer at position {}: {:?}{:?}",
                i, nucs[i - 1], nucs[i]
            );
        }
    }

    #[test]
    fn test_nucleotide_trit_roundtrip_all_seeds() {
        let trits: Vec<u8> = vec![0, 1, 2, 2, 1, 0, 0, 0, 1, 1, 2, 2];
        for seed in Nucleotide::ALL {
            let nucs =
                TernaryCodec::trits_to_nucleotides(&trits, seed).unwrap();
            let decoded =
                TernaryCodec::nucleotides_to_trits(&nucs, seed).unwrap();
            assert_eq!(trits, decoded, "roundtrip failed with seed {:?}", seed);
        }
    }

    #[test]
    fn test_encode_decode_no_overlap() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Hello, DNA!";

        let encoded = codec.encode(data).unwrap();
        assert!(!encoded.strands.is_empty());

        // Verify no homopolymers in any strand
        for strand in &encoded.strands {
            let (_, max_run) = strand.max_homopolymer_run();
            assert!(max_run <= 1, "homopolymer found in encoded strand");
        }

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(
            decoded, data,
            "decoded data doesn't match original"
        );
    }

    #[test]
    fn test_encode_decode_with_overlap() {
        let codec = TernaryCodec::new(TernaryConfig::default());
        let data = b"DNA storage is the future of data archival.";

        let encoded = codec.encode(data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_binary_data() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        // All possible byte values
        let data: Vec<u8> = (0..=255).collect();

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_single_byte() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = vec![42u8];

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_density_metrics() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = vec![0u8; 100]; // 100 bytes

        let encoded = codec.encode(&data).unwrap();

        // Should achieve roughly 1.58 bits/nt
        // With the 4-byte header overhead on 100 bytes, density will be lower
        let bpn = encoded.bits_per_nucleotide();
        assert!(
            bpn > 1.0 && bpn < 2.0,
            "bits_per_nucleotide {} outside expected range",
            bpn
        );
    }

    #[test]
    fn test_empty_input_error() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let result = codec.encode(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_strand_count_with_overlap() {
        let codec_no_overlap = TernaryCodec::new(TernaryConfig::no_overlap());
        let codec_overlap = TernaryCodec::new(TernaryConfig::default());
        let data = vec![0u8; 200]; // Enough data to require multiple strands

        let enc_no = codec_no_overlap.encode(&data).unwrap();
        let enc_yes = codec_overlap.encode(&data).unwrap();

        // Overlap should produce more strands (redundancy)
        assert!(
            enc_yes.strand_count() >= enc_no.strand_count(),
            "overlap ({}) should produce >= strands than no-overlap ({})",
            enc_yes.strand_count(),
            enc_no.strand_count()
        );
    }
}
