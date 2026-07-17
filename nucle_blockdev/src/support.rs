//! Shared encode+ECC / decode+ECC / noise-injection helpers used by both
//! backends -- the same real composition `nucle_vfs::syscall`'s
//! `dna_write_with_codec_impl`/`dna_read` uses at the file level (codec
//! encode → RS parity → noise → consensus vote → RS decode → codec
//! decode), minus primer tagging, since blocks are addressed by index,
//! not retrieved by searching primer-tagged strands.
//!
//! Coverage depth (multiple independently-noised copies per logical
//! strand, consensus-voted before Reed-Solomon) isn't optional here the
//! way it might look -- plain RS alone can only correct a fully-dropped
//! strand (an erasure) or a strand that's blindly wrong but still the
//! same length (Berlekamp-Welch). It can't correct an insertion/deletion,
//! since that changes the strand's byte length and desyncs RS's
//! fixed-width symbol alignment. Both `TwistBioscience` and
//! `OxfordNanopore` (the profiles the two backends use) are indel-heavy
//! by design (see their doc comments in `nucle_synth::profiles`), so
//! skipping consensus here would make "strong error-correction" a claim
//! this crate doesn't actually back up -- confirmed the hard way: an
//! earlier version of this module without coverage/consensus failed
//! every single read, deterministically, not occasionally.

use nucle_codec::base::{DnaCodec, DnaStrand, StrandCollection};
use nucle_codec::ternary::TernaryCodec;
use nucle_ecc::pipeline::consensus_then_rs_decode;
use nucle_ecc::reed_solomon::{ReedSolomon, RsConfig};
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;

/// A block's on-medium representation: the codec-encoded data strands
/// plus (if any redundancy was configured) Reed-Solomon parity strands,
/// each packed 4 bases per byte since parity symbols span the full
/// 0-255 range (see `DnaStrand::from_packed_bytes`). This is the single
/// "ground truth" copy each backend stores -- noisy read/write copies
/// are derived from it on demand via [`noisy_copies`], never stored
/// themselves.
#[derive(Clone)]
pub struct StoredBlock {
    pub data_strands: Vec<DnaStrand>,
    pub parity_strands: Vec<DnaStrand>,
    pub original_size: usize,
}

/// What a backend actually stores per block: one group of
/// (independently-noised, per [`noisy_copies`]) coverage copies per
/// logical data/parity strand, ready to be consensus-voted at read time.
#[derive(Clone)]
pub struct StoredGroups {
    pub data_groups: Vec<Vec<DnaStrand>>,
    pub parity_groups: Vec<Vec<DnaStrand>>,
    pub original_size: usize,
}

/// Codec-encode `data`, then compute `parity_count` Reed-Solomon parity
/// strands over it (skipped entirely if `parity_count == 0`).
pub fn encode(data: &[u8], parity_count: usize) -> Result<StoredBlock, String> {
    let codec = TernaryCodec::default_codec();
    let encoded = codec.encode(data).map_err(|e| format!("codec encode failed: {e}"))?;

    let mut parity_strands = Vec::new();
    if parity_count > 0 {
        let rs = ReedSolomon::new(RsConfig::new(parity_count));
        let strand_bytes: Vec<Vec<u8>> =
            encoded.strands.iter().map(|s| s.bases().iter().map(|n| n.to_bits()).collect()).collect();
        let parity_bytes = rs.encode_block(&strand_bytes).map_err(|e| format!("ECC encode failed: {e}"))?;
        parity_strands = parity_bytes.iter().map(|p| DnaStrand::from_packed_bytes(p)).collect();
    }

    Ok(StoredBlock { data_strands: encoded.strands, parity_strands, original_size: data.len() })
}

/// Runs `strands` through one noise profile, producing `coverage_depth`
/// independently-noised copies of *each* logical strand -- grouped one
/// `Vec` per original strand (zero to `coverage_depth` entries, since
/// any individual copy may be dropped entirely), exactly what
/// [`nucle_ecc::pipeline::consensus_then_rs_decode`] expects to vote
/// across. Reuses `nucle_synth::NoiseEngine` (the same engine
/// `nucle_vfs` applies at the file level) rather than reimplementing
/// error injection.
pub fn noisy_copies(strands: &[DnaStrand], profile: HardwareProfile, coverage_depth: u32, seed: u64) -> Vec<Vec<DnaStrand>> {
    if strands.is_empty() {
        return Vec::new();
    }
    if matches!(profile, HardwareProfile::Pristine) {
        return strands.iter().map(|s| vec![s.clone(); coverage_depth.max(1) as usize]).collect();
    }

    let collection = StrandCollection::from_strands(strands.to_vec(), 0);
    let config = SimulationConfig {
        seed,
        coverage_depth: coverage_depth.max(1),
        synthesis_profile: profile,
        sequencing_profile: HardwareProfile::Pristine,
        simulate_decay: false,
        decay_rate: 0.0,
        storage_time: 0.0,
    };
    let result = NoiseEngine::new(config).simulate(&collection);

    // `simulate` emits copies grouped strand-major, copy-minor (strand 0's
    // N copies, then strand 1's N copies, ...) -- see its own
    // implementation -- so chunking the flat output back into
    // fixed-size groups recovers per-strand grouping without needing
    // `simulate` to expose that structure itself.
    result
        .pool
        .strands
        .chunks(coverage_depth.max(1) as usize)
        .map(|chunk| chunk.iter().filter(|s| s.is_intact).map(|s| s.sequence.clone()).collect())
        .collect()
}

/// Applies fresh, independent noise to each of `copies` individually --
/// one noisy variant per input copy, not fanned out into more copies --
/// used for read-time noise on an *already*-multi-copy stored group
/// (e.g. the rewritable backend's "every read is a fresh pass through
/// the sensor"), as opposed to [`noisy_copies`]'s write-time fan-out
/// from a single ground-truth strand into several.
pub fn refresh_noise(copies: &[DnaStrand], profile: HardwareProfile, seed: u64) -> Vec<DnaStrand> {
    if copies.is_empty() || matches!(profile, HardwareProfile::Pristine) {
        return copies.to_vec();
    }
    let collection = StrandCollection::from_strands(copies.to_vec(), 0);
    let result = NoiseEngine::simulate_single_profile(&collection, profile, seed);
    result.pool.strands.into_iter().filter(|s| s.is_intact).map(|s| s.sequence).collect()
}

/// Consensus-votes each group of noisy copies, then Reed-Solomon
/// decodes and codec-decodes the result -- the real recovery pipeline,
/// not a simplified stand-in.
pub fn decode(
    data_groups: &[Vec<DnaStrand>],
    parity_groups: &[Vec<DnaStrand>],
    original_size: usize,
    parity_count: usize,
) -> Result<Vec<u8>, String> {
    let recovered_strands = consensus_then_rs_decode(data_groups, parity_groups, RsConfig::new(parity_count));
    let collection = StrandCollection::from_strands(recovered_strands, original_size);
    TernaryCodec::default_codec().decode(&collection).map_err(|e| format!("codec decode failed: {e}"))
}
