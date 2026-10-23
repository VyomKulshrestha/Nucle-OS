//! # HEDGES-style indel-correcting codec
//!
//! Press, Hawkins, Jones, Schaub and Finkelstein, "HEDGES error-correcting
//! code for DNA storage corrects indels and allows sequence constraints",
//! PNAS 117(31), 2020. The other codecs here need consensus over many
//! reads before they can decode a strand, because a single insertion or
//! deletion shifts everything after it. HEDGES decodes a strand from one
//! noisy read.
//!
//! **Encoding.** Each base carries one payload bit. The base for bit `i` is
//! chosen from the three bases that differ from the previous base (so the
//! code never writes a homopolymer), in an order fixed by a hash of a
//! secret salt, the bit position and the previous `context_bits` bits. The
//! bit picks the first or second base of that order. Because every base
//! depends on the bits before it, a wrong guess about any bit makes the
//! bases that follow stop matching the read.
//!
//! **Decoding.** A best-first (A*) search over states (bits decoded, read
//! bases consumed, recent bits, previous base). From each state it tries
//! both values of the next bit as a match or substitution, both as a
//! deletion, and an insertion of the next read base. Each edit costs about
//! `-ln(probability)` of that edit on the channel ([`EditCosts`]), so the
//! cheapest path is the most likely one, not merely the one with the fewest
//! edits. The lower bound `|read bases left - bits left|` times the cheaper
//! indel cost keeps the search admissible. When the cheapest path fails the
//! strand's CRC, the search keeps going and tries the next-cheapest
//! complete path (list decoding), up to [`HedgesConfig::max_candidates`].
//!
//! **Strands.** Each strand carries a 2-byte index, `payload_bytes` of
//! data and a CRC-8, so a read that decodes to the wrong bits is rejected
//! rather than trusted. Lost or rejected strands are erasures for an outer
//! code such as the Reed-Solomon layer the VFS puts around the other codecs.
//!
//! This is a rate-1/2 variant (one bit per base) with the homopolymer
//! constraint built into the base choice; the published code also offers
//! other rates and a GC-window constraint.

use crate::base::{DnaCodec, DnaError, DnaStrand, Nucleotide, StrandCollection};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

const ORDER: [Nucleotide; 4] = [Nucleotide::A, Nucleotide::C, Nucleotide::G, Nucleotide::T];

/// The six orderings of three candidate bases.
const PERMUTATIONS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];

/// Search cost of each edit, about `-10 ln(p)` for an edit of probability
/// `p` per base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditCosts {
    pub substitution: u32,
    pub insertion: u32,
    pub deletion: u32,
}

impl EditCosts {
    /// Costs for a channel with these per-base edit probabilities.
    pub fn for_channel(substitution: f64, insertion: f64, deletion: f64) -> Self {
        let cost = |p: f64| (-10.0 * p.clamp(1e-9, 0.5).ln()).round().max(1.0) as u32;
        Self { substitution: cost(substitution), insertion: cost(insertion), deletion: cost(deletion) }
    }
}

impl Default for EditCosts {
    /// The low-fidelity array pool measured on ERP173549 (run ERR15127744):
    /// substitution 0.98%, insertion 0.10%, deletion 0.43% per base.
    fn default() -> Self {
        Self::for_channel(0.0098, 0.0010, 0.0043)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HedgesConfig {
    /// Data bytes per strand (strand length is `(payload_bytes + 3) * 8`).
    pub payload_bytes: usize,
    /// How many previous bits feed the hash.
    pub context_bits: u32,
    pub salt: u64,
    /// Search states expanded per read before giving up on it.
    pub max_expansions: usize,
    /// Complete decodes tried per read before giving up when each fails
    /// its CRC.
    pub max_candidates: usize,
    pub costs: EditCosts,
}

impl Default for HedgesConfig {
    fn default() -> Self {
        Self {
            payload_bytes: 16,
            context_bits: 12,
            salt: 0x4845_4447_4553,
            max_expansions: 400_000,
            max_candidates: 16,
            costs: EditCosts::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HedgesCodec {
    config: HedgesConfig,
}

impl HedgesCodec {
    pub fn new(config: HedgesConfig) -> Self {
        Self { config }
    }

    pub fn strand_len(&self) -> usize {
        (self.config.payload_bytes + 3) * 8
    }

    fn mask(&self) -> u64 {
        (1u64 << self.config.context_bits) - 1
    }

    fn choose(&self, i: usize, history: u64, prev: Nucleotide, bit: u8) -> Nucleotide {
        let h = mix(self.config.salt ^ ((i as u64) << 40) ^ (history & self.mask()));
        let mut candidates = [Nucleotide::A; 3];
        let mut k = 0;
        for &b in &ORDER {
            if b != prev {
                candidates[k] = b;
                k += 1;
            }
        }
        candidates[PERMUTATIONS[(h % 6) as usize][bit as usize]]
    }

    /// The bases for `bits` (each 0 or 1).
    pub fn encode_bits(&self, bits: &[u8]) -> Vec<Nucleotide> {
        let mut prev = Nucleotide::A;
        let mut history = 0u64;
        bits.iter()
            .enumerate()
            .map(|(i, &b)| {
                let base = self.choose(i, history, prev, b);
                history = (history << 1) | b as u64;
                prev = base;
                base
            })
            .collect()
    }

    /// The `n_bits` most consistent with `read`, allowing substitutions,
    /// insertions and deletions, with the path's cost; `None` if the search
    /// exceeds `max_expansions`.
    pub fn decode_bits(&self, read: &[Nucleotide], n_bits: usize) -> Option<(Vec<u8>, u32)> {
        self.decode_bits_where(read, n_bits, |_| true)
    }

    /// Like [`Self::decode_bits`], but keeps searching past complete paths
    /// that `accept` rejects, returning the cheapest accepted one (at most
    /// `max_candidates` complete paths are tried).
    pub fn decode_bits_where(
        &self,
        read: &[Nucleotide],
        n_bits: usize,
        mut accept: impl FnMut(&[u8]) -> bool,
    ) -> Option<(Vec<u8>, u32)> {
        let costs = self.config.costs;
        let indel = costs.insertion.min(costs.deletion);
        let mut candidates = 0usize;
        #[derive(Clone, Copy)]
        struct Node {
            i: u32,
            j: u32,
            history: u64,
            prev: Nucleotide,
            cost: u32,
            parent: u32,
            bit: u8,
        }
        let len = read.len();
        let bound = |i: usize, j: usize| ((len - j) as i64 - (n_bits - i) as i64).unsigned_abs() as u32 * indel;
        let mut arena: Vec<Node> = vec![Node { i: 0, j: 0, history: 0, prev: Nucleotide::A, cost: 0, parent: u32::MAX, bit: 2 }];
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((bound(0, 0), 0u32)));
        let mut best: HashMap<(u32, u32, u64, u8), u32> = HashMap::new();
        let mut expansions = 0usize;

        while let Some(Reverse((_, idx))) = heap.pop() {
            let node = arena[idx as usize];
            let (i, j) = (node.i as usize, node.j as usize);
            if i == n_bits && j == len {
                let mut bits = vec![0u8; n_bits];
                let mut k = idx;
                while k != u32::MAX {
                    let n = arena[k as usize];
                    if n.bit < 2 {
                        bits[n.i as usize - 1] = n.bit;
                    }
                    k = n.parent;
                }
                if accept(&bits) {
                    return Some((bits, node.cost));
                }
                candidates += 1;
                if candidates >= self.config.max_candidates {
                    return None;
                }
                continue;
            }
            let key = (node.i, node.j, node.history & self.mask(), node.prev as u8);
            if best.get(&key).is_some_and(|&c| c < node.cost) {
                continue;
            }
            expansions += 1;
            if expansions > self.config.max_expansions {
                return None;
            }

            let mut push = |arena: &mut Vec<Node>, heap: &mut BinaryHeap<Reverse<(u32, u32)>>, n: Node| {
                let key = (n.i, n.j, n.history & self.mask(), n.prev as u8);
                let goal = n.i as usize == n_bits && n.j as usize == len;
                if !goal && best.get(&key).is_some_and(|&c| c <= n.cost) {
                    return;
                }
                best.insert(key, n.cost);
                let f = n.cost + bound(n.i as usize, n.j as usize);
                arena.push(n);
                heap.push(Reverse((f, arena.len() as u32 - 1)));
            };

            // An extra base in the read.
            if j < len {
                push(&mut arena, &mut heap, Node { j: node.j + 1, cost: node.cost + costs.insertion, parent: idx, bit: 2, ..node });
            }
            if i < n_bits {
                for b in 0..2u8 {
                    let expected = self.choose(i, node.history, node.prev, b);
                    let history = (node.history << 1) | b as u64;
                    // Read base matches or was substituted.
                    if j < len {
                        let cost = node.cost + if read[j] != expected { costs.substitution } else { 0 };
                        push(&mut arena, &mut heap, Node { i: node.i + 1, j: node.j + 1, history, prev: expected, cost, parent: idx, bit: b });
                    }
                    // The written base is missing from the read.
                    push(&mut arena, &mut heap, Node { i: node.i + 1, j: node.j, history, prev: expected, cost: node.cost + costs.deletion, parent: idx, bit: b });
                }
            }
        }
        None
    }

    /// Decode one read into (strand index, payload), or `None` if it does
    /// not decode or its CRC fails.
    pub fn decode_read(&self, read: &DnaStrand) -> Option<(u16, Vec<u8>)> {
        let n_bits = self.strand_len();
        let crc_ok = |bits: &[u8]| {
            let bytes = bits_to_bytes(bits);
            let body = bytes.len() - 1;
            crc8(&bytes[..body]) == bytes[body]
        };
        let (bits, _) = self.decode_bits_where(read.bases(), n_bits, crc_ok)?;
        let bytes = bits_to_bytes(&bits);
        let body = bytes.len() - 1;
        Some((u16::from_be_bytes([bytes[0], bytes[1]]), bytes[2..body].to_vec()))
    }
}

impl DnaCodec for HedgesCodec {
    fn name(&self) -> &str {
        "hedges"
    }

    fn encode(&self, data: &[u8]) -> Result<StrandCollection, DnaError> {
        let p = self.config.payload_bytes;
        let chunks = data.len().div_ceil(p).max(1);
        if chunks > u16::MAX as usize + 1 {
            return Err(DnaError::EncodingError(format!("{} strands exceed the 16-bit strand index", chunks)));
        }
        let strands = (0..chunks)
            .map(|k| {
                let mut bytes = (k as u16).to_be_bytes().to_vec();
                let chunk = &data[(k * p).min(data.len())..((k + 1) * p).min(data.len())];
                bytes.extend_from_slice(chunk);
                bytes.resize(2 + p, 0);
                bytes.push(crc8(&bytes));
                DnaStrand::new(self.encode_bits(&bytes_to_bits(&bytes)))
            })
            .collect();
        Ok(StrandCollection::from_strands(strands, data.len()))
    }

    fn decode(&self, strands: &StrandCollection) -> Result<Vec<u8>, DnaError> {
        let p = self.config.payload_bytes;
        let needed = strands.original_size.div_ceil(p).max(1);
        // A CRC-8 passes about one wrong decode in 256, so when several
        // reads cover a strand the most common payload wins.
        let mut votes: Vec<HashMap<Vec<u8>, usize>> = vec![HashMap::new(); needed];
        for read in &strands.strands {
            if let Some((index, payload)) = self.decode_read(read) {
                if let Some(slot) = votes.get_mut(index as usize) {
                    *slot.entry(payload).or_insert(0) += 1;
                }
            }
        }
        let missing: Vec<usize> = votes.iter().enumerate().filter(|(_, v)| v.is_empty()).map(|(k, _)| k).collect();
        if !missing.is_empty() {
            return Err(DnaError::DecodingError(format!("{} of {} strands did not decode: {:?}", missing.len(), needed, missing)));
        }
        let mut out: Vec<u8> = votes
            .into_iter()
            .flat_map(|v| v.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(payload, _)| payload).unwrap_or_default())
            .collect();
        out.truncate(strands.original_size);
        Ok(out)
    }
}

fn bytes_to_bits(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|&b| (0..8).rev().map(move |k| (b >> k) & 1)).collect()
}

fn bits_to_bytes(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8).map(|c| c.iter().fold(0u8, |acc, &b| (acc << 1) | b)).collect()
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read with independent substitutions, insertions and deletions at
    /// `rate` each, from a small deterministic generator.
    fn corrupt(bases: &[Nucleotide], rate: f64, seed: u64) -> Vec<Nucleotide> {
        let mut x = seed;
        let mut next = || {
            x = mix(x);
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut out = Vec::new();
        for &b in bases {
            let r = next();
            if r < rate {
                continue;
            }
            if r < 2.0 * rate {
                out.push(ORDER[((next() * 4.0) as usize).min(3)]);
            } else {
                out.push(b);
            }
            if next() < rate {
                out.push(ORDER[((next() * 4.0) as usize).min(3)]);
            }
        }
        out
    }

    #[test]
    fn round_trips_and_never_writes_a_homopolymer() {
        let codec = HedgesCodec::new(HedgesConfig::default());
        let data: Vec<u8> = (0..200u32).map(|i| (i * 37 % 256) as u8).collect();
        let encoded = codec.encode(&data).unwrap();
        assert_eq!(encoded.strands.len(), 13);
        for s in &encoded.strands {
            assert_eq!(s.len(), codec.strand_len());
            assert!(s.bases().windows(2).all(|w| w[0] != w[1]));
        }
        assert_eq!(codec.decode(&encoded).unwrap(), data);
    }

    #[test]
    fn decodes_single_reads_with_indels() {
        let codec = HedgesCodec::new(HedgesConfig { costs: EditCosts::for_channel(0.01, 0.01, 0.01), ..HedgesConfig::default() });
        let data: Vec<u8> = (0..16 * 40).map(|i| (i * 7 + 3) as u8).collect();
        let encoded = codec.encode(&data).unwrap();
        // About 1% substitutions, 1% insertions and 1% deletions per base:
        // several indels in most 152 nt reads.
        let reads: Vec<DnaStrand> = encoded
            .strands
            .iter()
            .enumerate()
            .map(|(k, s)| DnaStrand::new(corrupt(s.bases(), 0.01, k as u64 + 1)))
            .collect();
        let decoded = reads.iter().filter(|r| codec.decode_read(r).is_some()).count();
        assert!(decoded >= 38, "{decoded} of 40 single reads decoded");
        let with_indels = reads.iter().zip(&encoded.strands).filter(|(r, s)| r.len() != s.len()).count();
        assert!(with_indels >= 15, "the channel should shift most reads: {with_indels}");
    }

    #[test]
    fn a_wrong_decode_is_rejected_not_returned() {
        let codec = HedgesCodec::new(HedgesConfig::default());
        let encoded = codec.encode(&[42u8; 16]).unwrap();
        // Garbage of the right length must not decode to a valid strand.
        let junk = DnaStrand::new((0..codec.strand_len()).map(|k| ORDER[k * 7 % 4]).collect());
        assert!(codec.decode_read(&junk).is_none());
        // The real strand still decodes.
        assert_eq!(codec.decode_read(&encoded.strands[0]).unwrap().1, vec![42u8; 16]);
    }

    #[test]
    fn missing_strands_are_reported() {
        let codec = HedgesCodec::new(HedgesConfig::default());
        let mut encoded = codec.encode(&[1u8; 40]).unwrap();
        encoded.strands.remove(1);
        let err = codec.decode(&encoded).unwrap_err();
        assert!(err.to_string().contains("1 of 3 strands did not decode"), "{err}");
    }

    #[test]
    fn the_most_common_payload_wins_when_reads_disagree() {
        let codec = HedgesCodec::new(HedgesConfig::default());
        let truth = codec.encode(&[7u8; 16]).unwrap();
        // A valid strand for the same index with other data stands in for
        // a read whose wrong decode slipped past the CRC; it arrives first.
        let impostor = codec.encode(&[9u8; 16]).unwrap().strands[0].clone();
        let mut reads = truth.clone();
        reads.strands = vec![impostor, truth.strands[0].clone(), truth.strands[0].clone()];
        assert_eq!(codec.decode(&reads).unwrap(), vec![7u8; 16]);
    }

    #[test]
    fn bits_bytes_and_crc_helpers() {
        assert_eq!(bits_to_bytes(&bytes_to_bits(&[0xA5, 0x01])), vec![0xA5, 0x01]);
        // CRC-8 (poly 0x07) check value.
        assert_eq!(crc8(b"123456789"), 0xF4);
    }
}
