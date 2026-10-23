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
/// and blind decoding recovered 94.70% of strands, against 94.65% with the
/// true labels. At 0.25, chains of near-threshold pairs merged distinct
/// strands (92.6% purity, 83.3% recovered). Raise it for noisier reads.
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

impl ClusterConfig {
    /// The default configuration with its merge threshold set from the
    /// reads themselves: 1.6 times the 90th percentile of
    /// [`mate_distances`], kept between 0.03 and 0.15. Clean reads get a
    /// tight threshold that keeps near-identical strands apart; noisy reads
    /// get room for their errors. On the three real datasets measured
    /// (Illumina at 0.16% and 1.7% raw error, MinION at 5.8%) this lands on
    /// the best of the fixed thresholds tried; see `docs/architecture.md`.
    pub fn fitted(reads: &[DnaStrand]) -> Self {
        let mut config = Self::default();
        let d = mate_distances(reads, &config, FITTED_SAMPLE);
        if !d.is_empty() {
            let q90 = d[((d.len() - 1) as f64 * 0.9).round() as usize];
            config.max_distance_fraction = (FITTED_SCALE * q90).clamp(FITTED_MIN, FITTED_MAX);
        }
        config
    }
}

/// Reads sampled, and the scale and bounds applied, by [`ClusterConfig::fitted`].
const FITTED_SAMPLE: usize = 2000;
const FITTED_SCALE: f64 = 1.6;
const FITTED_MIN: f64 = 0.03;
const FITTED_MAX: f64 = 0.15;

/// Distinct clusters tracked per LSH bucket; bounds the work a huge bucket
/// (a motif shared by most strands) can cause.
const MAX_BUCKET_REPRESENTATIVES: usize = 32;

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
            // A bucket can mix several strands (pools whose strands share
            // an index or motif put them in the same bucket). Check each
            // member against one representative of every cluster seen in
            // this bucket so far, so reads of one strand join even when
            // they are not adjacent in the bucket.
            let mut reps: Vec<usize> = Vec::new();
            for &m in members {
                let mut placed = false;
                for &r in &reps {
                    if find(&mut parent, r) == find(&mut parent, m) {
                        placed = true;
                        break;
                    }
                    if same_strand(&reads[r], &reads[m], config) {
                        union(&mut parent, r, m);
                        placed = true;
                        break;
                    }
                }
                if !placed && reps.len() < MAX_BUCKET_REPRESENTATIVES {
                    reps.push(m);
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

/// Candidate mates checked per sampled read in [`mate_distances`], most
/// shared LSH bands first.
const MAX_MATES: usize = 64;

/// Buckets larger than this (a motif many strands share) say nothing about
/// which reads are mates, so [`mate_distances`] ignores them.
const MAX_MATE_BUCKET: usize = 1024;

/// For up to `sample` reads spread evenly through `reads`, the edit
/// distance to the closest other read sharing an LSH bucket with it, as a
/// fraction of the longer read, sorted ascending. A read's closest mate is
/// almost always another read of the same strand, so this measures read
/// noise without labels. Reads with no candidate within 30% are skipped.
pub fn mate_distances(reads: &[DnaStrand], config: &ClusterConfig, sample: usize) -> Vec<f64> {
    let k = config.k.clamp(1, 16);
    let rows = config.rows.max(1);
    let bands = config.bands.max(1);
    let sketches: Vec<Vec<u64>> = reads.iter().map(|r| sketch(r.bases(), k, bands * rows, config.seed)).collect();
    let step = (reads.len() / sample.max(1)).max(1);
    let picked: Vec<usize> = (0..reads.len()).step_by(step).take(sample).collect();
    // Bands each candidate shares with each sampled read.
    let mut shared: Vec<HashMap<usize, u32>> = vec![HashMap::new(); picked.len()];
    for band in 0..bands {
        let key = |sk: &[u64]| sk[band * rows..(band + 1) * rows].iter().fold(band as u64, |acc, &v| mix(acc ^ v));
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, sk) in sketches.iter().enumerate().filter(|(_, sk)| !sk.is_empty()) {
            buckets.entry(key(sk)).or_default().push(i);
        }
        for (slot, &i) in picked.iter().enumerate().filter(|&(_, &i)| !sketches[i].is_empty()) {
            if let Some(members) = buckets.get(&key(&sketches[i])).filter(|m| m.len() <= MAX_MATE_BUCKET) {
                for &m in members.iter().filter(|&&m| m != i) {
                    *shared[slot].entry(m).or_insert(0) += 1;
                }
            }
        }
    }

    let mut out = Vec::with_capacity(picked.len());
    for (slot, &i) in picked.iter().enumerate() {
        let mut mates: Vec<(usize, u32)> = shared[slot].iter().map(|(&m, &n)| (m, n)).collect();
        mates.sort_unstable_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        let a = reads[i].bases();
        let mut limit = (0.3 * a.len() as f64) as usize;
        let mut best = None;
        for &(m, _) in mates.iter().take(MAX_MATES) {
            let b = reads[m].bases();
            if let Some(d) = bounded_edit_distance(a, b, limit) {
                best = Some(d as f64 / a.len().max(b.len()).max(1) as f64);
                if d == 0 {
                    break;
                }
                limit = d - 1;
            }
        }
        out.extend(best);
    }
    out.sort_by(f64::total_cmp);
    out
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

    /// `pairs` pairs of strands, the two in a pair 6 substitutions apart
    /// (5% of 120 nt), with `copies` reads of each at `rate` per error type.
    fn near_identical_pairs(rng: &mut StdRng, pairs: usize, copies: usize, rate: f64) -> (Vec<DnaStrand>, Vec<usize>) {
        let (mut reads, mut truth) = (Vec::new(), Vec::new());
        for p in 0..pairs {
            let a = random_strand(rng, 120);
            let mut b = a.clone();
            for k in 0..6 {
                let i = 10 + 18 * k;
                b[i] = Nucleotide::ALL[(Nucleotide::ALL.iter().position(|&n| n == b[i]).unwrap() + 1) % 4];
            }
            for (id, s) in [(2 * p, &a), (2 * p + 1, &b)] {
                for _ in 0..copies {
                    reads.push(noisy(rng, s, rate));
                    truth.push(id);
                }
            }
        }
        (reads, truth)
    }

    #[test]
    fn the_fitted_threshold_follows_read_noise() {
        let mut rng = StdRng::seed_from_u64(5);
        let (clean, _) = near_identical_pairs(&mut rng, 20, 10, 0.0005);
        assert_eq!(ClusterConfig::fitted(&clean).max_distance_fraction, FITTED_MIN);
        // About 6% combined error per read, MinION-like.
        let (noisy_reads, _) = near_identical_pairs(&mut rng, 20, 10, 0.02);
        let t = ClusterConfig::fitted(&noisy_reads).max_distance_fraction;
        assert!(t > 0.08 && t <= FITTED_MAX, "{t}");
        let d = mate_distances(&noisy_reads, &ClusterConfig::default(), 100);
        assert_eq!(d.len(), 100);
        assert!(d.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn a_fitted_threshold_keeps_near_identical_clean_strands_apart() {
        let mut rng = StdRng::seed_from_u64(9);
        let (reads, truth) = near_identical_pairs(&mut rng, 25, 10, 0.0005);
        let pure = |clusters: &[Vec<usize>]| clusters.iter().all(|c| c.iter().all(|&i| truth[i] == truth[c[0]]));
        // The nanopore default merges each pair.
        assert!(!pure(&cluster_reads(&reads, &ClusterConfig::default())));
        let fitted = cluster_reads(&reads, &ClusterConfig::fitted(&reads));
        assert!(pure(&fitted));
        assert_eq!(fitted.len(), 50, "sizes: {:?}", fitted.iter().map(Vec::len).collect::<Vec<_>>());
    }
}
