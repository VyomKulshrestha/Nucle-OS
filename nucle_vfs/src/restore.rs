//! Files rebuilt from sequencing reads alone, with no pool directory or
//! catalog: what comes back from a sequencer after a pool was synthesized
//! from an order sheet (`nucle hardware order`).
//!
//! The archive header strands say which files the pool holds, with each
//! file's codec, size, strand counts and the first bytes of its hash. The
//! primer library is generated from a fixed seed, so a fresh library holds
//! the same pairs the writing pool gave its files, and each pair picks its
//! file's reads out of the rest. A file's reads are then decoded as
//! [`crate::syscall::NucleOS`] decodes them, and the result is kept only if
//! its hash matches the header's.
//!
//! HEDGES files are rebuilt: each read carries its strand's index, so the
//! reads sort themselves. The other codecs need the strand order the pool
//! directory keeps and are reported as not rebuilt.

use crate::syscall::{hedges_rebuild_data_strands, HEDGES_PER_READ_EXPANSIONS, MIN_PRIMER_CAPACITY, PRIMER_LENGTH, PRIMER_SEED};
use nucle_codec::base::{DnaCodec, DnaStrand, StrandCollection};
use nucle_codec::bootstrap::{codec_name, ArchiveHeader, FileEntry, CODEC_HEDGES};
use nucle_codec::hedges::{HedgesCodec, HedgesConfig};
use nucle_ecc::cluster::{cluster_reads, ClusterConfig};
use nucle_ecc::consensus::{build_consensus, build_consensus_with_length};
use nucle_index::primer::{PrimerLibrary, PrimerPair};
use sha2::{Digest, Sha256};

/// Reads tried one by one before deciding whether single reads decode.
const HEDGES_TRIAL_READS: usize = 8;

/// One file the header lists, and what became of it.
#[derive(Debug, Clone)]
pub struct RestoredFile {
    pub name: String,
    pub size: usize,
    pub codec: &'static str,
    /// Reads that carried the primer pair the file was found under.
    pub reads: usize,
    /// The file's bytes, when they decoded to the header's hash.
    pub bytes: Option<Vec<u8>>,
    /// Why there are no bytes.
    pub problem: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub reads: usize,
    /// Reads left once the outer primer sites were found and cut off.
    pub reads_inside_flanks: usize,
    pub header: ArchiveHeader,
    pub files: Vec<RestoredFile>,
}

/// Rebuilds every file the reads' header strands list. `flanks`, when the
/// order carried one outer primer pair (`--flank`), is cut off each read
/// first; reads without it are dropped.
pub fn restore_from_reads(reads: &[DnaStrand], flanks: Option<&PrimerPair>) -> Result<RestoreReport, String> {
    let inner: Vec<DnaStrand> = match flanks {
        Some(pair) => reads.iter().filter_map(|r| pair.untag_strand(r)).collect(),
        None => reads.to_vec(),
    };
    if inner.is_empty() {
        return Err(format!("none of the {} reads carried the primer sites given", reads.len()));
    }
    let header = read_header(&inner)?;

    let library = PrimerLibrary::generate(header.files.len().max(MIN_PRIMER_CAPACITY), PRIMER_LENGTH, PRIMER_SEED);
    let mut groups: Vec<Vec<DnaStrand>> = vec![Vec::new(); library.primers.len()];
    for read in &inner {
        if let Some((i, untagged)) = library.primers.iter().enumerate().find_map(|(i, p)| p.untag_strand(read).map(|u| (i, u))) {
            groups[i].push(untagged);
        }
    }

    let mut used = vec![false; groups.len()];
    let files = header
        .files
        .iter()
        .map(|entry| {
            let mut restored = RestoredFile {
                name: entry.name.clone(),
                size: entry.size as usize,
                codec: codec_name(entry.codec),
                reads: 0,
                bytes: None,
                problem: None,
            };
            if entry.codec != CODEC_HEDGES {
                restored.problem = Some(format!("{} strands do not carry their own order; only HEDGES files are rebuilt from reads", restored.codec));
                return restored;
            }
            // The pair the writing pool gave this file is one of the
            // library's; the hash says which.
            for (i, group) in groups.iter().enumerate() {
                if used[i] || group.is_empty() {
                    continue;
                }
                if let Some(bytes) = rebuild_hedges(group, entry, header.rs_parity as usize) {
                    used[i] = true;
                    restored.reads = group.len();
                    restored.bytes = Some(bytes);
                    return restored;
                }
            }
            restored.problem = Some("no primer pair's reads decoded to the header's hash".to_string());
            restored
        })
        .collect();

    Ok(RestoreReport { reads: reads.len(), reads_inside_flanks: inner.len(), header, files })
}

/// The header from the reads themselves, or failing that from the
/// consensus of each cluster of reads.
fn read_header(reads: &[DnaStrand]) -> Result<ArchiveHeader, String> {
    if let Ok(header) = ArchiveHeader::from_strands(reads) {
        return Ok(header);
    }
    // Header strands come in fixed lengths, whatever the files; polishing
    // each cluster's consensus to them recovers strands a plain vote of a
    // few noisy reads gets wrong.
    let empty = ArchiveHeader { codec: 0, strand_len: 0, rs_parity: 0, files: Vec::new() };
    let mut lengths: Vec<usize> = empty.to_strands().map(|s| s.iter().map(DnaStrand::len).collect()).unwrap_or_default();
    lengths.sort_unstable();
    lengths.dedup();
    let config = ClusterConfig::fitted(reads);
    let consensus: Vec<DnaStrand> = cluster_reads(reads, &config)
        .iter()
        .flat_map(|c| {
            let group: Vec<DnaStrand> = c.iter().take(64).map(|&i| reads[i].clone()).collect();
            let plain = build_consensus(&group).map(|r| r.sequence);
            let polished: Vec<DnaStrand> = lengths.iter().filter_map(|&len| build_consensus_with_length(&group, len).map(|r| r.sequence)).collect();
            plain.into_iter().chain(polished)
        })
        .collect();
    ArchiveHeader::from_strands(&consensus).map_err(|e| format!("no archive header in the reads or their consensus: {e}"))
}

/// A HEDGES file from its untagged reads, if they decode to its hash.
/// Every read names its strand's index; reads are sorted by it, voted as
/// the pool's own read path votes them, and Reed-Solomon fills the gaps.
fn rebuild_hedges(reads: &[DnaStrand], entry: &FileEntry, header_parity: usize) -> Option<Vec<u8>> {
    let (data, parity) = (entry.data_strands as usize, entry.parity_strands as usize);
    let codec = HedgesCodec::new(HedgesConfig::default());
    let indexer = HedgesCodec::new(HedgesConfig { max_expansions: HEDGES_PER_READ_EXPANSIONS, ..HedgesConfig::default() });
    let mut by_index: Vec<Vec<DnaStrand>> = vec![Vec::new(); data + parity];
    let mut place = |index: u16, group: &[DnaStrand]| {
        if let Some(slot) = by_index.get_mut(index as usize) {
            slot.extend_from_slice(group);
        }
    };
    // Single reads name their index when the channel is clean enough; a
    // trial of the first few says whether it is. Otherwise, and for any
    // read that does not decode, reads are clustered and each cluster's
    // consensus names the index for all its reads.
    let trial = reads.len().min(HEDGES_TRIAL_READS);
    let decoded_in_trial = reads[..trial].iter().filter(|r| indexer.decode_read(r).is_some()).count();
    let mut unplaced: Vec<DnaStrand> = Vec::new();
    if decoded_in_trial * 4 >= trial {
        for read in reads {
            match indexer.decode_read(read) {
                Some((index, _)) => place(index, std::slice::from_ref(read)),
                None => unplaced.push(read.clone()),
            }
        }
    } else {
        unplaced = reads.to_vec();
    }
    if !unplaced.is_empty() {
        for cluster in cluster_reads(&unplaced, &ClusterConfig::fitted(&unplaced)) {
            let group: Vec<DnaStrand> = cluster.iter().map(|&i| unplaced[i].clone()).collect();
            let sample: Vec<DnaStrand> = group.iter().take(64).cloned().collect();
            let Some(consensus) = build_consensus_with_length(&sample, codec.strand_len()) else { continue };
            if let Some((index, _)) = codec.decode_consensus(&consensus.sequence, &consensus.confidence) {
                place(index, &group);
            }
        }
    }
    let (data_groups, parity_groups) = by_index.split_at(data);
    // The header keeps the largest parity per stripe; a file stored with
    // less has a divisor of its own parity count.
    let mut per_stripe: Vec<usize> = (1..=parity).rev().filter(|p| parity % p == 0).collect();
    if header_parity > 0 && parity % header_parity == 0 {
        per_stripe.retain(|&p| p != header_parity);
        per_stripe.insert(0, header_parity);
    }
    if parity == 0 {
        per_stripe = vec![0];
    }
    per_stripe.into_iter().find_map(|p| {
        let strands = hedges_rebuild_data_strands(data_groups, parity_groups, p);
        if strands.len() != data {
            return None;
        }
        let bytes = codec.decode(&StrandCollection::from_strands(strands, entry.size as usize)).ok()?;
        (Sha256::digest(&bytes)[..4] == entry.hash_prefix).then_some(bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall::{CodecKind, NucleOS};
    use nucle_synth::noise::{NoiseEngine, SimulationConfig};
    use nucle_synth::profiles::HardwareProfile;

    /// `copies` reads of every strand through `profile`'s realistic channel.
    fn reads_of(strands: &[DnaStrand], profile: HardwareProfile, copies: u32, seed: u64) -> Vec<DnaStrand> {
        let collection = StrandCollection::from_strands(strands.to_vec(), 0);
        let result = NoiseEngine::new(SimulationConfig::realistic(profile, copies, seed)).simulate(&collection);
        result.pool.strands.into_iter().filter(|s| s.is_intact).map(|s| s.sequence).collect()
    }

    fn data(len: u32, salt: u32) -> Vec<u8> {
        (0..len).map(|i| ((i * 31 + salt * 17) % 251) as u8).collect()
    }

    #[test]
    fn a_hedges_file_comes_back_from_noisy_reads_alone() {
        let bytes = data(300, 1);
        let mut os = NucleOS::new(100);
        os.dna_write_with_codec("pilot/a.bin", &bytes, 4, CodecKind::Hedges).unwrap();
        let reads = reads_of(&os.strands_with_header().unwrap(), HardwareProfile::Illumina, 8, 7);
        let report = restore_from_reads(&reads, None).unwrap();
        assert_eq!(report.files.len(), 1);
        let file = &report.files[0];
        assert_eq!((file.name.as_str(), file.codec), ("pilot/a.bin", "hedges"));
        assert_eq!(file.bytes.as_deref(), Some(&bytes[..]), "{:?}", file.problem);
    }

    #[test]
    fn flanked_reads_of_two_files_each_find_their_own_primers() {
        let (a, b) = (data(200, 2), data(260, 3));
        let mut os = NucleOS::new(100);
        os.dna_write_with_codec("pilot/a.bin", &a, 4, CodecKind::Hedges).unwrap();
        os.dna_write_with_codec("pilot/b.bin", &b, 2, CodecKind::Hedges).unwrap();
        let flank = PrimerPair::new(
            "flank",
            DnaStrand::from_str("ACGTCAGTGCATGACTGACT").unwrap(),
            DnaStrand::from_str("TGCATCAGTCAGACTGCATG").unwrap(),
        );
        let ordered: Vec<DnaStrand> = os.strands_with_header().unwrap().iter().map(|s| flank.tag_strand(s)).collect();
        let reads = reads_of(&ordered, HardwareProfile::OxfordNanopore, 12, 11);
        let report = restore_from_reads(&reads, Some(&flank)).unwrap();
        let got: Vec<(&str, Option<&[u8]>)> = report.files.iter().map(|f| (f.name.as_str(), f.bytes.as_deref())).collect();
        assert_eq!(got, vec![("pilot/a.bin", Some(&a[..])), ("pilot/b.bin", Some(&b[..]))], "{:?}", report.files);
        // Without the flanks named, nothing parses as a header.
        assert!(restore_from_reads(&reads, None).is_err());
    }

    #[test]
    fn a_file_in_another_codec_is_listed_but_not_rebuilt() {
        let mut os = NucleOS::new(100);
        os.dna_write_with_codec("pilot/t.txt", b"ternary strands keep no order of their own", 2, CodecKind::Ternary).unwrap();
        let report = restore_from_reads(&os.strands_with_header().unwrap(), None).unwrap();
        let file = &report.files[0];
        assert!(file.bytes.is_none());
        assert!(file.problem.as_deref().unwrap().contains("only HEDGES"), "{:?}", file.problem);
    }
}
