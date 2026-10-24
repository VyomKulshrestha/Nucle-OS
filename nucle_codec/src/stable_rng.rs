//! # Stable Random Streams
//!
//! Some data in a pool is rebuilt from a seed rather than stored: each
//! Fountain droplet's segment list and whitening mask, and the primer
//! library a pool opens with. Rebuilding gives back the same data only if
//! the seed produces the same numbers as when it was written, so these
//! streams can't come from the `rand` crate, which changed what a seeded
//! `gen_range` returns in 0.9. The noise simulator (`nucle_synth`) and the
//! blind evaluation of real reads use it too, so a seed in a test or a
//! published measurement gives the same result whatever rand does.
//!
//! [`StableRng`] is a fixed copy of what those streams were written with:
//! rand 0.8's `StdRng` (ChaCha with 12 rounds, a 64-bit block counter and a
//! zero stream, refilled four blocks at a time), seeded the way rand_core
//! 0.6's `seed_from_u64` fills the key from PCG32, and sampled the way rand
//! 0.8 samples an `f64`, a `u8`, a `usize` range and an `f64` range. It was
//! checked against rand 0.8 draw for draw while that was still the
//! workspace's generator, and its tests pin the streams with digests
//! recorded then, so archives written under rand 0.8 still decode.

use std::ops::Range;

/// Words of keystream generated per refill: four 16-word ChaCha blocks.
const BUFFER_WORDS: usize = 64;

/// A deterministic generator whose output never changes; see the module
/// documentation.
#[derive(Debug, Clone)]
pub struct StableRng {
    key: [u32; 8],
    counter: u64,
    buffer: [u32; BUFFER_WORDS],
    index: usize,
}

impl StableRng {
    /// The generator rand 0.8's `StdRng::seed_from_u64(seed)` was.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut state = seed;
        let mut key = [0u32; 8];
        for word in &mut key {
            *word = pcg32(&mut state);
        }
        Self { key, counter: 0, buffer: [0; BUFFER_WORDS], index: BUFFER_WORDS }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= BUFFER_WORDS {
            self.refill();
            self.index = 0;
        }
        let value = self.buffer[self.index];
        self.index += 1;
        value
    }

    /// Two words, low first; one left before a refill pairs with the first
    /// word after it.
    pub fn next_u64(&mut self) -> u64 {
        if self.index < BUFFER_WORDS - 1 {
            let (lo, hi) = (self.buffer[self.index], self.buffer[self.index + 1]);
            self.index += 2;
            u64::from(hi) << 32 | u64::from(lo)
        } else if self.index >= BUFFER_WORDS {
            self.refill();
            self.index = 2;
            u64::from(self.buffer[1]) << 32 | u64::from(self.buffer[0])
        } else {
            let lo = self.buffer[BUFFER_WORDS - 1];
            self.refill();
            self.index = 1;
            u64::from(self.buffer[0]) << 32 | u64::from(lo)
        }
    }

    /// A uniform `f64` in [0, 1) from the top 53 bits of a `u64`.
    pub fn gen_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// The low byte of a `u32`.
    pub fn gen_u8(&mut self) -> u8 {
        self.next_u32() as u8
    }

    /// A uniform value in `range`, by widening multiplication with
    /// rejection outside a zone (rand 0.8's `UniformInt::sample_single`).
    pub fn gen_range(&mut self, range: Range<usize>) -> usize {
        assert!(range.start < range.end, "gen_range called with an empty range");
        let span = (range.end - 1 - range.start).wrapping_add(1) as u64;
        if span == 0 {
            return self.next_u64() as usize;
        }
        let zone = (span << span.leading_zeros()).wrapping_sub(1);
        loop {
            let product = u128::from(self.next_u64()) * u128::from(span);
            let (hi, lo) = ((product >> 64) as u64, product as u64);
            if lo <= zone {
                return range.start + hi as usize;
            }
        }
    }

    /// A uniform `f64` in `range` (rand 0.8's `UniformFloat::sample_single`):
    /// 52 random bits make a value in [1, 2), shifted and scaled into the
    /// range; one that rounds up to its end is drawn again with the scale
    /// one unit in the last place smaller.
    pub fn gen_f64_range(&mut self, range: Range<f64>) -> f64 {
        let (low, high) = (range.start, range.end);
        assert!(low < high, "gen_f64_range called with an empty range");
        let mut scale = high - low;
        loop {
            let one_to_two = f64::from_bits((self.next_u64() >> 12) | (1023u64 << 52));
            let value = (one_to_two - 1.0) * scale + low;
            if value < high {
                return value;
            }
            scale = f64::from_bits(scale.to_bits() - 1);
        }
    }

    fn refill(&mut self) {
        for block in 0..4 {
            let words = chacha12_block(&self.key, self.counter);
            self.buffer[block * 16..block * 16 + 16].copy_from_slice(&words);
            self.counter = self.counter.wrapping_add(1);
        }
    }
}

/// One step of PCG32, as rand_core 0.6 uses it to expand a `u64` seed.
fn pcg32(state: &mut u64) -> u32 {
    const MUL: u64 = 6_364_136_223_846_793_005;
    const INC: u64 = 11_634_580_027_462_260_723;
    *state = state.wrapping_mul(MUL).wrapping_add(INC);
    let s = *state;
    let xorshifted = (((s >> 18) ^ s) >> 27) as u32;
    xorshifted.rotate_right((s >> 59) as u32)
}

/// One ChaCha block with 12 rounds: constants, key, a 64-bit counter and a
/// zero 64-bit stream.
fn chacha12_block(key: &[u32; 8], counter: u64) -> [u32; 16] {
    let mut input = [0u32; 16];
    input[..4].copy_from_slice(&[0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574]);
    input[4..12].copy_from_slice(key);
    input[12] = counter as u32;
    input[13] = (counter >> 32) as u32;
    let mut x = input;
    for _ in 0..6 {
        quarter_round(&mut x, 0, 4, 8, 12);
        quarter_round(&mut x, 1, 5, 9, 13);
        quarter_round(&mut x, 2, 6, 10, 14);
        quarter_round(&mut x, 3, 7, 11, 15);
        quarter_round(&mut x, 0, 5, 10, 15);
        quarter_round(&mut x, 1, 6, 11, 12);
        quarter_round(&mut x, 2, 7, 8, 13);
        quarter_round(&mut x, 3, 4, 9, 14);
    }
    for (word, start) in x.iter_mut().zip(input) {
        *word = word.wrapping_add(start);
    }
    x
}

fn quarter_round(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(16);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(12);
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(8);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(7);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a over `bytes`, folded into `hash`.
    fn fnv(hash: u64, bytes: &[u8]) -> u64 {
        bytes.iter().fold(hash, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    }
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

    /// The streams themselves, as digests: recorded while this generator
    /// matched rand 0.8 draw for draw, they keep holding once rand moves on.
    #[test]
    fn streams_are_pinned() {
        for (seed, expected) in [(0u64, 0x2361_e5f6_36b0_f514u64), (42, 0xfe35_1c42_5e27_eb53), (u64::MAX, 0xd929_41e6_2ab3_8250)] {
            let mut rng = StableRng::seed_from_u64(seed);
            let mut h = FNV_OFFSET;
            for i in 0..1000usize {
                h = fnv(h, &rng.next_u64().to_le_bytes());
                h = fnv(h, &rng.gen_f64().to_bits().to_le_bytes());
                h = fnv(h, &[rng.gen_u8()]);
                h = fnv(h, &(rng.gen_range(i % 7..(i % 7) + 1 + i % 500) as u64).to_le_bytes());
            }
            assert_eq!(h, expected, "seed {seed}");
        }
    }

    /// `gen_f64_range`, checked the same way: rand 0.8's `gen_range` over
    /// the same ranges gave these digests.
    #[test]
    fn float_ranges_are_pinned() {
        for (seed, expected) in [(0u64, 0xbe3e_780b_b0a4_c8f5u64), (42, 0x5f77_7773_e4a9_13c8), (u64::MAX, 0x78ba_018f_8d85_e320)] {
            let mut rng = StableRng::seed_from_u64(seed);
            let mut h = FNV_OFFSET;
            for i in 0..5000 {
                let value = match i % 3 {
                    0 => rng.gen_f64_range(0.1..0.5),
                    1 => rng.gen_f64_range(-3.0..7.25),
                    _ => rng.gen_f64_range(0.0..1e-300),
                };
                h = fnv(h, &value.to_bits().to_le_bytes());
            }
            assert_eq!(h, expected, "seed {seed}");
        }
    }
}
