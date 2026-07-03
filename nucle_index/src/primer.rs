//! # Primer-Based Addressing System
//!
//! In DNA storage, primers serve as molecular addresses. Each file
//! gets a unique pair of forward + reverse primers that are:
//!
//! - **Orthogonal**: No cross-reactivity between primer pairs
//! - **GC-balanced**: 40–60% GC for reliable hybridization
//! - **No self-complementarity**: Avoids hairpin formation
//! - **Distinct Tm**: Melting temperatures within a narrow range
//!
//! Strands are physically tagged with their primer pair, enabling
//! selective PCR amplification (random access) of specific files.

use nucle_codec::base::{DnaStrand, Nucleotide};
use nucle_codec::constraints::{ConstraintConfig, ConstraintValidator};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fmt;

/// Standard primer length in nucleotides (20 nt is typical for PCR).
pub const DEFAULT_PRIMER_LENGTH: usize = 20;

/// Maximum edit-distance fraction (relative to primer length) tolerated when
/// locating a primer inside a physically synthesized/sequenced strand.
/// `PrimerLibrary::is_orthogonal` guarantees every pair of primers in a
/// library differs by a Hamming distance of at least 30% of their length, so
/// staying comfortably under that (20%) still tells different files' primers
/// apart while tolerating Nanopore-grade noise (~2-3% substitution + ~2%
/// insertion + ~2% deletion per base, so ~1-2 expected errors over a 20nt
/// primer).
const MAX_PRIMER_ERROR_FRACTION: f64 = 0.2;

/// How many bases of net insertion/deletion inside a primer to search for
/// when its exact boundary has been shifted by indel noise.
const MAX_PRIMER_SHIFT: usize = 4;

// ---------------------------------------------------------------------------
// Primer Pair
// ---------------------------------------------------------------------------

/// A pair of forward and reverse primers that uniquely address a file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PrimerPair {
    /// Unique identifier for this primer pair.
    pub id: String,
    /// Forward primer (5' → 3'), prepended to each strand.
    pub forward: DnaStrand,
    /// Reverse primer (5' → 3'), appended as reverse complement.
    pub reverse: DnaStrand,
}

impl PrimerPair {
    /// Create a new primer pair.
    pub fn new(id: &str, forward: DnaStrand, reverse: DnaStrand) -> Self {
        Self {
            id: id.to_string(),
            forward,
            reverse,
        }
    }

    /// Tag a data strand with this primer pair.
    ///
    /// Result: [forward_primer][data][reverse_complement_of_reverse]
    pub fn tag_strand(&self, data: &DnaStrand) -> DnaStrand {
        let mut tagged = Vec::with_capacity(
            self.forward.len() + data.len() + self.reverse.len(),
        );
        tagged.extend(self.forward.bases());
        tagged.extend(data.bases());
        tagged.extend(self.reverse.reverse_complement().bases());
        DnaStrand::new(tagged)
    }

    /// Extract the data portion from a tagged strand.
    ///
    /// Returns None if the strand doesn't match this primer pair. Primer
    /// boundaries are located by approximate (edit-distance-tolerant)
    /// matching rather than an exact-position slice, because an insertion or
    /// deletion inside either primer -- routine under Nanopore-grade noise --
    /// shifts exactly where the primer ends without changing that it's still
    /// recognizably the same primer.
    pub fn untag_strand(&self, tagged: &DnaStrand) -> Option<DnaStrand> {
        let fwd_end = Self::find_primer_end(tagged.bases(), self.forward.bases())?;
        let rev_complement = self.reverse.reverse_complement();
        let rev_start = Self::find_primer_start_from_end(tagged.bases(), rev_complement.bases())?;

        if rev_start <= fwd_end {
            return None;
        }

        Some(DnaStrand::new(tagged.bases()[fwd_end..rev_start].to_vec()))
    }

    /// Check if a strand starts with this primer pair's forward primer
    /// (tolerant of the same primer-boundary noise as [`Self::untag_strand`]).
    pub fn matches_forward(&self, strand: &DnaStrand) -> bool {
        Self::find_primer_end(strand.bases(), self.forward.bases()).is_some()
    }

    /// Find where `primer` ends inside `haystack`, assuming it starts at
    /// position 0 (true for a forward primer, which is always prepended).
    /// A net insertion or deletion inside the primer shifts where it ends
    /// without shifting where it starts, so this checks candidate end
    /// positions near the primer's nominal length and keeps whichever has
    /// the lowest edit distance, rejecting anything over
    /// `MAX_PRIMER_ERROR_FRACTION`.
    fn find_primer_end(haystack: &[Nucleotide], primer: &[Nucleotide]) -> Option<usize> {
        let nominal = primer.len();
        let max_errors = (nominal as f64 * MAX_PRIMER_ERROR_FRACTION).ceil() as usize;

        let lo = nominal.saturating_sub(MAX_PRIMER_SHIFT);
        let hi = (nominal + MAX_PRIMER_SHIFT).min(haystack.len());

        let mut best: Option<(usize, usize)> = None; // (end position, edit distance)
        for end in lo..=hi {
            let dist = edit_distance(primer, &haystack[..end]);
            if best.map_or(true, |(_, best_dist)| dist < best_dist) {
                best = Some((end, dist));
            }
        }

        best.filter(|&(_, dist)| dist <= max_errors).map(|(end, _)| end)
    }

    /// Mirror of [`Self::find_primer_end`] for a primer anchored to the
    /// *end* of `haystack` (the reverse-complement of the reverse primer,
    /// which is appended). Returns the start index of that occurrence in
    /// `haystack`'s own orientation.
    fn find_primer_start_from_end(haystack: &[Nucleotide], primer: &[Nucleotide]) -> Option<usize> {
        let reversed_haystack: Vec<Nucleotide> = haystack.iter().rev().copied().collect();
        let reversed_primer: Vec<Nucleotide> = primer.iter().rev().copied().collect();
        let end_from_reversed = Self::find_primer_end(&reversed_haystack, &reversed_primer)?;
        Some(haystack.len() - end_from_reversed)
    }

    /// Estimated melting temperature (Tm) using the Wallace rule.
    /// Tm = 2°C × (A+T count) + 4°C × (G+C count)
    pub fn forward_tm(&self) -> f64 {
        Self::wallace_tm(&self.forward)
    }

    /// Estimated Tm for the reverse primer.
    pub fn reverse_tm(&self) -> f64 {
        Self::wallace_tm(&self.reverse)
    }

    fn wallace_tm(strand: &DnaStrand) -> f64 {
        let gc = strand.bases().iter().filter(|n| n.is_gc()).count();
        let at = strand.len() - gc;
        2.0 * at as f64 + 4.0 * gc as f64
    }
}

/// Levenshtein edit distance between two nucleotide sequences (substitution,
/// insertion, and deletion each cost 1).
fn edit_distance(a: &[Nucleotide], b: &[Nucleotide]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        dp[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[n][m]
}

impl fmt::Display for PrimerPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Primer[{}] fwd={} rev={} Tm={:.0}°C/{:.0}°C",
            self.id,
            self.forward,
            self.reverse,
            self.forward_tm(),
            self.reverse_tm()
        )
    }
}

// ---------------------------------------------------------------------------
// Primer Library
// ---------------------------------------------------------------------------

/// A library of orthogonal primer pairs for addressing files.
///
/// Guarantees all primer pairs are:
/// - Unique (no duplicate sequences)
/// - Orthogonal (no cross-reactivity)
/// - Biologically valid (GC content, no hairpins)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimerLibrary {
    /// All generated primer pairs.
    pub primers: Vec<PrimerPair>,
    /// Map from primer ID to index in the primers vec.
    id_map: HashMap<String, usize>,
    /// Primer length.
    pub primer_length: usize,
}

impl PrimerLibrary {
    /// Create an empty primer library.
    pub fn new(primer_length: usize) -> Self {
        Self {
            primers: Vec::new(),
            id_map: HashMap::new(),
            primer_length,
        }
    }

    /// Number of primer pairs in the library.
    pub fn len(&self) -> usize {
        self.primers.len()
    }

    /// Whether the library is empty.
    pub fn is_empty(&self) -> bool {
        self.primers.is_empty()
    }

    /// Get a primer pair by ID.
    pub fn get(&self, id: &str) -> Option<&PrimerPair> {
        self.id_map.get(id).map(|&idx| &self.primers[idx])
    }

    /// Generate a library of `count` orthogonal primer pairs.
    pub fn generate(count: usize, primer_length: usize, seed: u64) -> Self {
        let mut library = Self::new(primer_length);
        let mut rng = StdRng::seed_from_u64(seed);
        let validator = ConstraintValidator::new(ConstraintConfig {
            gc_min: 0.40,
            gc_max: 0.60,
            max_homopolymer: 3,
            max_palindrome: 6,
            min_strand_length: 1,     // Primers are short
            max_strand_length: 1000,
            gc_window_size: primer_length,
        });

        let mut attempts = 0;
        let max_attempts = count * 1000;

        while library.len() < count && attempts < max_attempts {
            attempts += 1;

            // Generate random forward and reverse primers
            let forward = Self::random_primer(&mut rng, primer_length);
            let reverse = Self::random_primer(&mut rng, primer_length);

            // Validate biological constraints
            if !validator.is_valid(&forward) || !validator.is_valid(&reverse) {
                continue;
            }

            // Check orthogonality against existing primers
            let id = format!("P{:04}", library.len());
            let pair = PrimerPair::new(&id, forward, reverse);

            if library.is_orthogonal(&pair) {
                let idx = library.primers.len();
                library.id_map.insert(pair.id.clone(), idx);
                library.primers.push(pair);
            }
        }

        library
    }

    /// Generate a random primer sequence.
    fn random_primer(rng: &mut StdRng, length: usize) -> DnaStrand {
        let bases: Vec<Nucleotide> = (0..length)
            .map(|_| Nucleotide::ALL[rng.gen_range(0..4)])
            .collect();
        DnaStrand::new(bases)
    }

    /// Check if a new primer pair is orthogonal to all existing ones.
    ///
    /// Orthogonality: Hamming distance between any two primers must be
    /// at least 30% of the primer length (no cross-hybridization).
    fn is_orthogonal(&self, new_pair: &PrimerPair) -> bool {
        let min_distance = (self.primer_length as f64 * 0.3).ceil() as usize;

        for existing in &self.primers {
            // Check forward vs forward
            if Self::hamming_distance(&new_pair.forward, &existing.forward) < min_distance {
                return false;
            }
            // Check reverse vs reverse
            if Self::hamming_distance(&new_pair.reverse, &existing.reverse) < min_distance {
                return false;
            }
            // Check cross: new forward vs existing reverse
            if Self::hamming_distance(&new_pair.forward, &existing.reverse) < min_distance {
                return false;
            }
            // Check cross: new reverse vs existing forward
            if Self::hamming_distance(&new_pair.reverse, &existing.forward) < min_distance {
                return false;
            }
        }
        true
    }

    /// Hamming distance between two DNA strands of equal length.
    fn hamming_distance(a: &DnaStrand, b: &DnaStrand) -> usize {
        let len = a.len().min(b.len());
        a.bases()[..len]
            .iter()
            .zip(b.bases()[..len].iter())
            .filter(|(x, y)| x != y)
            .count()
    }

    /// Find which primer pair matches a tagged strand.
    pub fn identify_strand(&self, strand: &DnaStrand) -> Option<&PrimerPair> {
        self.primers.iter().find(|p| p.matches_forward(strand))
    }

    /// Assign a primer pair to a file by name.
    /// Returns the next available primer pair, or None if exhausted.
    pub fn assign_next(&self, used_count: usize) -> Option<&PrimerPair> {
        self.primers.get(used_count)
    }
}

impl fmt::Display for PrimerLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Primer Library ({} pairs, {} nt each):", self.len(), self.primer_length)?;
        for pair in &self.primers {
            writeln!(f, "  {}", pair)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primer_pair_tag_untag() {
        let fwd = DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap();
        let rev = DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap();
        let pair = PrimerPair::new("test", fwd, rev);

        let data = DnaStrand::from_str("AAACCCGGGTTT").unwrap();
        let tagged = pair.tag_strand(&data);

        // Tagged should be longer
        assert_eq!(tagged.len(), 20 + 12 + 20);

        // Untag should recover original data
        let recovered = pair.untag_strand(&tagged).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_untag_tolerates_single_insertion_in_forward_primer() {
        let fwd = DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap();
        let rev = DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap();
        let pair = PrimerPair::new("test", fwd, rev);
        let data = DnaStrand::from_str("AAACCCGGGTTT").unwrap();
        let tagged = pair.tag_strand(&data);

        // Insert an extra base into the forward primer region (index 5),
        // simulating a Nanopore insertion error -- this shifts every base
        // after it, so the old exact-slice-at-fwd_len approach would grab
        // the wrong window entirely.
        let mut noisy = tagged.bases().to_vec();
        noisy.insert(5, Nucleotide::T);
        let noisy = DnaStrand::new(noisy);

        assert!(pair.matches_forward(&noisy));
        let recovered = pair.untag_strand(&noisy).expect("should recover despite the insertion");
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_untag_tolerates_single_deletion_in_reverse_primer() {
        let fwd = DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap();
        let rev = DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap();
        let pair = PrimerPair::new("test", fwd, rev);
        let data = DnaStrand::from_str("AAACCCGGGTTT").unwrap();
        let tagged = pair.tag_strand(&data);

        // Delete a base from inside the appended reverse-complement region
        // (near the very end), simulating a Nanopore deletion error.
        let mut noisy = tagged.bases().to_vec();
        let last = noisy.len() - 1;
        noisy.remove(last - 3);
        let noisy = DnaStrand::new(noisy);

        let recovered = pair.untag_strand(&noisy).expect("should recover despite the deletion");
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_untag_tolerates_substitution_in_both_primers() {
        let fwd = DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap();
        let rev = DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap();
        let pair = PrimerPair::new("test", fwd, rev);
        let data = DnaStrand::from_str("AAACCCGGGTTT").unwrap();
        let tagged = pair.tag_strand(&data);

        let mut noisy = tagged.bases().to_vec();
        noisy[2] = Nucleotide::G; // corrupt a base inside the forward primer
        let last = noisy.len() - 1;
        noisy[last - 2] = Nucleotide::A; // corrupt a base inside the reverse-complement region
        let noisy = DnaStrand::new(noisy);

        let recovered = pair.untag_strand(&noisy).expect("should recover despite substitutions");
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_primer_mismatch() {
        let pair = PrimerPair::new(
            "test",
            DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap(),
            DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap(),
        );

        // Strand with wrong primer
        let wrong = DnaStrand::from_str("TTTTTTTTTTTTTTTTTTTTAAAA").unwrap();
        assert!(pair.untag_strand(&wrong).is_none());
    }

    #[test]
    fn test_primer_library_generation() {
        let library = PrimerLibrary::generate(5, 20, 42);

        assert_eq!(library.len(), 5);

        // All primers should be 20 nt
        for pair in &library.primers {
            assert_eq!(pair.forward.len(), 20);
            assert_eq!(pair.reverse.len(), 20);
        }

        // All IDs should be unique
        let ids: Vec<&str> = library.primers.iter().map(|p| p.id.as_str()).collect();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j]);
            }
        }
    }

    #[test]
    fn test_primer_orthogonality() {
        let library = PrimerLibrary::generate(10, 20, 42);

        // Verify all pairs are orthogonal (Hamming distance check)
        let min_dist = (20.0_f64 * 0.3).ceil() as usize;
        for i in 0..library.len() {
            for j in (i + 1)..library.len() {
                let dist = PrimerLibrary::hamming_distance(
                    &library.primers[i].forward,
                    &library.primers[j].forward,
                );
                assert!(
                    dist >= min_dist,
                    "primers {} and {} too similar (dist {})",
                    i, j, dist
                );
            }
        }
    }

    #[test]
    fn test_identify_strand() {
        let library = PrimerLibrary::generate(3, 20, 42);
        let data = DnaStrand::from_str("AAACCCGGGTTT").unwrap();

        let tagged = library.primers[1].tag_strand(&data);
        let identified = library.identify_strand(&tagged);

        assert!(identified.is_some());
        assert_eq!(identified.unwrap().id, library.primers[1].id);
    }

    #[test]
    fn test_primer_tm() {
        let pair = PrimerPair::new(
            "test",
            DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap(), // 10 GC, 10 AT
            DnaStrand::from_str("GCTAGCTAGCTAGCTAGCTA").unwrap(), // 10 GC, 10 AT
        );

        // Wallace rule: 2*AT + 4*GC = 2*10 + 4*10 = 60°C
        assert!((pair.forward_tm() - 60.0).abs() < 0.01);
        assert!((pair.reverse_tm() - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_get_by_id() {
        let library = PrimerLibrary::generate(3, 20, 42);

        assert!(library.get("P0000").is_some());
        assert!(library.get("P0001").is_some());
        assert!(library.get("P0002").is_some());
        assert!(library.get("nonexistent").is_none());
    }
}
