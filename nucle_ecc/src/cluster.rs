//! # Read Clustering
//!
//! A sequencer returns reads unlabeled and shuffled: nothing says which
//! stored strand a read came from. Consensus needs those reads grouped
//! first. This module recovers the groups from the reads alone, in the
//! spirit of Rashtchian et al., "Clustering Billions of Reads for DNA Data
//! Storage" (NeurIPS 2017): a cheap signature proposes candidate pairs and
//! an edit-distance check confirms them.
//!
//! 1. Each read is reduced to a MinHash sketch of its k-mers. Two reads of
//!    the same strand share many k-mers even after indels shift them, while
//!    two unrelated random strands share almost none.
//! 2. Locality-sensitive hashing splits each sketch into bands; reads that
//!    agree on every value of any one band land in the same bucket and
//!    become candidates.
//! 3. A candidate pair is merged (union-find) only if a banded edit
//!    distance confirms the reads are within `max_distance_fraction` of
//!    each other, so a false LSH collision never joins two strands.
//!
//! Reads of one strand need only form a connected chain of confirmed pairs
//! to end up in one cluster, not all-pairs agreement.

use nucle_codec::base::{DnaStrand, Nucleotide};
use std::collections::HashMap;

/// Tuning for [`cluster_reads`]. The defaults were chosen on real MinION
/// reads (the Clustered Nanopore Reads dataset, about 6% per-read error,
/// 110 nt strands): at a 0.15 distance fraction, clusters were 100% pure
/// and blind decoding recovered 94.4% of strands, against 94.65% with the
/// true labels. At 0.25, chains of near-threshold pairs merged distinct
/// strands (92.4% purity, 82.9% recovered). Raise it for noisier reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterConfig {
    /// k-mer length for sketching (at most 16).
    pub k: usize,
    /// LSH bands; more bands find more true pairs.
    pub bands: usize,
    /// MinHash values per band; more rows make buckets more selective.
    pub rows: usize,
    /// Largest edit distance, as a fraction of the longer read, that still
    /// counts as the same strand.
    pub max_distance_fraction: f64,
    /// Seed for the MinHash functions, for reproducible clusterings.
    pub seed: u64,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self { k: 8, bands: 32, rows: 1, max_distance_fraction: 0.15, seed: 0x6E75_636C_655F_6F73 }
    }
}

/// Group reads that come from the same strand. Returns clusters as lists of
/// indices into `reads`, largest first; every read is in exactly one
/// cluster (an unmatched read is a cluster of one).
pub fn cluster_reads(reads: &[DnaStrand], config: &ClusterConfig) -> Vec<Vec<usize>> {
    let k = config.k.clamp(1, 16);
    let hashes = config.bands.max(1) * config.rows.max(1);
    let sketches: Vec<Vec<u64>> = reads.iter().map(|r| sketch(r.bases(), k, hashes, config.seed)).collect();

    let mut parent: Vec<usize> = (0..reads.len()).collect();
    for band in 0..config.bands.max(1) {
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, sk) in sketches.iter().enumerate() {
            if sk.is_empty() {
                continue;
            }
            let slice = &sk[band * config.rows.max(1)..(band + 1) * config.rows.max(1)];
            let key = slice.iter().fold(band as u64, |acc, &v| mix(acc ^ v));
            buckets.entry(key).or_default().push(i);
        }
        for members in buckets.values().filter(|m| m.len() > 1) {
            // Confirm each member against the bucket's first read and its
            // neighbour: linear in the bucket, and enough for connectivity.
            let anchor = members[0];
            for w in members.windows(2) {
                for (a, b) in [(anchor, w[1]), (w[0], w[1])] {
                    if a != b && find(&mut parent, a) != find(&mut parent, b) && same_strand(&reads[a], &reads[b], config) {
                        union(&mut parent, a, b);
                    }
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..reads.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut clusters: Vec<Vec<usize>> = groups.into_values().collect();
    clusters.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    clusters
}

fn same_strand(a: &DnaStrand, b: &DnaStrand, config: &ClusterConfig) -> bool {
    let limit = (config.max_distance_fraction * a.len().max(b.len()) as f64).floor() as usize;
    bounded_edit_distance(a.bases(), b.bases(), limit).is_some()
}

/// Edit distance if it is at most `limit`, computed only inside the
/// diagonal band `|i - j| <= limit`; `None` once it must exceed `limit`.
pub fn bounded_edit_distance(a: &[Nucleotide], b: &[Nucleotide], limit: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    const INF: usize = usize::MAX / 2;
    let m = b.len();
    let mut prev = vec![INF; m + 1];
    let mut cur = vec![INF; m + 1];
    for (j, cell) in prev.iter_mut().enumerate().take(limit.min(m) + 1) {
        *cell = j;
    }
    for i in 1..=a.len() {
        let lo = i.saturating_sub(limit);
        let hi = (i + limit).min(m);
        cur.iter_mut().for_each(|c| *c = INF);
        if lo == 0 {
            cur[0] = i;
        }
        let mut row_min = cur[0];
        for j in lo.max(1)..=hi {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let v = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[m] <= limit).then_some(prev[m])
}

fn sketch(bases: &[Nucleotide], k: usize, hashes: usize, seed: u64) -> Vec<u64> {
    if bases.len() < k {
        return Vec::new();
    }
    let mask: u64 = if k == 32 { u64::MAX } else { (1u64 << (2 * k)) - 1 };
    let mut mins = vec![u64::MAX; hashes];
    let mut kmer: u64 = 0;
    for (i, &n) in bases.iter().enumerate() {
        kmer = ((kmer << 2) | code(n)) & mask;
        if i + 1 < k {
            continue;
        }
        for (h, m) in mins.iter_mut().enumerate() {
            let v = mix(kmer ^ mix(seed.wrapping_add(h as u64)));
            if v < *m {
                *m = v;
            }
        }
    }
    mins
}

fn code(n: Nucleotide) -> u64 {
    match n {
        Nucleotide::A => 0,
        Nucleotide::C => 1,
        Nucleotide::G => 2,
        Nucleotide::T => 3,
    }
}

/// SplitMix64 finalizer: a fast, well-mixed 64-bit hash.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb)] = ra.min(rb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn random_strand(rng: &mut StdRng, len: usize) -> Vec<Nucleotide> {
        (0..len).map(|_| Nucleotide::ALL[rng.gen_range(0..4)]).collect()
    }

    /// A noisy copy with independent substitution/insertion/deletion at `rate` each.
    fn noisy(rng: &mut StdRng, src: &[Nucleotide], rate: f64) -> DnaStrand {
        let mut out = Vec::new();
        for &b in src {
            let r: f64 = rng.gen();
            if r < rate {
                continue; // deletion
            } else if r < 2.0 * rate {
                out.push(Nucleotide::ALL[rng.gen_range(0..4)]); // substitution
            } else {
                out.push(b);
            }
            if rng.gen::<f64>() < rate {
                out.push(Nucleotide::ALL[rng.gen_range(0..4)]); // insertion
            }
        }
        DnaStrand::new(out)
    }

    #[test]
    fn bounded_edit_distance_matches_full_distance_within_the_limit() {
        let s = |x: &str| DnaStrand::from_str(x).unwrap();
        let (a, b) = (s("ACGTACGTAC"), s("ACGAACGTTAC"));
        let full = crate::realdata::edit_distance(a.bases(), b.bases());
        assert_eq!(bounded_edit_distance(a.bases(), b.bases(), full), Some(full));
        assert_eq!(bounded_edit_distance(a.bases(), b.bases(), 10), Some(full));
        assert_eq!(bounded_edit_distance(a.bases(), b.bases(), full - 1), None);
        assert_eq!(bounded_edit_distance(s("AAAA").bases(), s("AAAAAAAA").bases(), 3), None);
    }

    #[test]
    fn recovers_shuffled_noisy_clusters_exactly() {
        let mut rng = StdRng::seed_from_u64(7);
        let strands: Vec<Vec<Nucleotide>> = (0..60).map(|_| random_strand(&mut rng, 110)).collect();
        let mut reads = Vec::new();
        let mut truth = Vec::new();
        for (id, s) in strands.iter().enumerate() {
            for _ in 0..12 {
                reads.push(noisy(&mut rng, s, 0.02));
                truth.push(id);
            }
        }
        // Shuffle reads and labels together.
        for i in (1..reads.len()).rev() {
            let j = rng.gen_range(0..=i);
            reads.swap(i, j);
            truth.swap(i, j);
        }
        let clusters = cluster_reads(&reads, &ClusterConfig::default());
        assert_eq!(clusters.len(), 60, "sizes: {:?}", clusters.iter().map(Vec::len).collect::<Vec<_>>());
        for c in &clusters {
            assert!(c.iter().all(|&i| truth[i] == truth[c[0]]), "a cluster mixed two strands");
        }
    }

    #[test]
    fn unrelated_reads_stay_apart() {
        let mut rng = StdRng::seed_from_u64(11);
        let reads: Vec<DnaStrand> = (0..200).map(|_| DnaStrand::new(random_strand(&mut rng, 110))).collect();
        let clusters = cluster_reads(&reads, &ClusterConfig::default());
        assert_eq!(clusters.len(), 200);
    }

    #[test]
    fn every_read_lands_in_exactly_one_cluster() {
        let mut rng = StdRng::seed_from_u64(3);
        let base = random_strand(&mut rng, 100);
        // About 7.5% combined error per read, MinION-like.
        let reads: Vec<DnaStrand> = (0..30).map(|_| noisy(&mut rng, &base, 0.025)).collect();
        let clusters = cluster_reads(&reads, &ClusterConfig::default());
        let mut seen: Vec<usize> = clusters.concat();
        seen.sort_unstable();
        assert_eq!(seen, (0..30).collect::<Vec<_>>());
        assert!(clusters[0].len() >= 28, "largest cluster {}", clusters[0].len());
    }
}
