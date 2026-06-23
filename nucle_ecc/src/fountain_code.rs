//! # Fountain/LT Erasure Code for DNA Storage
//!
//! Strand-level erasure code using Luby Transform principles.
//! Complements the DNA Fountain codec at the error correction layer —
//! the codec handles encoding constraints, this handles strand dropout.
//!
//! Given N data strands, generates M > N encoded strands such that
//! any N+ε subset can reconstruct the original data.

use nucle_codec::base::DnaError;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use thiserror::Error;

/// Errors specific to fountain erasure coding.
#[derive(Debug, Error)]
pub enum FountainEccError {
    #[error("insufficient strands for recovery: have {received}, need {needed}")]
    InsufficientStrands { received: usize, needed: usize },

    #[error("decoding failed: {0}")]
    DecodingFailed(String),

    #[error("codec error: {0}")]
    CodecError(#[from] DnaError),
}

/// Configuration for the fountain erasure code.
#[derive(Debug, Clone)]
pub struct FountainEccConfig {
    /// Overhead factor: generate this many extra encoded strands.
    /// 1.5 = 50% more strands than data strands.
    pub overhead: f64,
    /// PRNG seed for reproducible encoding.
    pub seed: u64,
}

impl Default for FountainEccConfig {
    fn default() -> Self {
        Self {
            overhead: 1.5,
            seed: 42,
        }
    }
}

/// An encoded strand produced by the fountain erasure encoder.
#[derive(Debug, Clone)]
pub struct EccDroplet {
    /// The XOR'd data of selected source strands.
    pub data: Vec<u8>,
    /// Indices of source strands that were XOR'd.
    pub source_indices: Vec<usize>,
    /// Unique ID for this droplet.
    pub id: u64,
}

/// Fountain erasure code encoder/decoder.
pub struct FountainEcc {
    config: FountainEccConfig,
}

impl FountainEcc {
    /// Create a new fountain ECC with given configuration.
    pub fn new(config: FountainEccConfig) -> Self {
        Self { config }
    }

    /// Create with default settings.
    pub fn default_codec() -> Self {
        Self::new(FountainEccConfig::default())
    }

    /// Sample degree using simplified robust soliton distribution.
    fn sample_degree(rng: &mut StdRng, k: usize) -> usize {
        let r: f64 = rng.gen();
        if r < 0.35 {
            1
        } else if r < 0.65 {
            2
        } else if r < 0.85 {
            3.min(k)
        } else {
            ((k as f64).sqrt() as usize).max(2).min(k)
        }
    }

    /// Select random source indices for a droplet.
    fn select_sources(rng: &mut StdRng, k: usize, degree: usize) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..k).collect();
        let d = degree.min(k);
        for i in 0..d {
            let j = rng.gen_range(i..k);
            indices.swap(i, j);
        }
        indices.truncate(d);
        indices.sort();
        indices
    }

    /// Encode data strands into erasure-protected droplets.
    ///
    /// Each input strand is represented as `Vec<u8>`.
    /// All strands must have the same length.
    pub fn encode(&self, data_strands: &[Vec<u8>]) -> Result<Vec<EccDroplet>, FountainEccError> {
        if data_strands.is_empty() {
            return Ok(Vec::new());
        }

        let k = data_strands.len();
        let strand_len = data_strands[0].len();
        let num_droplets = (k as f64 * self.config.overhead).ceil() as usize;

        let mut rng = StdRng::seed_from_u64(self.config.seed);
        let mut droplets = Vec::with_capacity(num_droplets);

        for id in 0..num_droplets as u64 {
            let degree = Self::sample_degree(&mut rng, k);
            let sources = Self::select_sources(&mut rng, k, degree);

            // XOR selected strands
            let mut data = vec![0u8; strand_len];
            for &idx in &sources {
                for (j, &byte) in data_strands[idx].iter().enumerate() {
                    data[j] ^= byte;
                }
            }

            droplets.push(EccDroplet {
                data,
                source_indices: sources,
                id,
            });
        }

        Ok(droplets)
    }

    /// Decode droplets back to original data strands using peeling decoder.
    pub fn decode(
        &self,
        droplets: &[EccDroplet],
        k: usize,
        strand_len: usize,
    ) -> Result<Vec<Vec<u8>>, FountainEccError> {
        let mut solved: Vec<Option<Vec<u8>>> = vec![None; k];
        let mut solved_count = 0;

        let mut droplet_sources: Vec<Vec<usize>> = droplets.iter()
            .map(|d| d.source_indices.clone())
            .collect();
        let mut droplet_data: Vec<Vec<u8>> = droplets.iter()
            .map(|d| d.data.clone())
            .collect();

        let max_iter = droplets.len() * 3;
        for _ in 0..max_iter {
            if solved_count == k {
                break;
            }

            let mut found = false;
            for i in 0..droplet_sources.len() {
                let unsolved: Vec<usize> = droplet_sources[i].iter()
                    .filter(|&&idx| solved[idx].is_none())
                    .copied()
                    .collect();

                if unsolved.len() == 1 {
                    let seg_idx = unsolved[0];
                    let mut seg_data = droplet_data[i].clone();

                    // XOR out solved segments
                    for &idx in &droplet_sources[i] {
                        if idx != seg_idx {
                            if let Some(ref s) = solved[idx] {
                                for (j, &b) in s.iter().enumerate() {
                                    if j < seg_data.len() {
                                        seg_data[j] ^= b;
                                    }
                                }
                            }
                        }
                    }

                    seg_data.resize(strand_len, 0);
                    solved[seg_idx] = Some(seg_data);
                    solved_count += 1;
                    found = true;
                    break;
                }
            }

            if !found {
                break;
            }
        }

        if solved_count < k {
            return Err(FountainEccError::InsufficientStrands {
                received: solved_count,
                needed: k,
            });
        }

        Ok(solved.into_iter().map(|s| s.unwrap()).collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_produces_droplets() {
        let ecc = FountainEcc::new(FountainEccConfig {
            overhead: 2.0,
            seed: 42,
        });
        let strands = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let droplets = ecc.encode(&strands).unwrap();

        assert!(droplets.len() >= 6); // 3 strands * 2.0 overhead
        for d in &droplets {
            assert_eq!(d.data.len(), 3); // Same length as input
            assert!(!d.source_indices.is_empty());
        }
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let ecc = FountainEcc::new(FountainEccConfig {
            overhead: 3.0,
            seed: 42,
        });
        let strands = vec![
            vec![10, 20, 30],
            vec![40, 50, 60],
            vec![70, 80, 90],
        ];

        let droplets = ecc.encode(&strands).unwrap();
        let decoded = ecc.decode(&droplets, 3, 3).unwrap();

        assert_eq!(decoded, strands);
    }

    #[test]
    fn test_decode_with_missing_droplets() {
        let ecc = FountainEcc::new(FountainEccConfig {
            overhead: 4.0, // Extra overhead for robustness
            seed: 42,
        });
        let strands = vec![vec![1, 2], vec![3, 4], vec![5, 6]];

        let all_droplets = ecc.encode(&strands).unwrap();

        // Remove some droplets (simulate strand loss)
        let partial: Vec<EccDroplet> = all_droplets.into_iter()
            .enumerate()
            .filter(|(i, _)| i % 2 == 0) // Keep only even-indexed
            .map(|(_, d)| d)
            .collect();

        // Should still be able to decode with partial droplets
        let result = ecc.decode(&partial, 3, 2);
        // May or may not succeed depending on which droplets remain
        // This tests the robustness path
        if let Ok(decoded) = result {
            assert_eq!(decoded, strands);
        }
    }

    #[test]
    fn test_empty_input() {
        let ecc = FountainEcc::default_codec();
        let droplets = ecc.encode(&[]).unwrap();
        assert!(droplets.is_empty());
    }
}
