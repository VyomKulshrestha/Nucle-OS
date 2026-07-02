//! # Reed-Solomon Error Correction for DNA Storage
//!
//! Implements Reed-Solomon codes over GF(256) as the **outer code** layer
//! for DNA storage. RS codes operate across strands, adding parity strands
//! that can recover from strand-level erasures (dropout) and corruptions.
//!
//! ## How it works
//!
//! 1. Organize data strands into blocks
//! 2. For each byte position across strands, compute RS parity symbols
//! 3. Store parity as additional strands
//! 4. On decode, use parity to recover missing/corrupted strands
//!
//! ## Capabilities
//!
//! With `t` parity strands per block:
//! - Can correct up to `t/2` corrupted strands
//! - Can recover up to `t` erased (known-missing) strands

use nucle_codec::base::DnaError;
use thiserror::Error;

/// Errors specific to Reed-Solomon encoding/decoding.
#[derive(Debug, Error)]
pub enum RsError {
    #[error("too many errors to correct: {errors} errors with {parity} parity symbols")]
    TooManyErrors { errors: usize, parity: usize },

    #[error("block size mismatch: expected {expected}, got {actual}")]
    BlockSizeMismatch { expected: usize, actual: usize },

    #[error("codec error: {0}")]
    CodecError(#[from] DnaError),
}

// ---------------------------------------------------------------------------
// GF(256) Arithmetic — the finite field underlying Reed-Solomon
// ---------------------------------------------------------------------------

/// Galois Field GF(256) arithmetic.
///
/// All RS operations happen in this field. The irreducible polynomial
/// is x^8 + x^4 + x^3 + x^2 + 1 (0x11D), which is standard for
/// RS codes used in QR codes, DVDs, and now DNA storage.
pub struct GF256;

impl GF256 {
    /// Irreducible polynomial for GF(256): x^8 + x^4 + x^3 + x^2 + 1
    const PRIMITIVE_POLY: u16 = 0x11D;

    /// Addition in GF(256) is XOR.
    pub fn add(a: u8, b: u8) -> u8 {
        a ^ b
    }

    /// Subtraction in GF(256) is also XOR (same as addition).
    pub fn sub(a: u8, b: u8) -> u8 {
        a ^ b
    }

    /// Multiplication in GF(256) using Russian Peasant algorithm.
    pub fn mul(a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            return 0;
        }
        let mut result: u16 = 0;
        let mut a_val = a as u16;
        let mut b_val = b as u16;

        for _ in 0..8 {
            if b_val & 1 != 0 {
                result ^= a_val;
            }
            let high_bit = a_val & 0x80;
            a_val <<= 1;
            if high_bit != 0 {
                a_val ^= Self::PRIMITIVE_POLY;
            }
            b_val >>= 1;
        }

        result as u8
    }

    /// Multiplicative inverse in GF(256) using extended Euclidean algorithm.
    /// Returns 0 for input 0 (undefined, but safe default).
    pub fn inv(a: u8) -> u8 {
        if a == 0 {
            return 0;
        }
        // a^254 = a^(-1) in GF(256) since a^255 = 1 for all nonzero a
        Self::pow(a, 254)
    }

    /// Division in GF(256): a / b = a * b^(-1).
    pub fn div(a: u8, b: u8) -> u8 {
        if b == 0 {
            panic!("division by zero in GF(256)");
        }
        Self::mul(a, Self::inv(b))
    }

    /// Exponentiation in GF(256) using square-and-multiply.
    pub fn pow(base: u8, exp: u32) -> u8 {
        if exp == 0 {
            return 1;
        }
        let mut result: u8 = 1;
        let mut b = base;
        let mut e = exp;
        while e > 0 {
            if e & 1 != 0 {
                result = Self::mul(result, b);
            }
            b = Self::mul(b, b);
            e >>= 1;
        }
        result
    }

    /// Evaluate a polynomial at point x in GF(256).
    /// Coefficients are [c0, c1, c2, ...] for c0 + c1*x + c2*x^2 + ...
    pub fn poly_eval(coeffs: &[u8], x: u8) -> u8 {
        let mut result: u8 = 0;
        let mut x_pow: u8 = 1;
        for &c in coeffs {
            result = Self::add(result, Self::mul(c, x_pow));
            x_pow = Self::mul(x_pow, x);
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Reed-Solomon Encoder
// ---------------------------------------------------------------------------

/// Configuration for the Reed-Solomon codec.
#[derive(Debug, Clone)]
pub struct RsConfig {
    /// Number of parity symbols (strands) per block.
    /// Higher = more redundancy = more error tolerance.
    pub parity_count: usize,
}

impl Default for RsConfig {
    fn default() -> Self {
        Self { parity_count: 4 }
    }
}

impl RsConfig {
    /// Create with specified parity count.
    pub fn new(parity_count: usize) -> Self {
        Self { parity_count }
    }

    /// Maximum erasures this configuration can recover.
    pub fn max_erasures(&self) -> usize {
        self.parity_count
    }

    /// Maximum errors (unknown positions) this can correct.
    pub fn max_errors(&self) -> usize {
        self.parity_count / 2
    }
}

/// GF(256) has only 256 distinct field elements, so a single Reed-Solomon
/// block can address at most this many codeword positions (data + parity
/// strands combined) before the `x`-coordinates used for interpolation wrap
/// around and collide. Beyond this size, `encode_block`/`decode_block` split
/// the input into independent stripes, each within this limit.
const MAX_BLOCK_SYMBOLS: usize = 256;

/// Reed-Solomon encoder/decoder for DNA strand-level error correction.
pub struct ReedSolomon {
    config: RsConfig,
}

impl ReedSolomon {
    /// Create a new RS codec with the given configuration.
    pub fn new(config: RsConfig) -> Self {
        Self { config }
    }

    /// Create with default settings (4 parity symbols).
    pub fn default_codec() -> Self {
        Self::new(RsConfig::default())
    }

    /// Access configuration.
    pub fn config(&self) -> &RsConfig {
        &self.config
    }

    /// Maximum number of data strands a single stripe can hold for this
    /// codec's parity count, while staying within GF(256)'s 256 positions.
    fn max_data_per_stripe(&self) -> usize {
        MAX_BLOCK_SYMBOLS.saturating_sub(self.config.parity_count).max(1)
    }

    /// Encode a block of data strands, producing parity strands.
    ///
    /// Model: treat each byte position as an independent GF(256) polynomial.
    /// Data strands are evaluations of P(x) at x = 0, 1, ..., k-1.
    /// Parity strands are evaluations of P(x) at x = k, k+1, ..., k+n-1.
    ///
    /// Inputs larger than [`MAX_BLOCK_SYMBOLS`] minus the parity count are
    /// split into independent stripes, each producing its own full set of
    /// parity strands — otherwise `x`-coordinates beyond 255 would wrap
    /// around a `u8` and collide with earlier ones.
    pub fn encode_block(&self, data_strands: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, RsError> {
        if data_strands.is_empty() {
            return Ok(Vec::new());
        }

        let max_per_stripe = self.max_data_per_stripe();
        if data_strands.len() <= max_per_stripe {
            return self.encode_stripe(data_strands);
        }

        let mut all_parity = Vec::new();
        for chunk in data_strands.chunks(max_per_stripe) {
            all_parity.extend(self.encode_stripe(chunk)?);
        }
        Ok(all_parity)
    }

    fn encode_stripe(&self, data_strands: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, RsError> {
        let strand_len = data_strands[0].len();
        let k = data_strands.len();
        let n = self.config.parity_count;

        let mut parity_strands: Vec<Vec<u8>> = vec![vec![0u8; strand_len]; n];

        for pos in 0..strand_len {
            let points: Vec<(u8, u8)> = (0..k)
                .map(|i| {
                    let val = if pos < data_strands[i].len() { data_strands[i][pos] } else { 0 };
                    (i as u8, val)
                })
                .collect();

            for j in 0..n {
                let x_target = (k + j) as u8;
                parity_strands[j][pos] = Self::lagrange_eval(&points, x_target);
            }
        }

        Ok(parity_strands)
    }

    /// Decode a block, recovering missing strands from parity.
    ///
    /// Mirrors the striping `encode_block` performs: `received` is split
    /// into the same-sized stripes, `parity` into groups of this codec's
    /// parity count per stripe, and each stripe is decoded independently.
    pub fn decode_block(
        &self,
        received: &[Option<Vec<u8>>],
        parity: &[Vec<u8>],
    ) -> Result<Vec<Vec<u8>>, RsError> {
        let n = self.config.parity_count;

        if n == 0 {
            return received.iter().enumerate().map(|(i, s)| {
                s.clone().ok_or(RsError::TooManyErrors { errors: i + 1, parity: 0 })
            }).collect();
        }

        let max_per_stripe = self.max_data_per_stripe();
        if received.len() <= max_per_stripe {
            return self.decode_stripe(received, parity);
        }

        let mut result = Vec::with_capacity(received.len());
        for (data_chunk, parity_chunk) in received.chunks(max_per_stripe).zip(parity.chunks(n)) {
            result.extend(self.decode_stripe(data_chunk, parity_chunk)?);
        }
        Ok(result)
    }

    fn decode_stripe(
        &self,
        received: &[Option<Vec<u8>>],
        parity: &[Vec<u8>],
    ) -> Result<Vec<Vec<u8>>, RsError> {
        let k = received.len();
        let n = parity.len();

        let erased: Vec<usize> = received.iter()
            .enumerate()
            .filter(|(_, s)| s.is_none())
            .map(|(i, _)| i)
            .collect();

        if erased.len() > n {
            return Err(RsError::TooManyErrors {
                errors: erased.len(),
                parity: n,
            });
        }

        if erased.is_empty() {
            return Ok(received.iter().map(|s| s.clone().unwrap()).collect());
        }

        let strand_len = received.iter()
            .filter_map(|s| s.as_ref())
            .map(|s| s.len())
            .next()
            .unwrap_or_else(|| parity.first().map(|p| p.len()).unwrap_or(0));

        let mut result: Vec<Vec<u8>> = received.iter()
            .map(|s| s.clone().unwrap_or_else(|| vec![0u8; strand_len]))
            .collect();

        for pos in 0..strand_len {
            let mut known_points: Vec<(u8, u8)> = Vec::new();

            for (i, strand_opt) in received.iter().enumerate() {
                if let Some(strand) = strand_opt {
                    let val = if pos < strand.len() { strand[pos] } else { 0 };
                    known_points.push((i as u8, val));
                }
            }

            for (j, parity_strand) in parity.iter().enumerate() {
                let val = if pos < parity_strand.len() { parity_strand[pos] } else { 0 };
                known_points.push(((k + j) as u8, val));
            }

            if known_points.len() < k {
                return Err(RsError::TooManyErrors {
                    errors: erased.len(),
                    parity: n,
                });
            }

            let points = &known_points[..k];

            for &erased_idx in &erased {
                result[erased_idx][pos] = Self::lagrange_eval(points, erased_idx as u8);
            }
        }

        Ok(result)
    }

    /// Lagrange interpolation: evaluate the polynomial through `points` at `x_target`.
    fn lagrange_eval(points: &[(u8, u8)], x_target: u8) -> u8 {
        let mut value: u8 = 0;

        for (j, &(x_j, y_j)) in points.iter().enumerate() {
            let mut basis: u8 = 1;
            for (m, &(x_m, _)) in points.iter().enumerate() {
                if m != j {
                    let num = GF256::sub(x_target, x_m);
                    let den = GF256::sub(x_j, x_m);
                    basis = GF256::mul(basis, GF256::div(num, den));
                }
            }
            value = GF256::add(value, GF256::mul(y_j, basis));
        }

        value
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gf256_add_sub() {
        assert_eq!(GF256::add(0, 0), 0);
        assert_eq!(GF256::add(0xFF, 0xFF), 0); // a + a = 0 in GF(256)
        assert_eq!(GF256::add(0xAB, 0), 0xAB);
        assert_eq!(GF256::sub(0xAB, 0xAB), 0);
    }

    #[test]
    fn test_gf256_mul() {
        assert_eq!(GF256::mul(0, 42), 0);
        assert_eq!(GF256::mul(1, 42), 42);
        assert_eq!(GF256::mul(42, 1), 42);
    }

    #[test]
    fn test_gf256_inverse() {
        // a * a^(-1) = 1 for all nonzero a
        for a in 1..=255u8 {
            let inv = GF256::inv(a);
            assert_eq!(
                GF256::mul(a, inv), 1,
                "inverse failed for a={}: inv={}, product={}",
                a, inv, GF256::mul(a, inv)
            );
        }
    }

    #[test]
    fn test_gf256_div() {
        for a in 1..=255u8 {
            assert_eq!(GF256::div(a, a), 1, "a/a should be 1 for a={}", a);
        }
        assert_eq!(GF256::div(0, 42), 0); // 0/x = 0
    }

    #[test]
    fn test_rs_encode_no_erasures() {
        let rs = ReedSolomon::new(RsConfig::new(2));
        let data = vec![
            vec![1, 2, 3, 4],
            vec![5, 6, 7, 8],
            vec![9, 10, 11, 12],
        ];

        let parity = rs.encode_block(&data).unwrap();
        assert_eq!(parity.len(), 2); // 2 parity strands
        assert_eq!(parity[0].len(), 4); // Same length as data strands

        // Decode with no erasures should return original data
        let received: Vec<Option<Vec<u8>>> = data.iter()
            .map(|s| Some(s.clone()))
            .collect();
        let decoded = rs.decode_block(&received, &parity).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_rs_recover_one_erasure() {
        let rs = ReedSolomon::new(RsConfig::new(2));
        let data = vec![
            vec![10, 20, 30],
            vec![40, 50, 60],
            vec![70, 80, 90],
        ];

        let parity = rs.encode_block(&data).unwrap();

        // Erase strand 1
        let received = vec![
            Some(data[0].clone()),
            None, // Erased!
            Some(data[2].clone()),
        ];

        let decoded = rs.decode_block(&received, &parity).unwrap();
        assert_eq!(decoded[1], data[1], "failed to recover erased strand");
    }

    #[test]
    fn test_rs_recover_two_erasures() {
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data = vec![
            vec![1, 2],
            vec![3, 4],
            vec![5, 6],
            vec![7, 8],
            vec![9, 10],
        ];

        let parity = rs.encode_block(&data).unwrap();

        // Erase strands 0 and 3
        let received = vec![
            None,
            Some(data[1].clone()),
            Some(data[2].clone()),
            None,
            Some(data[4].clone()),
        ];

        let decoded = rs.decode_block(&received, &parity).unwrap();
        assert_eq!(decoded[0], data[0], "failed to recover strand 0");
        assert_eq!(decoded[3], data[3], "failed to recover strand 3");
    }

    #[test]
    fn test_rs_encode_large_block_does_not_panic() {
        // 300 data strands with 4 parity would put x-coordinates at 300..303,
        // which overflow a u8 and previously collided with earlier indices,
        // causing a division-by-zero panic. Striping must avoid that.
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data: Vec<Vec<u8>> = (0..300u32).map(|i| vec![(i % 256) as u8, ((i * 7) % 256) as u8]).collect();

        let parity = rs.encode_block(&data).unwrap();
        // Two stripes of <=252 data strands each => 2 * 4 parity strands.
        assert_eq!(parity.len(), 8);

        let received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        let decoded = rs.decode_block(&received, &parity).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_rs_recover_erasures_across_striped_block() {
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data: Vec<Vec<u8>> = (0..300u32).map(|i| vec![(i % 256) as u8]).collect();
        let parity = rs.encode_block(&data).unwrap();

        // Erase one strand in the first stripe and one in the second.
        let mut received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        received[10] = None;
        received[260] = None;

        let decoded = rs.decode_block(&received, &parity).unwrap();
        assert_eq!(decoded[10], data[10]);
        assert_eq!(decoded[260], data[260]);
    }

    #[test]
    fn test_rs_zero_parity_passthrough() {
        let rs = ReedSolomon::new(RsConfig::new(0));
        let data = vec![vec![1, 2], vec![3, 4]];
        let received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        let decoded = rs.decode_block(&received, &[]).unwrap();
        assert_eq!(decoded, data);

        let received_with_gap: Vec<Option<Vec<u8>>> = vec![Some(data[0].clone()), None];
        assert!(rs.decode_block(&received_with_gap, &[]).is_err());
    }

    #[test]
    fn test_rs_too_many_erasures() {
        let rs = ReedSolomon::new(RsConfig::new(1));
        let data = vec![vec![1], vec![2], vec![3]];
        let parity = rs.encode_block(&data).unwrap();

        // Erase 2 strands but only 1 parity
        let received = vec![None, None, Some(data[2].clone())];
        let result = rs.decode_block(&received, &parity);
        assert!(result.is_err());
    }

    #[test]
    fn test_rs_config() {
        let config = RsConfig::new(6);
        assert_eq!(config.max_erasures(), 6);
        assert_eq!(config.max_errors(), 3);
    }
}
