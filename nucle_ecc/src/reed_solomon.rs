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
//! With `t` parity strands per block, decoding is combined
//! error-and-erasure Berlekamp-Welch (see `decode_stripe`):
//! - Can correct up to `t/2` corrupted-but-present strands, blindly --
//!   the caller never has to say which ones are wrong
//! - Can recover up to `t` erased (known-missing) strands
//! - The two combine: `2 * errors + erasures <= t`

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

    /// Divide polynomial `dividend` by `divisor` (both little-endian
    /// coefficient lists, i.e. index 0 is the constant term), returning
    /// `(quotient, remainder)`.
    pub fn poly_divmod(dividend: &[u8], divisor: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut divisor_deg = divisor.len();
        while divisor_deg > 0 && divisor[divisor_deg - 1] == 0 {
            divisor_deg -= 1;
        }
        assert!(divisor_deg > 0, "division by zero polynomial");
        let lead_inv = Self::inv(divisor[divisor_deg - 1]);

        let mut rem = dividend.to_vec();
        let mut rem_deg = rem.len();
        while rem_deg > 0 && rem[rem_deg - 1] == 0 {
            rem_deg -= 1;
        }

        if rem_deg < divisor_deg {
            return (vec![0], rem);
        }

        let quot_deg = rem_deg - divisor_deg;
        let mut quotient = vec![0u8; quot_deg + 1];

        for shift in (0..=quot_deg).rev() {
            let cur_top = shift + divisor_deg - 1;
            if cur_top >= rem.len() {
                continue;
            }
            let coeff = Self::mul(rem[cur_top], lead_inv);
            quotient[shift] = coeff;
            if coeff != 0 {
                for i in 0..divisor_deg {
                    rem[shift + i] = Self::sub(rem[shift + i], Self::mul(coeff, divisor[i]));
                }
            }
        }

        (quotient, rem)
    }

    /// Solve the linear system `matrix * x = rhs` over GF(256) via
    /// Gauss-Jordan elimination with partial pivoting. `matrix` is `dim`
    /// square rows; returns `None` if the system is singular.
    pub fn solve_linear_system(mut matrix: Vec<Vec<u8>>, mut rhs: Vec<u8>) -> Option<Vec<u8>> {
        let dim = rhs.len();
        for col in 0..dim {
            let pivot = (col..dim).find(|&r| matrix[r][col] != 0)?;
            matrix.swap(col, pivot);
            rhs.swap(col, pivot);

            let inv = Self::inv(matrix[col][col]);
            for c in col..dim {
                matrix[col][c] = Self::mul(matrix[col][c], inv);
            }
            rhs[col] = Self::mul(rhs[col], inv);

            for r in 0..dim {
                if r != col && matrix[r][col] != 0 {
                    let factor = matrix[r][col];
                    for c in col..dim {
                        matrix[r][c] = Self::sub(matrix[r][c], Self::mul(factor, matrix[col][c]));
                    }
                    rhs[r] = Self::sub(rhs[r], Self::mul(factor, rhs[col]));
                }
            }
        }
        Some(rhs)
    }
}

// ---------------------------------------------------------------------------
// Reed-Solomon Encoder
// ---------------------------------------------------------------------------

/// Configuration for the Reed-Solomon codec.
#[derive(Debug, Clone, Copy)]
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

    /// Decode a block, recovering missing strands from parity and
    /// correcting strands that arrived but are silently wrong.
    ///
    /// `parity` is `Option`-per-slot (not a dense, possibly-shorter list)
    /// so that a parity strand which failed to arrive keeps its true
    /// codeword position `k + j` -- collapsing it out of the list would
    /// shift every later parity strand onto the wrong evaluation point
    /// and corrupt the whole stripe's math, independent of how many
    /// strands are actually wrong.
    ///
    /// Mirrors the striping `encode_block` performs: `received` is split
    /// into the same-sized stripes, `parity` into groups of this codec's
    /// parity count per stripe, and each stripe is decoded independently.
    pub fn decode_block(
        &self,
        received: &[Option<Vec<u8>>],
        parity: &[Option<Vec<u8>>],
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

    /// Decode one stripe using combined error-and-erasure Reed-Solomon
    /// decoding (Berlekamp-Welch): strands marked `None` are known
    /// erasures at their true codeword position; strands that are
    /// `Some` but silently wrong are corrected blindly (their position
    /// is never known in advance) as long as `2*errors + erasures <=
    /// parity_count` -- this is what lets a strand whose consensus vote
    /// landed on a confident-but-wrong answer get fixed automatically,
    /// without a caller having to guess which strand that was.
    fn decode_stripe(
        &self,
        received: &[Option<Vec<u8>>],
        parity: &[Option<Vec<u8>>],
    ) -> Result<Vec<Vec<u8>>, RsError> {
        let k = received.len();
        let n = self.config.parity_count;

        let erasures = received.iter().filter(|s| s.is_none()).count()
            + parity.iter().filter(|s| s.is_none()).count();
        if erasures > n {
            return Err(RsError::TooManyErrors { errors: erasures, parity: n });
        }

        let strand_len = received.iter().flatten()
            .chain(parity.iter().flatten())
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        if k == 0 {
            return Ok(Vec::new());
        }

        let mut result = vec![vec![0u8; strand_len]; k];

        for pos in 0..strand_len {
            let mut avail: Vec<(u8, u8)> = Vec::new();
            for (i, strand_opt) in received.iter().enumerate() {
                if let Some(strand) = strand_opt {
                    let val = if pos < strand.len() { strand[pos] } else { 0 };
                    avail.push((i as u8, val));
                }
            }
            for (j, parity_opt) in parity.iter().enumerate() {
                if let Some(strand) = parity_opt {
                    let val = if pos < strand.len() { strand[pos] } else { 0 };
                    avail.push(((k + j) as u8, val));
                }
            }

            if avail.len() < k {
                return Err(RsError::TooManyErrors { errors: erasures, parity: n });
            }

            let e_max = (avail.len() - k) / 2;
            let p_coeffs = (0..=e_max).rev()
                .find_map(|e| Self::try_welch_decode(&avail, k, e))
                .ok_or(RsError::TooManyErrors { errors: erasures.max(e_max), parity: n })?;

            for i in 0..k {
                result[i][pos] = GF256::poly_eval(&p_coeffs, i as u8);
            }
        }

        Ok(result)
    }

    /// Attempt Berlekamp-Welch decoding of `avail` points assuming
    /// exactly `e` of them are wrong, recovering the degree-`<k`
    /// polynomial `P` whose coefficients are the true data/parity
    /// symbols. Returns `None` if the assumption doesn't hold (system is
    /// singular, or the reconstructed error locator doesn't explain
    /// every point) -- the caller tries progressively smaller `e`.
    ///
    /// Finds `Q` (degree `< e+k`) and monic `E` (degree `e`) satisfying
    /// `Q(x_i) = E(x_i) * y_i` for every available point; `P = Q / E`
    /// when `E`'s roots are exactly the error locations. `E(x) = 1` (e=0)
    /// degenerates to plain consistency-checked interpolation.
    fn try_welch_decode(avail: &[(u8, u8)], k: usize, e: usize) -> Option<Vec<u8>> {
        let need = 2 * e + k;
        if avail.len() < need {
            return None;
        }

        // Unknowns: q_0..q_{e+k-1} (Q's coefficients), then e_0..e_{e-1}
        // (E's coefficients below its implicit monic leading term).
        let dim = need;
        let mut matrix = vec![vec![0u8; dim]; dim];
        let mut rhs = vec![0u8; dim];

        for row in 0..dim {
            let (x, y) = avail[row];
            let mut x_pow = 1u8;
            for j in 0..(e + k) {
                matrix[row][j] = x_pow;
                x_pow = GF256::mul(x_pow, x);
            }
            let mut x_pow = 1u8;
            for l in 0..e {
                // Equation: sum q_j x^j + y * sum e_l x^l = y * x^e
                // (GF(256) subtraction is addition, so no sign flip needed.)
                matrix[row][e + k + l] = GF256::mul(y, x_pow);
                x_pow = GF256::mul(x_pow, x);
            }
            rhs[row] = GF256::mul(y, x_pow);
        }

        let solution = GF256::solve_linear_system(matrix, rhs)?;
        let q_coeffs = solution[0..(e + k)].to_vec();
        let mut e_coeffs = solution[(e + k)..(2 * e + k)].to_vec();
        e_coeffs.push(1); // monic leading term

        for &(x, y) in avail {
            let qx = GF256::poly_eval(&q_coeffs, x);
            let ex = GF256::poly_eval(&e_coeffs, x);
            if qx != GF256::mul(y, ex) {
                return None;
            }
        }

        let (p_coeffs, remainder) = GF256::poly_divmod(&q_coeffs, &e_coeffs);
        if remainder.iter().any(|&c| c != 0) || p_coeffs.len() > k {
            return None;
        }

        let mut p_coeffs = p_coeffs;
        p_coeffs.resize(k, 0);
        Some(p_coeffs)
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
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();
        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
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
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        // Erase strand 1
        let received = vec![
            Some(data[0].clone()),
            None, // Erased!
            Some(data[2].clone()),
        ];

        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
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
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        // Erase strands 0 and 3
        let received = vec![
            None,
            Some(data[1].clone()),
            Some(data[2].clone()),
            None,
            Some(data[4].clone()),
        ];

        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
        assert_eq!(decoded[0], data[0], "failed to recover strand 0");
        assert_eq!(decoded[3], data[3], "failed to recover strand 3");
    }

    #[test]
    fn test_rs_corrects_silent_error_without_knowing_position() {
        // No erasures at all -- one received strand is simply wrong (as if
        // consensus voted confidently on the wrong answer). With 4 parity
        // strands, max_errors() = 2, so a single blind error must be
        // correctable without the caller ever marking it as missing.
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data = vec![
            vec![10, 20, 30],
            vec![40, 50, 60],
            vec![70, 80, 90],
            vec![15, 25, 35],
            vec![99, 1, 2],
        ];
        let parity = rs.encode_block(&data).unwrap();
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        let mut received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        // Corrupt strand 2 without marking it as erased.
        received[2] = Some(vec![71, 80, 90]);

        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
        assert_eq!(decoded, data, "blind single-strand error should be corrected without an erasure hint");
    }

    #[test]
    fn test_rs_combines_erasure_and_blind_error() {
        // 1 known erasure + 1 unknown error, with 4 parity: 2*1 + 1 = 3 <= 4.
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data = vec![
            vec![10, 20],
            vec![40, 50],
            vec![70, 80],
            vec![15, 25],
            vec![99, 1],
        ];
        let parity = rs.encode_block(&data).unwrap();
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        let mut received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        received[0] = None; // known erasure
        received[3] = Some(vec![16, 25]); // silent error

        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_rs_parity_reindexing_does_not_corrupt_decode() {
        // A parity strand missing from the MIDDLE of the list must not
        // shift the x-coordinates of the parity strands after it -- each
        // parity slot's position is its true codeword index, not its
        // position within whatever subset survived.
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data = vec![
            vec![1, 2],
            vec![3, 4],
            vec![5, 6],
            vec![7, 8],
            vec![9, 10],
        ];
        let parity = rs.encode_block(&data).unwrap();
        let mut parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();
        parity_opt[1] = None; // drop the 2nd parity strand, keep 3rd and 4th at their true slots

        let received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
        assert_eq!(decoded, data);
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
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();
        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_rs_recover_erasures_across_striped_block() {
        let rs = ReedSolomon::new(RsConfig::new(4));
        let data: Vec<Vec<u8>> = (0..300u32).map(|i| vec![(i % 256) as u8]).collect();
        let parity = rs.encode_block(&data).unwrap();
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        // Erase one strand in the first stripe and one in the second.
        let mut received: Vec<Option<Vec<u8>>> = data.iter().map(|s| Some(s.clone())).collect();
        received[10] = None;
        received[260] = None;

        let decoded = rs.decode_block(&received, &parity_opt).unwrap();
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
        let parity_opt: Vec<Option<Vec<u8>>> = parity.iter().map(|p| Some(p.clone())).collect();

        // Erase 2 strands but only 1 parity
        let received = vec![None, None, Some(data[2].clone())];
        let result = rs.decode_block(&received, &parity_opt);
        assert!(result.is_err());
    }

    #[test]
    fn test_rs_config() {
        let config = RsConfig::new(6);
        assert_eq!(config.max_erasures(), 6);
        assert_eq!(config.max_errors(), 3);
    }
}
