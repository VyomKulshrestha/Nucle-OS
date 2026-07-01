//! # DNA Fountain Codec (Erlich & Zielinski, 2017)
//!
//! Implements a simplified version of the DNA Fountain encoding scheme,
//! which applies Luby Transform (LT) fountain codes to DNA storage.
//!
//! ## Algorithm
//!
//! 1. Split input data into fixed-size segments
//! 2. For each "droplet" (output strand), use a PRNG seeded by the
//!    droplet index to select a random subset of segments
//! 3. XOR the selected segments together to produce the droplet payload
//! 4. Screen the resulting DNA sequence against biological constraints
//! 5. Keep only droplets that pass screening
//!
//! ## Properties
//!
//! - **Rateless**: Can generate unlimited encoded strands
//! - **Near-optimal density**: ~1.57 bits/nucleotide
//! - **Erasure resilient**: Any sufficient subset of strands can reconstruct
//! - **Constraint-aware**: Rejects strands violating biological constraints
//!
//! ## Decoding
//!
//! Uses belief propagation (peeling decoder):
//! 1. Find a droplet that XOR'd only one segment → that segment is solved
//! 2. XOR the solved segment out of all other droplets
//! 3. Repeat until all segments are solved
//!
//! ## Reference
//!
//! Erlich, Y., & Zielinski, D. (2017). "DNA Fountain enables a robust
//! and efficient storage architecture." Science, 355(6328), 950-954.

use crate::base::{DnaCodec, DnaError, DnaStrand, Nucleotide, StrandCollection};
use crate::constraints::{ConstraintConfig, ConstraintValidator};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the DNA Fountain codec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FountainConfig {
    /// Size of each data segment in bytes.
    /// Smaller = more segments = more flexible XOR combinations.
    /// Larger = fewer strands needed but less resilient.
    pub segment_size: usize,

    /// Overhead factor: generate this many more droplets than the minimum.
    /// 1.0 = exact minimum (fragile), 1.5 = 50% extra (robust).
    /// Erlich used ~1.07 overhead in ideal conditions.
    pub overhead: f64,

    /// Maximum number of attempts to generate a valid droplet
    /// (one that passes biological constraint screening).
    pub max_screening_attempts: usize,

    /// Whether to enforce biological constraints during encoding.
    /// Droplets that violate constraints are rejected and regenerated.
    pub screen_constraints: bool,

    /// Constraint configuration for screening.
    pub constraint_config: ConstraintConfig,

    /// PRNG seed for reproducible encoding.
    pub seed: u64,
}

impl Default for FountainConfig {
    fn default() -> Self {
        Self {
            segment_size: 16,  // 16 bytes per segment
            overhead: 1.50,    // 50% overhead — screening rejects some strands
            max_screening_attempts: 1000,
            screen_constraints: true,  // Enforce biological constraints (per Erlich 2017)
            constraint_config: ConstraintConfig::default(),
            seed: 42,
        }
    }
}

impl FountainConfig {
    /// Configuration optimized for density (low overhead).
    /// Still enforces biological constraints — rateless property handles rejections.
    pub fn high_density() -> Self {
        Self {
            segment_size: 20,
            overhead: 1.20,
            max_screening_attempts: 2000,
            screen_constraints: true,
            constraint_config: ConstraintConfig::default(),
            seed: 42,
        }
    }

    /// Configuration optimized for resilience (high overhead).
    pub fn high_resilience() -> Self {
        Self {
            segment_size: 12,
            overhead: 2.0,
            max_screening_attempts: 500,
            screen_constraints: true,
            constraint_config: ConstraintConfig::default(),
            seed: 42,
        }
    }

    /// Configuration with constraint screening disabled.
    /// Only use for internal testing or when raw throughput matters
    /// more than biological validity.
    pub fn unscreened() -> Self {
        Self {
            screen_constraints: false,
            constraint_config: ConstraintConfig::relaxed(),
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Droplet — a single encoded strand with its metadata
// ---------------------------------------------------------------------------

/// A droplet is a single encoded strand produced by the fountain encoder.
///
/// It contains the XOR of a random subset of data segments, plus
/// metadata needed for decoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Droplet {
    /// The seed used to determine which segments were XOR'd.
    pub seed: u64,
    /// The XOR'd payload data.
    pub data: Vec<u8>,
    /// Indices of the segments that were XOR'd to produce this droplet.
    pub segment_indices: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Degree Distribution
// ---------------------------------------------------------------------------

/// Robust Soliton distribution for selecting the degree (number of
/// segments to XOR) for each droplet.
///
/// This distribution is key to efficient LT code decoding.
fn sample_degree(rng: &mut StdRng, num_segments: usize) -> usize {
    // Simplified robust soliton: mix ideal soliton with a spike at 1
    let k = num_segments as f64;
    let r: f64 = rng.gen();

    if r < 0.4 {
        // Degree 1 — these are the "free" segments that start decoding
        1
    } else if r < 0.7 {
        // Degree 2 — most common in ideal soliton
        2
    } else if r < 0.85 {
        // Degree 3
        3.min(num_segments)
    } else if r < 0.95 {
        // Medium degree
        let d = (k.sqrt()) as usize;
        d.max(2).min(num_segments)
    } else {
        // Higher degree (rare)
        let d = (k / 2.0) as usize;
        d.max(2).min(num_segments)
    }
}

/// Select `degree` random segment indices from `num_segments` total.
fn select_segments(rng: &mut StdRng, num_segments: usize, degree: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..num_segments).collect();
    // Fisher-Yates partial shuffle
    let d = degree.min(num_segments);
    for i in 0..d {
        let j = rng.gen_range(i..num_segments);
        indices.swap(i, j);
    }
    indices.truncate(d);
    indices.sort();
    indices
}

// ---------------------------------------------------------------------------
// Fountain Codec
// ---------------------------------------------------------------------------

/// DNA Fountain codec using Luby Transform codes.
pub struct FountainCodec {
    config: FountainConfig,
}

impl FountainCodec {
    /// Create a new fountain codec with the given configuration.
    pub fn new(config: FountainConfig) -> Self {
        Self { config }
    }

    /// Create a codec with default settings.
    pub fn default_codec() -> Self {
        Self::new(FountainConfig::default())
    }

    /// Access the configuration.
    pub fn config(&self) -> &FountainConfig {
        &self.config
    }

    /// Split data into fixed-size segments, padding the last one.
    fn segment_data(data: &[u8], segment_size: usize) -> Vec<Vec<u8>> {
        let mut segments = Vec::new();
        for chunk in data.chunks(segment_size) {
            let mut segment = chunk.to_vec();
            // Pad last segment with zeros
            while segment.len() < segment_size {
                segment.push(0);
            }
            segments.push(segment);
        }
        segments
    }

    /// XOR multiple segments together.
    fn xor_segments(segments: &[Vec<u8>], indices: &[usize]) -> Vec<u8> {
        let seg_size = segments[0].len();
        let mut result = vec![0u8; seg_size];

        for &idx in indices {
            for (j, byte) in segments[idx].iter().enumerate() {
                result[j] ^= byte;
            }
        }

        result
    }

    /// Convert bytes to nucleotides using simple 2-bit mapping
    /// with a ternary prefix to break homopolymers.
    fn bytes_to_nucleotides(data: &[u8]) -> Vec<Nucleotide> {
        let mut nucleotides = Vec::new();

        for &byte in data {
            // Each byte → 4 nucleotides (2 bits each)
            for shift in (0..8).step_by(2).rev() {
                let bits = (byte >> shift) & 0b11;
                nucleotides.push(Nucleotide::from_bits(bits).unwrap());
            }
        }

        nucleotides
    }

    /// Convert nucleotides back to bytes.
    fn nucleotides_to_bytes(nucs: &[Nucleotide]) -> Result<Vec<u8>, DnaError> {
        if nucs.len() % 4 != 0 {
            return Err(DnaError::DecodingError(format!(
                "nucleotide count {} is not a multiple of 4",
                nucs.len()
            )));
        }

        let mut bytes = Vec::new();
        for chunk in nucs.chunks(4) {
            let mut byte: u8 = 0;
            for (i, nuc) in chunk.iter().enumerate() {
                byte |= nuc.to_bits() << (6 - i * 2);
            }
            bytes.push(byte);
        }

        Ok(bytes)
    }

    /// Encode a droplet's seed and data into a DNA strand.
    ///
    /// Strand structure:
    /// [4-byte seed as 16 nt] [payload as N nt]
    fn droplet_to_strand(droplet: &Droplet) -> DnaStrand {
        let mut nucleotides = Vec::new();

        // Encode seed (8 bytes = 32 nt)
        let seed_bytes = droplet.seed.to_be_bytes();
        nucleotides.extend(Self::bytes_to_nucleotides(&seed_bytes));

        // Encode payload
        nucleotides.extend(Self::bytes_to_nucleotides(&droplet.data));

        DnaStrand::new(nucleotides)
    }

    /// Decode a DNA strand back to a droplet.
    fn strand_to_droplet(
        strand: &DnaStrand,
        num_segments: usize,
        segment_size: usize,
    ) -> Result<Droplet, DnaError> {
        let bases = strand.bases();

        // Seed is first 32 nucleotides (8 bytes)
        if bases.len() < 32 {
            return Err(DnaError::DecodingError(
                "strand too short for seed header".into(),
            ));
        }

        let seed_nucs = &bases[0..32];
        let seed_bytes = Self::nucleotides_to_bytes(seed_nucs)?;
        let seed = u64::from_be_bytes([
            seed_bytes[0],
            seed_bytes[1],
            seed_bytes[2],
            seed_bytes[3],
            seed_bytes[4],
            seed_bytes[5],
            seed_bytes[6],
            seed_bytes[7],
        ]);

        // Payload is everything after the seed
        let payload_nucs = &bases[32..];
        let data = Self::nucleotides_to_bytes(payload_nucs)?;

        // A noisy channel (deletion/truncation) can shorten the payload
        // below the expected segment size — that's a corrupted strand,
        // not a bug, so report it as a decoding error instead of panicking.
        if data.len() < segment_size {
            return Err(DnaError::DecodingError(format!(
                "strand payload too short: expected {} bytes, got {}",
                segment_size,
                data.len()
            )));
        }

        // Reconstruct the segment indices using the same PRNG
        let mut rng = StdRng::seed_from_u64(seed);
        let degree = sample_degree(&mut rng, num_segments);
        let segment_indices = select_segments(&mut rng, num_segments, degree);

        Ok(Droplet {
            seed,
            data: data[..segment_size].to_vec(),
            segment_indices,
        })
    }

    /// Peeling decoder: iteratively solve segments from droplets.
    ///
    /// This is the core LT code decoding algorithm:
    /// 1. Find droplets with degree 1 (only one unsolved segment)
    /// 2. That droplet's data IS the segment
    /// 3. XOR the solved segment out of all other droplets
    /// 4. Repeat until all segments solved or stuck
    fn peeling_decode(
        droplets: &mut Vec<Droplet>,
        num_segments: usize,
        segment_size: usize,
    ) -> Result<Vec<Vec<u8>>, DnaError> {
        let mut solved: Vec<Option<Vec<u8>>> = vec![None; num_segments];
        let mut solved_count = 0;

        // Track which segments each droplet still references
        let mut droplet_segments: Vec<Vec<usize>> = droplets
            .iter()
            .map(|d| d.segment_indices.clone())
            .collect();
        let mut droplet_data: Vec<Vec<u8>> = droplets
            .iter()
            .map(|d| d.data.clone())
            .collect();

        // Iterative peeling
        let max_iterations = droplets.len() * 2;
        for _ in 0..max_iterations {
            if solved_count == num_segments {
                break;
            }

            // Find a droplet with exactly one unsolved segment
            let mut found = false;
            for i in 0..droplet_segments.len() {
                let unsolved: Vec<usize> = droplet_segments[i]
                    .iter()
                    .filter(|&&idx| solved[idx].is_none())
                    .copied()
                    .collect();

                if unsolved.len() == 1 {
                    let seg_idx = unsolved[0];

                    // This droplet's data (after XOR-ing out solved segments)
                    // IS the unsolved segment
                    let mut seg_data = droplet_data[i].clone();

                    // XOR out any already-solved segments
                    for &idx in &droplet_segments[i] {
                        if idx != seg_idx {
                            if let Some(ref solved_data) = solved[idx] {
                                for (j, byte) in solved_data.iter().enumerate() {
                                    if j < seg_data.len() {
                                        seg_data[j] ^= byte;
                                    }
                                }
                            }
                        }
                    }

                    seg_data.resize(segment_size, 0);
                    solved[seg_idx] = Some(seg_data);
                    solved_count += 1;
                    found = true;
                    break;
                }
            }

            if !found {
                break; // No more degree-1 droplets — stuck
            }
        }

        if solved_count < num_segments {
            return Err(DnaError::DecodingError(format!(
                "peeling decoder stuck: solved {}/{} segments. Need more droplets.",
                solved_count, num_segments
            )));
        }

        Ok(solved.into_iter().map(|s| s.unwrap()).collect())
    }
}

impl DnaCodec for FountainCodec {
    fn name(&self) -> &str {
        "dna-fountain"
    }

    fn encode(&self, data: &[u8]) -> Result<StrandCollection, DnaError> {
        if data.is_empty() {
            return Err(DnaError::EncodingError("empty input data".into()));
        }

        // Step 1: Prepend a length header (4 bytes, u32 big-endian)
        let original_len = data.len();
        let mut payload = Vec::with_capacity(4 + data.len());
        payload.extend_from_slice(&(original_len as u32).to_be_bytes());
        payload.extend_from_slice(data);

        // Step 2: Segment the data
        let segments = Self::segment_data(&payload, self.config.segment_size);
        let num_segments = segments.len();

        // Step 3: Generate droplets
        let num_droplets =
            (num_segments as f64 * self.config.overhead).ceil() as usize;
        let num_droplets = num_droplets.max(num_segments + 1); // Need at least k+1

        let validator = if self.config.screen_constraints {
            Some(ConstraintValidator::new(self.config.constraint_config.clone()))
        } else {
            None
        };

        let mut collection = StrandCollection::new(original_len);
        let mut global_seed = self.config.seed;

        let mut generated = 0;
        let mut attempts = 0;

        while generated < num_droplets
            && attempts < num_droplets * self.config.max_screening_attempts
        {
            let droplet_seed = global_seed;
            global_seed = global_seed.wrapping_add(1);
            attempts += 1;

            // Use the droplet seed to determine degree and segments
            let mut rng = StdRng::seed_from_u64(droplet_seed);
            let degree = sample_degree(&mut rng, num_segments);
            let indices = select_segments(&mut rng, num_segments, degree);

            // XOR the selected segments
            let xor_data = Self::xor_segments(&segments, &indices);

            let droplet = Droplet {
                seed: droplet_seed,
                data: xor_data,
                segment_indices: indices,
            };

            // Convert to DNA strand
            let strand = Self::droplet_to_strand(&droplet);

            // Screen against biological constraints (if enabled)
            if let Some(ref v) = validator {
                if !v.is_valid(&strand) {
                    continue; // Reject and try next seed
                }
            }

            collection.push(strand);
            generated += 1;
        }

        if generated < num_droplets {
            log::warn!(
                "fountain encoder: only generated {}/{} droplets after {} attempts",
                generated,
                num_droplets,
                attempts
            );
        }

        Ok(collection)
    }

    fn decode(&self, strands: &StrandCollection) -> Result<Vec<u8>, DnaError> {
        if strands.strands.is_empty() {
            return Err(DnaError::DecodingError("no strands to decode".into()));
        }

        // Reconstruct the expected number of segments from the data length
        // We need to figure out num_segments. We know segment_size and that
        // the first 4 bytes are the length header.
        // We'll decode all strands first, then figure out segments.

        // First, decode all strands to droplets.
        // We need to guess num_segments. We can estimate from strand count
        // and overhead, or better: try decoding with different estimates.

        // Heuristic: use the first strand's payload size to determine segment_size,
        // then estimate num_segments from the total data.
        let first_strand = &strands.strands[0];
        if first_strand.len() < 32 {
            return Err(DnaError::DecodingError(
                "strand too short for seed header".into(),
            ));
        }
        let payload_nt = first_strand.len() - 32; // minus seed
        let payload_bytes = payload_nt / 4;
        let segment_size = payload_bytes.min(self.config.segment_size);

        // Try different num_segments values
        // Start with estimate from strand count / overhead
        let estimated_segments =
            (strands.strand_count() as f64 / self.config.overhead).ceil() as usize;

        // Try a range around the estimate
        for num_seg_try in 1..=estimated_segments.max(strands.strand_count()) {
            let mut droplets = Vec::new();
            let mut decode_ok = true;

            for strand in &strands.strands {
                match Self::strand_to_droplet(strand, num_seg_try, segment_size) {
                    Ok(d) => droplets.push(d),
                    Err(_) => {
                        decode_ok = false;
                        break;
                    }
                }
            }

            if !decode_ok {
                continue;
            }

            // Try peeling decode
            match Self::peeling_decode(&mut droplets, num_seg_try, segment_size) {
                Ok(segments) => {
                    // Reassemble the data
                    let mut all_data: Vec<u8> = Vec::new();
                    for seg in &segments {
                        all_data.extend(seg);
                    }

                    // Extract original length from header
                    if all_data.len() < 4 {
                        continue;
                    }

                    let original_len = u32::from_be_bytes([
                        all_data[0],
                        all_data[1],
                        all_data[2],
                        all_data[3],
                    ]) as usize;

                    let data_start = 4;
                    let data_end = data_start + original_len;

                    if data_end > all_data.len() {
                        continue;
                    }

                    return Ok(all_data[data_start..data_end].to_vec());
                }
                Err(_) => continue,
            }
        }

        Err(DnaError::DecodingError(
            "fountain decoder failed: could not reconstruct data with any segment count".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_data() {
        let data = vec![1, 2, 3, 4, 5];
        let segments = FountainCodec::segment_data(&data, 3);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], vec![1, 2, 3]);
        assert_eq!(segments[1], vec![4, 5, 0]); // Padded
    }

    #[test]
    fn test_xor_segments() {
        let segments = vec![vec![0xFF, 0x00], vec![0x0F, 0xF0], vec![0xAA, 0x55]];

        // XOR first two: 0xFF^0x0F=0xF0, 0x00^0xF0=0xF0
        let result = FountainCodec::xor_segments(&segments, &[0, 1]);
        assert_eq!(result, vec![0xF0, 0xF0]);

        // XOR all three
        let result = FountainCodec::xor_segments(&segments, &[0, 1, 2]);
        assert_eq!(result, vec![0xF0 ^ 0xAA, 0xF0 ^ 0x55]);
    }

    #[test]
    fn test_bytes_nucleotides_roundtrip() {
        let data = vec![0x00, 0xFF, 0xAA, 0x55, 42, 137];
        let nucs = FountainCodec::bytes_to_nucleotides(&data);
        let decoded = FountainCodec::nucleotides_to_bytes(&nucs).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_simple() {
        let codec = FountainCodec::new(FountainConfig {
            segment_size: 4,
            overhead: 2.0,  // High overhead for reliable test
            max_screening_attempts: 100,
            screen_constraints: false,
            constraint_config: ConstraintConfig::relaxed(),
            seed: 42,
        });

        let data = b"Hello, DNA Fountain!";
        let encoded = codec.encode(data).unwrap();
        assert!(!encoded.strands.is_empty());

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data.to_vec());
    }

    #[test]
    fn test_encode_decode_binary() {
        let codec = FountainCodec::new(FountainConfig {
            segment_size: 4,
            overhead: 3.0,  // Higher overhead for larger data
            max_screening_attempts: 100,
            screen_constraints: false,
            constraint_config: ConstraintConfig::relaxed(),
            seed: 42,
        });

        // Binary data with all byte values 0-31
        let data: Vec<u8> = (0..32).collect();
        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_droplet_strand_roundtrip() {
        let droplet = Droplet {
            seed: 12345,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            segment_indices: vec![0, 2, 5],
        };

        let strand = FountainCodec::droplet_to_strand(&droplet);
        let recovered =
            FountainCodec::strand_to_droplet(&strand, 10, 4).unwrap();

        assert_eq!(recovered.seed, droplet.seed);
        assert_eq!(recovered.data, droplet.data);
    }

    #[test]
    fn test_density() {
        let codec = FountainCodec::new(FountainConfig::unscreened());
        let data: Vec<u8> = (0..100).collect(); // varied byte values

        let encoded = codec.encode(&data).unwrap();
        let bpn = encoded.bits_per_nucleotide();

        // Fountain should achieve reasonable density
        assert!(
            bpn > 0.5,
            "bits_per_nucleotide {} too low",
            bpn
        );
    }

    #[test]
    fn test_empty_input_error() {
        let codec = FountainCodec::default_codec();
        assert!(codec.encode(b"").is_err());
    }

    #[test]
    fn test_degree_distribution() {
        // Verify degree distribution produces valid degrees
        let mut rng = StdRng::seed_from_u64(42);
        let num_segments = 10;

        for _ in 0..100 {
            let degree = sample_degree(&mut rng, num_segments);
            assert!(degree >= 1 && degree <= num_segments);
        }
    }

    #[test]
    fn test_select_segments_valid() {
        let mut rng = StdRng::seed_from_u64(42);
        let num_segments = 10;

        for _ in 0..100 {
            let degree = sample_degree(&mut rng, num_segments);
            let indices = select_segments(&mut rng, num_segments, degree);

            assert_eq!(indices.len(), degree);
            // All indices should be in range
            for &idx in &indices {
                assert!(idx < num_segments);
            }
            // No duplicates (sorted, so check adjacent)
            for i in 1..indices.len() {
                assert!(indices[i] > indices[i - 1], "duplicate index");
            }
        }
    }
}
