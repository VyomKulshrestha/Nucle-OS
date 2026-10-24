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
//! - **Rateless**: Can generate deterministic continuation batches on demand
//! - **Near-optimal density**: ~1.57 bits/nucleotide
//! - **Erasure resilient**: Any full-rank surviving equation set reconstructs
//! - **Constraint-aware**: Rejects strands violating biological constraints
//!
//! ## Decoding
//!
//! Uses belief propagation (peeling decoder) with GF(2) elimination fallback:
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
use crate::stable_rng::StableRng;
use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the DNA Fountain codec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
            max_screening_attempts: 10_000,
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
fn sample_degree(rng: &mut StableRng, num_segments: usize) -> usize {
    // Simplified robust soliton: mix ideal soliton with a spike at 1
    let k = num_segments as f64;
    let r = rng.gen_f64();

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
fn select_segments(rng: &mut StableRng, num_segments: usize, degree: usize) -> Vec<usize> {
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

/// A deterministic continuation of a Fountain stream.
///
/// Call [`FountainCodec::encode_batch`] again with `next_seed` to create more
/// independent droplets without changing the data or codec configuration.
#[derive(Debug, Clone)]
pub struct FountainBatch {
    pub strands: StrandCollection,
    pub next_seed: u64,
}

impl FountainCodec {
    const WHITENING_KEY: u64 = 0xD1B5_4A32_D192_ED03;

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

    fn feistel_round(value: u32, round: u32) -> u32 {
        value
            .wrapping_add(0x9E37_79B9u32.wrapping_mul(round + 1))
            .rotate_left(7 + round)
            .wrapping_mul(0x85EB_CA6B)
    }

    /// Reversible scrambling prevents small sequential droplet seeds from
    /// becoming long A-rich prefixes under the 2-bit DNA mapping.
    fn permute_seed(seed: u64) -> u64 {
        let mut left = (seed >> 32) as u32;
        let mut right = seed as u32;
        for round in 0..6 {
            let next = left ^ Self::feistel_round(right, round);
            left = right;
            right = next;
        }
        ((left as u64) << 32) | right as u64
    }

    fn unpermute_seed(encoded: u64) -> u64 {
        let mut left = (encoded >> 32) as u32;
        let mut right = encoded as u32;
        for round in (0..6).rev() {
            let previous_right = left;
            let previous_left = right ^ Self::feistel_round(left, round);
            left = previous_left;
            right = previous_right;
        }
        ((left as u64) << 32) | right as u64
    }

    fn whiten_payload(seed: u64, data: &[u8]) -> Vec<u8> {
        let mut rng = StableRng::seed_from_u64(seed ^ Self::WHITENING_KEY);
        data.iter().map(|byte| byte ^ rng.gen_u8()).collect()
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
        if !nucs.len().is_multiple_of(4) {
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

        // Scramble the seed and payload before the 2-bit mapping. This is
        // reversible, but avoids deterministic A-rich headers and lets the
        // biological screen work for arbitrary binary input.
        let seed_bytes = Self::permute_seed(droplet.seed).to_be_bytes();
        nucleotides.extend(Self::bytes_to_nucleotides(&seed_bytes));

        let whitened = Self::whiten_payload(droplet.seed, &droplet.data);
        nucleotides.extend(Self::bytes_to_nucleotides(&whitened));

        DnaStrand::new(nucleotides)
    }

    /// Decode a DNA strand back to a droplet.
    fn strand_to_droplet(
        strand: &DnaStrand,
        num_segments: usize,
        segment_size: usize,
        legacy_raw: bool,
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
        let encoded_seed = u64::from_be_bytes([
            seed_bytes[0],
            seed_bytes[1],
            seed_bytes[2],
            seed_bytes[3],
            seed_bytes[4],
            seed_bytes[5],
            seed_bytes[6],
            seed_bytes[7],
        ]);
        let seed = if legacy_raw {
            encoded_seed
        } else {
            Self::unpermute_seed(encoded_seed)
        };

        // Payload is everything after the seed
        let payload_nucs = &bases[32..];
        let encoded_data = Self::nucleotides_to_bytes(payload_nucs)?;

        // A noisy channel (deletion/truncation) can shorten the payload
        // below the expected segment size — that's a corrupted strand,
        // not a bug, so report it as a decoding error instead of panicking.
        if encoded_data.len() < segment_size {
            return Err(DnaError::DecodingError(format!(
                "strand payload too short: expected {} bytes, got {}",
                segment_size,
                encoded_data.len()
            )));
        }
        let data = if legacy_raw {
            encoded_data
        } else {
            Self::whiten_payload(seed, &encoded_data)
        };

        // Reconstruct the segment indices using the same PRNG
        let mut rng = StableRng::seed_from_u64(seed);
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
        droplets: &mut [Droplet],
        num_segments: usize,
        segment_size: usize,
    ) -> Result<Vec<Vec<u8>>, DnaError> {
        let mut solved: Vec<Option<Vec<u8>>> = vec![None; num_segments];
        let mut solved_count = 0;

        // Track which segments each droplet still references
        let droplet_segments: Vec<Vec<usize>> = droplets
            .iter()
            .map(|d| d.segment_indices.clone())
            .collect();
        let droplet_data: Vec<Vec<u8>> = droplets
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

    /// Solve the droplet equations over GF(2). This recovers every full-rank
    /// surviving set, including sets where the peeling decoder has no
    /// degree-one equation and stalls.
    fn gaussian_decode(
        droplets: &[Droplet],
        num_segments: usize,
        segment_size: usize,
    ) -> Result<Vec<Vec<u8>>, DnaError> {
        let mut rows: Vec<(Vec<bool>, Vec<u8>)> = droplets
            .iter()
            .map(|droplet| {
                let mut coefficients = vec![false; num_segments];
                for &index in &droplet.segment_indices {
                    coefficients[index] = true;
                }
                (coefficients, droplet.data[..segment_size].to_vec())
            })
            .collect();
        let mut pivot_rows = vec![None; num_segments];
        let mut next_row = 0;

        for (column, pivot_row) in pivot_rows.iter_mut().enumerate() {
            let Some(pivot) = (next_row..rows.len()).find(|&row| rows[row].0[column]) else {
                continue;
            };
            rows.swap(next_row, pivot);

            for row in 0..rows.len() {
                if row == next_row || !rows[row].0[column] {
                    continue;
                }
                let pivot_coefficients = rows[next_row].0.clone();
                let pivot_data = rows[next_row].1.clone();
                for (coefficient, pivot_coefficient) in rows[row].0[column..]
                    .iter_mut()
                    .zip(&pivot_coefficients[column..])
                {
                    *coefficient ^= *pivot_coefficient;
                }
                for (value, pivot_value) in rows[row].1.iter_mut().zip(&pivot_data) {
                    *value ^= *pivot_value;
                }
            }

            *pivot_row = Some(next_row);
            next_row += 1;
        }

        if pivot_rows.iter().any(Option::is_none) {
            return Err(DnaError::DecodingError(format!(
                "fountain equations are rank deficient: rank {}/{}; generate or retain more droplets",
                next_row, num_segments
            )));
        }

        Ok(pivot_rows
            .into_iter()
            .map(|row| rows[row.unwrap()].1.clone())
            .collect())
    }

    fn decode_droplets(
        &self,
        strands: &StrandCollection,
        num_segments: usize,
        segment_size: usize,
        legacy_raw: bool,
    ) -> Result<Vec<u8>, DnaError> {
        let droplets = strands
            .strands
            .iter()
            .map(|strand| Self::strand_to_droplet(strand, num_segments, segment_size, legacy_raw))
            .collect::<Result<Vec<_>, _>>()?;
        let segments = Self::peeling_decode(&mut droplets.clone(), num_segments, segment_size)
            .or_else(|_| Self::gaussian_decode(&droplets, num_segments, segment_size))?;
        let all_data: Vec<u8> = segments.into_iter().flatten().collect();
        if all_data.len() < 4 {
            return Err(DnaError::DecodingError("decoded data has no length header".into()));
        }
        let original_len = u32::from_be_bytes(all_data[..4].try_into().unwrap()) as usize;
        let data_end = 4usize.saturating_add(original_len);
        if data_end > all_data.len() {
            return Err(DnaError::DecodingError("decoded length header exceeds payload".into()));
        }
        Ok(all_data[4..data_end].to_vec())
    }

    /// Generate an exact screened batch beginning at `start_seed`.
    ///
    /// Persist `next_seed` and pass it to a later call to extend the same
    /// rateless stream. Failure is atomic: this never reports a partial batch
    /// as a successful encoding.
    pub fn encode_batch(
        &self,
        data: &[u8],
        start_seed: u64,
        droplet_count: usize,
    ) -> Result<FountainBatch, DnaError> {
        if data.is_empty() {
            return Err(DnaError::EncodingError("empty input data".into()));
        }
        if droplet_count == 0 {
            return Err(DnaError::EncodingError("droplet count must be positive".into()));
        }

        let mut payload = Vec::with_capacity(4 + data.len());
        payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
        payload.extend_from_slice(data);
        let segments = Self::segment_data(&payload, self.config.segment_size);
        let validator = self.config.screen_constraints.then(|| {
            ConstraintValidator::new(self.config.constraint_config.clone())
        });
        let mut strands = StrandCollection::new(data.len());
        let mut seed = start_seed;
        let max_attempts = droplet_count.saturating_mul(self.config.max_screening_attempts);

        for _ in 0..max_attempts {
            let droplet_seed = seed;
            seed = seed.wrapping_add(1);
            let mut rng = StableRng::seed_from_u64(droplet_seed);
            let degree = sample_degree(&mut rng, segments.len());
            let indices = select_segments(&mut rng, segments.len(), degree);
            let droplet = Droplet {
                seed: droplet_seed,
                data: Self::xor_segments(&segments, &indices),
                segment_indices: indices,
            };
            let strand = Self::droplet_to_strand(&droplet);
            if validator.as_ref().is_some_and(|item| !item.is_valid(&strand)) {
                continue;
            }
            strands.push(strand);
            if strands.strand_count() == droplet_count {
                return Ok(FountainBatch { strands, next_seed: seed });
            }
        }

        Err(DnaError::EncodingError(format!(
            "constraint screening produced only {}/{} droplets after {} attempts",
            strands.strand_count(), droplet_count, max_attempts
        )))
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
        let num_segments = (data.len() + 4).div_ceil(self.config.segment_size);
        let num_droplets =
            (num_segments as f64 * self.config.overhead).ceil() as usize;
        let num_droplets = num_droplets.max(num_segments + 1); // Need at least k+1
        Ok(self.encode_batch(data, self.config.seed, num_droplets)?.strands)
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

        if strands.original_size > 0 {
            let num_segments = (strands.original_size + 4).div_ceil(self.config.segment_size);
            for legacy_raw in [false, true] {
                if let Ok(decoded) = self.decode_droplets(
                    strands,
                    num_segments,
                    segment_size,
                    legacy_raw,
                ) {
                    return Ok(decoded);
                }
            }
        }

        // Try different num_segments values
        // Start with estimate from strand count / overhead
        let estimated_segments = if strands.original_size > 0 {
            (strands.original_size + 4).div_ceil(self.config.segment_size)
        } else {
            (strands.strand_count() as f64 / self.config.overhead).ceil() as usize
        };

        // Try a range around the estimate
        for num_seg_try in 1..=estimated_segments.max(strands.strand_count()) {
            let mut droplets = Vec::new();
            let mut decode_ok = true;

            for strand in &strands.strands {
                match Self::strand_to_droplet(strand, num_seg_try, segment_size, true) {
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

    /// FNV-1a over `bytes`, folded into `hash`.
    fn fnv(hash: u64, bytes: &[u8]) -> u64 {
        bytes.iter().fold(hash, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    }
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

    fn strands_digest(strands: &[DnaStrand]) -> u64 {
        strands.iter().fold(FNV_OFFSET, |h, s| {
            fnv(fnv(h, &s.bases().iter().map(|n| n.to_bits()).collect::<Vec<u8>>()), b"|")
        })
    }

    /// The droplets a seed makes are rebuilt from it when a file is read, so
    /// they must never change: these digests were recorded from the rand 0.8
    /// encoder that wrote the archives in existing pools.
    #[test]
    fn encoding_is_pinned_to_what_existing_archives_hold() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
        let codec = FountainCodec::new(FountainConfig::default());
        let strands = codec.encode(&data).unwrap().strands;
        assert_eq!(strands.len(), 95);
        assert_eq!(strands_digest(&strands), 0x163c_5da2_e465_3aed);
        let batch = codec.encode_batch(&data, 7, 40).unwrap();
        assert_eq!(batch.next_seed, 18_226);
        assert_eq!(strands_digest(&batch.strands.strands), 0xe644_45b6_e944_43c3);
    }

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
            FountainCodec::strand_to_droplet(&strand, 10, 4, false).unwrap();

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
    fn test_screened_roundtrip_accepts_arbitrary_binary() {
        let codec = FountainCodec::default_codec();
        let data: Vec<u8> = (0..=255).collect();
        let encoded = codec.encode(&data).unwrap();
        let validator = ConstraintValidator::default_validator();

        assert!(encoded.strands.iter().all(|strand| validator.is_valid(strand)));
        assert_eq!(codec.decode(&encoded).unwrap(), data);
    }

    #[test]
    fn test_rateless_batches_continue_without_repeating_seeds() {
        let codec = FountainCodec::default_codec();
        let data = vec![0xA5; 128];
        let first = codec.encode_batch(&data, codec.config.seed, 12).unwrap();
        let second = codec.encode_batch(&data, first.next_seed, 12).unwrap();
        let first_seeds: Vec<u64> = first
            .strands
            .strands
            .iter()
            .map(|strand| FountainCodec::strand_to_droplet(strand, 9, 16, false).unwrap().seed)
            .collect();
        let second_seeds: Vec<u64> = second
            .strands
            .strands
            .iter()
            .map(|strand| FountainCodec::strand_to_droplet(strand, 9, 16, false).unwrap().seed)
            .collect();
        assert!(first_seeds.iter().all(|seed| !second_seeds.contains(seed)));

        let mut combined = first.strands;
        combined.strands.extend(second.strands.strands);
        assert_eq!(codec.decode(&combined).unwrap(), data);
    }

    #[test]
    fn test_full_rank_subset_recovers_after_droplet_loss() {
        let codec = FountainCodec::new(FountainConfig::high_resilience());
        let data: Vec<u8> = (0..192).map(|value| (value * 37) as u8).collect();
        let mut encoded = codec.encode(&data).unwrap();
        encoded.strands = encoded
            .strands
            .into_iter()
            .enumerate()
            .filter_map(|(index, strand)| (index % 5 != 0).then_some(strand))
            .collect();

        assert_eq!(codec.decode(&encoded).unwrap(), data);
    }

    #[test]
    fn test_legacy_raw_fountain_strands_still_decode() {
        let config = FountainConfig {
            segment_size: 8,
            overhead: 3.0,
            ..FountainConfig::unscreened()
        };
        let codec = FountainCodec::new(config.clone());
        let data = b"legacy fountain representation".to_vec();
        let mut payload = (data.len() as u32).to_be_bytes().to_vec();
        payload.extend_from_slice(&data);
        let segments = FountainCodec::segment_data(&payload, config.segment_size);
        let mut collection = StrandCollection::new(data.len());

        for seed in config.seed..config.seed + (segments.len() * 3) as u64 {
            let mut rng = StableRng::seed_from_u64(seed);
            let degree = sample_degree(&mut rng, segments.len());
            let indices = select_segments(&mut rng, segments.len(), degree);
            let mut bases = FountainCodec::bytes_to_nucleotides(&seed.to_be_bytes());
            bases.extend(FountainCodec::bytes_to_nucleotides(
                &FountainCodec::xor_segments(&segments, &indices),
            ));
            collection.push(DnaStrand::new(bases));
        }

        assert_eq!(codec.decode(&collection).unwrap(), data);
    }

    #[test]
    fn test_empty_input_error() {
        let codec = FountainCodec::default_codec();
        assert!(codec.encode(b"").is_err());
    }

    #[test]
    fn test_degree_distribution() {
        // Verify degree distribution produces valid degrees
        let mut rng = StableRng::seed_from_u64(42);
        let num_segments = 10;

        for _ in 0..100 {
            let degree = sample_degree(&mut rng, num_segments);
            assert!(degree >= 1 && degree <= num_segments);
        }
    }

    #[test]
    fn test_select_segments_valid() {
        let mut rng = StableRng::seed_from_u64(42);
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
