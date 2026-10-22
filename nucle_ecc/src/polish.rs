//! # Length-Aware Median Polishing
//!
//! Partial-order consensus ([`crate::consensus::build_consensus`]) is a
//! per-column majority vote, so it inherits any error a *majority* of reads
//! share. On real nanopore data that error is overwhelmingly a homopolymer
//! length call: scored against the Clustered Nanopore Reads dataset
//! (see [`crate::realdata`]), nearly every residual failure at high coverage
//! was a run like `CCCCCC` called as `CCCCC`, or a few junk bases glued onto
//! one end, both of which change the strand's length.
//!
//! A decoder always knows how long its strands are supposed to be, and the
//! voting stage throws that away. [`polish`] puts it back. It treats the
//! consensus as a candidate *median string* (the sequence minimizing the
//! summed edit distance to every read) and improves it one base edit at a
//! time:
//!
//! 1. While the candidate is the wrong length, apply the single insertion
//!    (if too short) or deletion (if too long) that keeps the summed edit
//!    distance lowest, even if that sum goes up: the length is a hard fact,
//!    the vote is only evidence.
//! 2. Then apply any single edit that strictly lowers the summed distance
//!    (substitutions only when a target length is set, so step 1's work is
//!    never undone), until none does.
//!
//! Every candidate edit is scored in `O(read length)` per read from
//! precomputed forward and backward alignment matrices, rather than by
//! realigning each read from scratch.

use nucle_codec::base::{DnaStrand, Nucleotide};

const BASES: [Nucleotide; 4] = [Nucleotide::A, Nucleotide::T, Nucleotide::G, Nucleotide::C];

/// Upper bound on accepted edits, so a pathological read set can never
/// make polishing loop for long.
const MAX_EDITS: usize = 24;

/// One read's alignment against the current candidate `c`:
/// `fwd[i][j]` = edit distance of `c[..i]` vs `read[..j]`,
/// `bwd[i][j]` = edit distance of `c[i..]` vs `read[j..]`.
struct Alignment<'a> {
    read: &'a [Nucleotide],
    fwd: Vec<Vec<u32>>,
    bwd: Vec<Vec<u32>>,
}

impl<'a> Alignment<'a> {
    fn new(c: &[Nucleotide], read: &'a [Nucleotide]) -> Self {
        let (n, m) = (c.len(), read.len());
        let mut fwd = vec![vec![0u32; m + 1]; n + 1];
        for (j, cell) in fwd[0].iter_mut().enumerate() {
            *cell = j as u32;
        }
        for i in 1..=n {
            fwd[i][0] = i as u32;
            for j in 1..=m {
                let sub = fwd[i - 1][j - 1] + u32::from(c[i - 1] != read[j - 1]);
                fwd[i][j] = sub.min(fwd[i - 1][j] + 1).min(fwd[i][j - 1] + 1);
            }
        }
        let mut bwd = vec![vec![0u32; m + 1]; n + 1];
        for (j, cell) in bwd[n].iter_mut().enumerate() {
            *cell = (m - j) as u32;
        }
        for i in (0..n).rev() {
            bwd[i][m] = (n - i) as u32;
            for j in (0..m).rev() {
                let sub = bwd[i + 1][j + 1] + u32::from(c[i] != read[j]);
                bwd[i][j] = sub.min(bwd[i + 1][j] + 1).min(bwd[i][j + 1] + 1);
            }
        }
        Self { read, fwd, bwd }
    }

    fn distance(&self) -> u32 {
        self.fwd[self.fwd.len() - 1][self.read.len()]
    }

    /// Distance after deleting candidate base `i`.
    fn with_deletion(&self, i: usize) -> u32 {
        (0..=self.read.len()).map(|j| self.fwd[i][j] + self.bwd[i + 1][j]).min().unwrap_or(u32::MAX)
    }

    /// Distance after inserting `b` before candidate position `i`.
    fn with_insertion(&self, i: usize, b: Nucleotide) -> u32 {
        let m = self.read.len();
        let mut best = u32::MAX;
        for j in 0..=m {
            best = best.min(self.fwd[i][j] + 1 + self.bwd[i][j]);
            if j < m {
                best = best.min(self.fwd[i][j] + u32::from(self.read[j] != b) + self.bwd[i][j + 1]);
            }
        }
        best
    }

    /// Distance after replacing candidate base `i` with `b`.
    fn with_substitution(&self, i: usize, b: Nucleotide) -> u32 {
        let m = self.read.len();
        let mut best = u32::MAX;
        for j in 0..=m {
            best = best.min(self.fwd[i][j] + 1 + self.bwd[i + 1][j]);
            if j < m {
                best = best.min(self.fwd[i][j] + u32::from(self.read[j] != b) + self.bwd[i + 1][j + 1]);
            }
        }
        best
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edit {
    Delete(usize),
    Insert(usize, Nucleotide),
    Substitute(usize, Nucleotide),
}

fn apply(c: &mut Vec<Nucleotide>, edit: Edit) {
    match edit {
        Edit::Delete(i) => {
            c.remove(i);
        }
        Edit::Insert(i, b) => c.insert(i, b),
        Edit::Substitute(i, b) => c[i] = b,
    }
}

/// Every distinct candidate edit of the requested kinds. Inserting `b`
/// next to an identical base yields the same string wherever in the run
/// it lands, so only the run's leading slot is kept; likewise deleting any
/// base of a run is one string, so only its first base is kept.
fn candidates(c: &[Nucleotide], deletions: bool, insertions: bool, substitutions: bool) -> Vec<Edit> {
    let mut out = Vec::new();
    if deletions {
        for i in 0..c.len() {
            if i == 0 || c[i] != c[i - 1] {
                out.push(Edit::Delete(i));
            }
        }
    }
    if insertions {
        for i in 0..=c.len() {
            for &b in &BASES {
                if i > 0 && c[i - 1] == b {
                    continue;
                }
                out.push(Edit::Insert(i, b));
            }
        }
    }
    if substitutions {
        for (i, &cur) in c.iter().enumerate() {
            for &b in BASES.iter().filter(|&&b| b != cur) {
                out.push(Edit::Substitute(i, b));
            }
        }
    }
    out
}

fn total_after(aligns: &[Alignment<'_>], edit: Edit) -> u64 {
    aligns
        .iter()
        .map(|a| {
            u64::from(match edit {
                Edit::Delete(i) => a.with_deletion(i),
                Edit::Insert(i, b) => a.with_insertion(i, b),
                Edit::Substitute(i, b) => a.with_substitution(i, b),
            })
        })
        .sum()
}

/// Polish `consensus` against `reads` toward an approximate median string,
/// forcing it to `target_len` bases when one is given. See the module docs.
///
/// Returns the input unchanged when there are fewer than two reads (no
/// evidence to polish with).
pub fn polish(consensus: &DnaStrand, reads: &[DnaStrand], target_len: Option<usize>) -> DnaStrand {
    let mut c: Vec<Nucleotide> = consensus.bases().to_vec();
    if reads.len() < 2 || c.is_empty() {
        return consensus.clone();
    }

    for _ in 0..MAX_EDITS {
        let aligns: Vec<Alignment<'_>> = reads.iter().map(|r| Alignment::new(&c, r.bases())).collect();
        let current: u64 = aligns.iter().map(|a| u64::from(a.distance())).sum();

        let (must_change, pool) = match target_len {
            Some(t) if c.len() > t && c.len() > 1 => (true, candidates(&c, true, false, false)),
            Some(t) if c.len() < t => (true, candidates(&c, false, true, false)),
            Some(_) => (false, candidates(&c, false, false, true)),
            None => (false, candidates(&c, true, true, true)),
        };

        let best = pool
            .into_iter()
            .map(|e| (total_after(&aligns, e), e))
            .min_by_key(|&(score, _)| score);

        match best {
            Some((score, edit)) if must_change || score < current => apply(&mut c, edit),
            _ => break,
        }
    }

    DnaStrand::new(c)
}

/// Per-position agreement of `reads` with `sequence`: for each base of
/// `sequence`, the fraction of reads that align a matching base to it in an
/// optimal global alignment.
pub fn column_agreement(sequence: &DnaStrand, reads: &[DnaStrand]) -> Vec<f64> {
    let c = sequence.bases();
    let mut hits = vec![0usize; c.len()];
    for read in reads {
        let a = Alignment::new(c, read.bases());
        let (mut i, mut j) = (c.len(), read.len());
        while i > 0 && j > 0 {
            let here = a.fwd[i][j];
            let diag = a.fwd[i - 1][j - 1];
            if c[i - 1] == a.read[j - 1] && here == diag {
                hits[i - 1] += 1;
                i -= 1;
                j -= 1;
            } else if here == diag + 1 {
                i -= 1;
                j -= 1;
            } else if here == a.fwd[i - 1][j] + 1 {
                i -= 1;
            } else {
                j -= 1;
            }
        }
    }
    let n = reads.len().max(1) as f64;
    hits.into_iter().map(|h| h as f64 / n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> DnaStrand {
        DnaStrand::from_str(x).unwrap()
    }

    #[test]
    fn forward_backward_scores_match_brute_force_realignment() {
        let c = s("ACGGTTCA");
        let read = s("ACGTTTCAG");
        let a = Alignment::new(c.bases(), read.bases());
        let brute = |x: &str| crate::realdata::edit_distance(s(x).bases(), read.bases()) as u32;
        assert_eq!(a.distance(), brute("ACGGTTCA"));
        assert_eq!(a.with_deletion(2), brute("ACGTTCA"));
        assert_eq!(a.with_insertion(8, Nucleotide::G), brute("ACGGTTCAG"));
        assert_eq!(a.with_insertion(0, Nucleotide::T), brute("TACGGTTCA"));
        assert_eq!(a.with_substitution(3, Nucleotide::T), brute("ACGTTTCA"));
    }

    #[test]
    fn restores_a_homopolymer_the_majority_under_called() {
        // Truth has CCCCCC; most reads lose one C (the nanopore failure mode),
        // so a plain vote says CCCCC. The known length decides it.
        let truth = "ATGACCCCCCGTAGT";
        let under = "ATGACCCCCGTAGT";
        let reads: Vec<DnaStrand> =
            [under, under, under, truth, truth].iter().map(|r| s(r)).collect();
        let polished = polish(&s(under), &reads, Some(truth.len()));
        assert_eq!(polished.to_string(), truth);
    }

    #[test]
    fn trims_a_junk_tail_back_to_the_known_length() {
        let truth = "GATTACAGATTACAGGT";
        let reads: Vec<DnaStrand> = [truth, truth, "GATTACAGATTACAGGTGAT", truth]
            .iter()
            .map(|r| s(r))
            .collect();
        let polished = polish(&s("GATTACAGATTACAGGTGAT"), &reads, Some(truth.len()));
        assert_eq!(polished.to_string(), truth);
    }

    #[test]
    fn leaves_a_correct_consensus_alone() {
        let truth = "ACGTACGGTCAAGT";
        let reads: Vec<DnaStrand> = [truth, "ACGTACGTCAAGT", truth, "ACGTTCGGTCAAGT"].iter().map(|r| s(r)).collect();
        assert_eq!(polish(&s(truth), &reads, Some(truth.len())).to_string(), truth);
        assert_eq!(polish(&s(truth), &reads, None).to_string(), truth);
    }

    #[test]
    fn column_agreement_counts_matching_reads_per_position() {
        let truth = s("ACGTAC");
        let reads = vec![s("ACGTAC"), s("ACGTAC"), s("ACTTAC"), s("ACGAC")];
        let conf = column_agreement(&truth, &reads);
        assert_eq!(conf.len(), 6);
        assert_eq!(conf[0], 1.0);
        // Position 2 (G): the third read has T there.
        assert_eq!(conf[2], 0.75);
        // Position 3 (T): the fourth read deleted it.
        assert_eq!(conf[3], 0.75);
    }

    #[test]
    fn single_read_is_returned_unchanged() {
        let only = s("ACGT");
        assert_eq!(polish(&only, std::slice::from_ref(&only), Some(10)), only);
    }
}
