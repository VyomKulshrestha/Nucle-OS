//! # Self-Describing Archive Header
//!
//! A DNA archive read back in fifty years arrives as a tube of molecules,
//! with no software configuration beside it. This module writes a small
//! header into the pool itself so that a reader holding only the strands
//! can learn how the rest were written.
//!
//! The layout follows the two-level idea the DNA Data Storage Alliance
//! published in its first specifications (2024): a tiny "sector zero" in an
//! encoding simple enough to decode from a written description, which says
//! how to read a larger "sector one" that holds the file table. It is
//! modelled on that idea, not an implementation of the Alliance's byte
//! layout.
//!
//! Both sectors use one fixed code, chosen to be re-implementable from this
//! paragraph alone:
//!
//! 1. Bytes are XORed with a whitening stream: `x = 0x6E75636C`, then for
//!    each byte `x = x * 1664525 + 1013904223 (mod 2^32)` and the byte is
//!    XORed with `x >> 24`. This keeps GC content near 50%.
//! 2. Each byte becomes six base-3 digits, most significant first.
//! 3. Each digit `t` picks the `t`-th base, in the order A, C, G, T, among
//!    the three bases that differ from the previous base (the first strand
//!    base follows an imagined `A`). No base can repeat, so the header never
//!    contains a homopolymer.
//!
//! Every header strand starts with a two-byte tag (`NU` for sector zero,
//! `N1` for sector one) and ends with a CRC-16/CCITT of everything before
//! it, so a reader can find header strands among data strands and reject
//! damaged copies. A strand may carry other bases before the header (a
//! primer, for instance); the decoder scans every offset.

use crate::base::{DnaStrand, Nucleotide};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const ORDER: [Nucleotide; 4] = [Nucleotide::A, Nucleotide::C, Nucleotide::G, Nucleotide::T];
const SECTOR_ZERO_TAG: [u8; 2] = *b"NU";
const SECTOR_ONE_TAG: [u8; 2] = *b"N1";
const FORMAT_VERSION: u8 = 1;

/// Codec ids used in the header.
pub const CODEC_TERNARY: u8 = 1;
pub const CODEC_FOUNTAIN: u8 = 2;
pub const CODEC_YINYANG: u8 = 3;
pub const CODEC_HEDGES: u8 = 4;

/// The header id for a codec's persisted name, 0 if unknown.
pub fn codec_id(name: &str) -> u8 {
    match name {
        "ternary" | "ternary-rotating-cipher" => CODEC_TERNARY,
        "fountain" | "dna-fountain" => CODEC_FOUNTAIN,
        "yin-yang" | "yinyang" => CODEC_YINYANG,
        "hedges" => CODEC_HEDGES,
        _ => 0,
    }
}

/// The persisted codec name for a header codec id.
pub fn codec_name(id: u8) -> &'static str {
    match id {
        CODEC_TERNARY => "ternary-rotating-cipher",
        CODEC_FOUNTAIN => "dna-fountain",
        CODEC_YINYANG => "yin-yang",
        CODEC_HEDGES => "hedges",
        _ => "unknown",
    }
}
/// Payload bytes carried by each sector-one strand.
const SECTOR_ONE_PAYLOAD: usize = 20;
/// Sector zero: tag(2) version(1) codec(1) strand_len(2) rs_parity(1)
/// sector_one_strands(1) crc(2).
const SECTOR_ZERO_BYTES: usize = 10;
/// Sector one: tag(2) index(1) total(1) len(1) payload(20) crc(2). The
/// payload bytes of all sector-one strands, in index order, form the file
/// table: a u16 file count, then per file a length-prefixed UTF-8 name,
/// codec (u8), size (u32), version (u16), data and parity strand counts
/// (u16 each) and four hash bytes, all little-endian.
const SECTOR_ONE_BYTES: usize = 5 + SECTOR_ONE_PAYLOAD + 2;

/// One stored file, as listed in the header's file table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    /// Codec of this file's strands (see [`ArchiveHeader::codec`]).
    pub codec: u8,
    pub size: u32,
    pub version: u16,
    pub data_strands: u16,
    pub parity_strands: u16,
    /// First four bytes of the file's content hash.
    pub hash_prefix: [u8; 4],
}

/// Everything a reader needs before decoding any data strand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveHeader {
    /// Default codec of the data strands: 1 ternary, 2 fountain,
    /// 3 yin-yang, 4 HEDGES. Each file entry also records its own.
    pub codec: u8,
    /// Nucleotides per data strand.
    pub strand_len: u16,
    /// Reed-Solomon parity strands per stripe.
    pub rs_parity: u8,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Error, PartialEq)]
pub enum BootstrapError {
    #[error("no intact sector-zero strand found")]
    NoSectorZero,
    #[error("sector one is incomplete: found {found} of {expected} strands")]
    IncompleteSectorOne { found: usize, expected: usize },
    #[error("file table is malformed: {0}")]
    BadFileTable(String),
    #[error("header does not fit: {0}")]
    TooLarge(String),
}

impl ArchiveHeader {
    /// The header as DNA: one sector-zero strand followed by the
    /// sector-one strands.
    pub fn to_strands(&self) -> Result<Vec<DnaStrand>, BootstrapError> {
        let table = self.file_table()?;
        let chunks: Vec<&[u8]> = if table.is_empty() { vec![&[][..]] } else { table.chunks(SECTOR_ONE_PAYLOAD).collect() };
        if chunks.len() > u8::MAX as usize {
            return Err(BootstrapError::TooLarge(format!("{} sector-one strands (max 255)", chunks.len())));
        }
        let mut zero = Vec::with_capacity(SECTOR_ZERO_BYTES);
        zero.extend_from_slice(&SECTOR_ZERO_TAG);
        zero.push(FORMAT_VERSION);
        zero.push(self.codec);
        zero.extend_from_slice(&self.strand_len.to_le_bytes());
        zero.push(self.rs_parity);
        zero.push(chunks.len() as u8);
        push_crc(&mut zero);
        let mut strands = vec![encode_bytes(&zero)];

        for (i, chunk) in chunks.iter().enumerate() {
            let mut s = Vec::with_capacity(SECTOR_ONE_BYTES);
            s.extend_from_slice(&SECTOR_ONE_TAG);
            s.push(i as u8);
            s.push(chunks.len() as u8);
            s.push(chunk.len() as u8);
            s.extend_from_slice(chunk);
            s.resize(5 + SECTOR_ONE_PAYLOAD, 0);
            push_crc(&mut s);
            strands.push(encode_bytes(&s));
        }
        Ok(strands)
    }

    /// Recover the header from any collection of strands or reads: data
    /// strands, duplicates, damaged copies and leading primer bases are all
    /// tolerated, as long as one intact copy of each header strand exists.
    pub fn from_strands(strands: &[DnaStrand]) -> Result<Self, BootstrapError> {
        let mut zero: Option<Vec<u8>> = None;
        let mut sector_one: Vec<Option<Vec<u8>>> = Vec::new();
        for strand in strands {
            let bases = strand.bases();
            for offset in 0..bases.len() {
                if zero.is_none() {
                    if let Some(bytes) = decode_at(bases, offset, SECTOR_ZERO_BYTES, &SECTOR_ZERO_TAG) {
                        zero = Some(bytes);
                        break;
                    }
                }
                if let Some(bytes) = decode_at(bases, offset, SECTOR_ONE_BYTES, &SECTOR_ONE_TAG) {
                    let (index, total) = (bytes[2] as usize, bytes[3] as usize);
                    if sector_one.len() < total {
                        sector_one.resize(total, None);
                    }
                    if index < sector_one.len() && sector_one[index].is_none() {
                        let len = (bytes[4] as usize).min(SECTOR_ONE_PAYLOAD);
                        sector_one[index] = Some(bytes[5..5 + len].to_vec());
                    }
                    break;
                }
            }
        }

        let zero = zero.ok_or(BootstrapError::NoSectorZero)?;
        let expected = zero[7] as usize;
        let found = sector_one.iter().take(expected).filter(|s| s.is_some()).count();
        if found < expected {
            return Err(BootstrapError::IncompleteSectorOne { found, expected });
        }
        let table: Vec<u8> = sector_one.into_iter().take(expected).flatten().flatten().collect();
        Ok(ArchiveHeader {
            codec: zero[3],
            strand_len: u16::from_le_bytes([zero[4], zero[5]]),
            rs_parity: zero[6],
            files: parse_file_table(&table)?,
        })
    }

    fn file_table(&self) -> Result<Vec<u8>, BootstrapError> {
        if self.files.len() > u16::MAX as usize {
            return Err(BootstrapError::TooLarge("more than 65,535 files".into()));
        }
        let mut out = (self.files.len() as u16).to_le_bytes().to_vec();
        for f in &self.files {
            let name = f.name.as_bytes();
            if name.len() > u8::MAX as usize {
                return Err(BootstrapError::TooLarge(format!("file name '{}' exceeds 255 bytes", f.name)));
            }
            out.push(name.len() as u8);
            out.extend_from_slice(name);
            out.push(f.codec);
            out.extend_from_slice(&f.size.to_le_bytes());
            out.extend_from_slice(&f.version.to_le_bytes());
            out.extend_from_slice(&f.data_strands.to_le_bytes());
            out.extend_from_slice(&f.parity_strands.to_le_bytes());
            out.extend_from_slice(&f.hash_prefix);
        }
        Ok(out)
    }
}

fn parse_file_table(table: &[u8]) -> Result<Vec<FileEntry>, BootstrapError> {
    let bad = |what: &str| BootstrapError::BadFileTable(what.to_string());
    let mut r = Reader { bytes: table, pos: 0 };
    let count = u16::from_le_bytes(r.take::<2>().ok_or_else(|| bad("missing file count"))?) as usize;
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.take::<1>().ok_or_else(|| bad("truncated name length"))?[0] as usize;
        let name = r.slice(len).ok_or_else(|| bad("truncated name"))?;
        let name = String::from_utf8(name.to_vec()).map_err(|_| bad("name is not UTF-8"))?;
        files.push(FileEntry {
            name,
            codec: r.take::<1>().ok_or_else(|| bad("truncated codec"))?[0],
            size: u32::from_le_bytes(r.take::<4>().ok_or_else(|| bad("truncated size"))?),
            version: u16::from_le_bytes(r.take::<2>().ok_or_else(|| bad("truncated version"))?),
            data_strands: u16::from_le_bytes(r.take::<2>().ok_or_else(|| bad("truncated data count"))?),
            parity_strands: u16::from_le_bytes(r.take::<2>().ok_or_else(|| bad("truncated parity count"))?),
            hash_prefix: r.take::<4>().ok_or_else(|| bad("truncated hash"))?,
        });
    }
    Ok(files)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn slice(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.bytes.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.slice(N).and_then(|s| s.try_into().ok())
    }
}

fn whiten(bytes: &mut [u8]) {
    let mut x: u32 = 0x6E75_636C;
    for b in bytes {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b ^= (x >> 24) as u8;
    }
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn push_crc(bytes: &mut Vec<u8>) {
    let crc = crc16(bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
}

fn next_base(prev: Nucleotide, trit: u8) -> Nucleotide {
    let mut k = 0;
    for &b in &ORDER {
        if b != prev {
            if k == trit {
                return b;
            }
            k += 1;
        }
    }
    unreachable!("trit is 0, 1 or 2")
}

fn trit_of(prev: Nucleotide, base: Nucleotide) -> Option<u8> {
    let mut k = 0;
    for &b in &ORDER {
        if b != prev {
            if b == base {
                return Some(k);
            }
            k += 1;
        }
    }
    None
}

fn encode_bytes(bytes: &[u8]) -> DnaStrand {
    let mut whitened = bytes.to_vec();
    whiten(&mut whitened);
    let mut prev = Nucleotide::A;
    let mut bases = Vec::with_capacity(bytes.len() * 6);
    for &byte in &whitened {
        let mut digits = [0u8; 6];
        let mut v = byte as u32;
        for d in digits.iter_mut().rev() {
            *d = (v % 3) as u8;
            v /= 3;
        }
        for &t in &digits {
            prev = next_base(prev, t);
            bases.push(prev);
        }
    }
    DnaStrand::new(bases)
}

/// Decode `n_bytes` starting at `offset` and return them if the tag and
/// CRC check out. Every header was written starting from an imagined `A`,
/// so decoding starts from one too, whatever precedes the header in the
/// strand.
fn decode_at(bases: &[Nucleotide], offset: usize, n_bytes: usize, tag: &[u8; 2]) -> Option<Vec<u8>> {
    let needed = n_bytes * 6;
    if offset + needed > bases.len() {
        return None;
    }
    let mut prev = Nucleotide::A;
    let mut out = Vec::with_capacity(n_bytes);
    for chunk in bases[offset..offset + needed].chunks(6) {
        let mut v: u32 = 0;
        for &base in chunk {
            v = v * 3 + trit_of(prev, base)? as u32;
            prev = base;
        }
        out.push(u8::try_from(v).ok()?);
        // Reject early once the (whitened) tag cannot match.
        if out.len() == 2 {
            let mut t = out.clone();
            whiten(&mut t);
            if &t[..] != tag {
                return None;
            }
        }
    }
    whiten(&mut out);
    let body = out.len() - 2;
    if crc16(&out[..body]).to_le_bytes() != out[body..] {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> ArchiveHeader {
        ArchiveHeader {
            codec: 1,
            strand_len: 150,
            rs_parity: 4,
            files: vec![
                FileEntry { name: "vault/patient_records_2026.csv".into(), codec: 1, size: 109, version: 1, data_strands: 4, parity_strands: 4, hash_prefix: [0xde, 0xad, 0xbe, 0xef] },
                FileEntry { name: "vault/genome.fasta".into(), codec: 2, size: 1_048_576, version: 3, data_strands: 36_000, parity_strands: 4_000, hash_prefix: [1, 2, 3, 4] },
            ],
        }
    }

    #[test]
    fn round_trips_through_dna() {
        let h = header();
        let strands = h.to_strands().unwrap();
        assert_eq!(ArchiveHeader::from_strands(&strands).unwrap(), h);
    }

    #[test]
    fn header_strands_have_no_homopolymers_and_balanced_gc() {
        for s in header().to_strands().unwrap() {
            assert!(s.bases().windows(2).all(|w| w[0] != w[1]), "homopolymer in {s}");
            let gc = s.bases().iter().filter(|b| matches!(b, Nucleotide::G | Nucleotide::C)).count() as f64 / s.len() as f64;
            assert!((0.3..=0.7).contains(&gc), "gc {gc} in {s}");
        }
    }

    #[test]
    fn found_among_shuffled_data_strands_with_primers_and_damaged_copies() {
        let h = header();
        let mut pool: Vec<DnaStrand> = (0..50).map(|i| DnaStrand::from_str(&"ACGTTGCA".repeat(10 + i % 5)).unwrap()).collect();
        // The primer ends in G, not the imagined A the header starts from.
        let primer = DnaStrand::from_str("GATTACAGATTACG").unwrap();
        for (i, s) in h.to_strands().unwrap().into_iter().enumerate() {
            // A damaged copy first (one substitution), then an intact one
            // behind a primer.
            let mut damaged = s.bases().to_vec();
            damaged[7] = if damaged[7] == Nucleotide::A { Nucleotide::C } else { Nucleotide::A };
            pool.insert(i * 7 % pool.len(), DnaStrand::new(damaged));
            let mut tagged = primer.bases().to_vec();
            tagged.extend_from_slice(s.bases());
            pool.push(DnaStrand::new(tagged));
        }
        assert_eq!(ArchiveHeader::from_strands(&pool).unwrap(), h);
    }

    #[test]
    fn reports_what_is_missing() {
        let strands = header().to_strands().unwrap();
        assert_eq!(ArchiveHeader::from_strands(&strands[1..]), Err(BootstrapError::NoSectorZero));
        let missing_one: Vec<DnaStrand> = strands.iter().enumerate().filter(|(i, _)| *i != 2).map(|(_, s)| s.clone()).collect();
        assert!(matches!(
            ArchiveHeader::from_strands(&missing_one),
            Err(BootstrapError::IncompleteSectorOne { found, expected }) if found + 1 == expected
        ));
    }

    #[test]
    fn empty_archive_still_has_a_header() {
        let h = ArchiveHeader { codec: 3, strand_len: 120, rs_parity: 2, files: vec![] };
        assert_eq!(ArchiveHeader::from_strands(&h.to_strands().unwrap()).unwrap(), h);
    }

    #[test]
    fn whitening_and_crc_are_the_documented_ones() {
        // CRC-16/CCITT-FALSE check value.
        assert_eq!(crc16(b"123456789"), 0x29B1);
        let mut zeros = [0u8; 3];
        whiten(&mut zeros);
        let mut x: u32 = 0x6E75_636C;
        let expect: Vec<u8> = (0..3)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        assert_eq!(zeros.to_vec(), expect);
    }
}
