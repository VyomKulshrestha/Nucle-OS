//! Shared playground demo logic: the interactive codec benchmark and the
//! encode -> noise -> recovery pipeline visualizer.
//!
//! This crate has no I/O of its own (no HTTP server, no wasm-bindgen) so the
//! exact same logic backs both the native `nucle_playground` tiny_http
//! server and the `nucle_wasm` in-browser build, instead of the two
//! diverging over time.

use nucle_codec::base::{DnaCodec, DnaStrand, Nucleotide, StrandCollection};
use nucle_codec::benchmark::benchmark_codec;
use nucle_codec::fountain::{FountainCodec, FountainConfig};
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::yinyang::{YinYangCodec, YinYangConfig};
use nucle_ecc::pipeline::consensus_then_rs_decode;
use nucle_ecc::reed_solomon::{ReedSolomon, RsConfig};
use nucle_synth::noise::{NoiseEngine, SimulationConfig, SimulationResult};
use nucle_synth::profiles::HardwareProfile;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Feature 2: interactive codec benchmark
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct BenchmarkRequest {
    pub codec: String,
    pub profile: String,
    pub redundancy: usize,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkResponse {
    pub codec_name: String,
    pub input_size: usize,
    pub strand_count: usize,
    pub total_nucleotides: usize,
    pub bits_per_nucleotide: f64,
    pub avg_gc_content: f64,
    pub max_homopolymer: usize,
    pub homopolymer_violation_count: usize,
    pub constraint_violations: usize,
    pub roundtrip_ok: bool,
    pub gc_distribution: Vec<usize>,
    pub recovery_probability: f64,
    pub estimated_cost_usd: f64,
}

pub fn parse_hw_profile(profile: &str) -> Result<HardwareProfile, String> {
    match profile.to_lowercase().as_str() {
        "illumina" => Ok(HardwareProfile::Illumina),
        "nanopore" => Ok(HardwareProfile::OxfordNanopore),
        "twist" => Ok(HardwareProfile::TwistBioscience),
        "idt" => Ok(HardwareProfile::Idt),
        "column-synthesis" => Ok(HardwareProfile::ColumnSynthesis),
        "pristine" => Ok(HardwareProfile::Pristine),
        other => Err(format!("unknown profile '{}'", other)),
    }
}

/// Same lookup used by `nucle bench`/`nucle benchmark` in the core CLI —
/// a real domain constant (USD per base for that platform), not a guess.
pub fn profile_cost_per_base(profile: HardwareProfile) -> f64 {
    match profile {
        HardwareProfile::TwistBioscience => 0.00015,
        HardwareProfile::Illumina => 0.0001,
        HardwareProfile::OxfordNanopore => 0.00005,
        HardwareProfile::Idt => 0.00012,
        HardwareProfile::ColumnSynthesis => 0.00008,
        HardwareProfile::Pristine => 0.00001,
    }
}

pub fn make_codec(name: &str) -> Result<Box<dyn DnaCodec>, String> {
    match name.to_lowercase().as_str() {
        "ternary" => Ok(Box::new(TernaryCodec::new(TernaryConfig::no_overlap()))),
        "ternary-overlap" => Ok(Box::new(TernaryCodec::new(TernaryConfig::default()))),
        "yinyang" | "yin-yang" => Ok(Box::new(YinYangCodec::new(YinYangConfig::default()))),
        "fountain" => Ok(Box::new(FountainCodec::new(FountainConfig::default()))),
        other => Err(format!("unknown codec '{}'", other)),
    }
}

pub fn dna_to_bytes(strand: &DnaStrand) -> Vec<u8> {
    strand.bases().iter().map(|n| n.to_bits()).collect()
}

pub fn bytes_to_dna(bytes: &[u8]) -> DnaStrand {
    let bases: Vec<Nucleotide> = bytes.iter().filter_map(|&b| Nucleotide::from_bits(b).ok()).collect();
    DnaStrand::new(bases)
}

/// Real sequencing coverage assumed for the recovery estimate below --
/// matches what `nucle benchmark` and NucleScript's own `simulate under`
/// use elsewhere, so this interactive estimate reflects the real engine's
/// behavior rather than a simplified one-shot read.
const RECOVERY_COVERAGE_DEPTH: u32 = 10;

/// Group a noise-simulation result's surviving reads by which pre-coverage
/// logical strand each is a copy of (see `NoiseEngine::simulate`'s
/// copy-major loop order: dividing the flat `strand_id` by coverage depth
/// recovers the original index). Returns a dense `Vec` of length
/// `input_count`; a logical position with zero surviving reads is an empty
/// group, which `consensus_then_rs_decode` treats as an erasure.
fn grouped_reads(result: &SimulationResult, coverage_depth: u32, input_count: usize) -> Vec<Vec<DnaStrand>> {
    let coverage = coverage_depth.max(1) as u64;
    let mut groups: Vec<Vec<DnaStrand>> = vec![Vec::new(); input_count];
    for s in result.pool.strands.iter().filter(|s| s.is_intact && !s.is_truncated) {
        if let Some(group) = groups.get_mut((s.strand_id / coverage) as usize) {
            group.push(s.sequence.clone());
        }
    }
    groups
}

/// Monte-Carlo estimate of encode -> [+RS parity] -> noise -> consensus ->
/// RS recover roundtrip success — the redundancy slider actually changes
/// this number because higher redundancy means more RS parity strands are
/// available to reconstruct fully-dropped data strands, and because
/// consensus-voting across `RECOVERY_COVERAGE_DEPTH` simulated reads per
/// strand corrects substitution errors that RS alone cannot (RS is an
/// erasure decoder, not an error decoder — it only recovers a strand
/// that's entirely missing, never one that survived corrupted).
pub fn estimate_recovery_probability(
    codec: &dyn DnaCodec,
    data: &[u8],
    profile: HardwareProfile,
    redundancy: usize,
    trials: u64,
) -> f64 {
    if profile == HardwareProfile::Pristine {
        return 1.0;
    }
    let Ok(encoded) = codec.encode(data) else {
        return 0.0;
    };

    let parity_strands: Vec<DnaStrand> = if redundancy > 0 {
        let rs = ReedSolomon::new(RsConfig::new(redundancy));
        let strand_bytes: Vec<Vec<u8>> = encoded.strands.iter().map(dna_to_bytes).collect();
        rs.encode_block(&strand_bytes).unwrap_or_default().iter().map(|p| bytes_to_dna(p)).collect()
    } else {
        Vec::new()
    };

    let data_collection = StrandCollection::from_strands(encoded.strands.clone(), data.len());
    let parity_collection = StrandCollection::from_strands(parity_strands.clone(), 0);

    let mut successes = 0u64;
    for t in 0..trials {
        let config = SimulationConfig {
            seed: 9000 + t,
            coverage_depth: RECOVERY_COVERAGE_DEPTH,
            synthesis_profile: profile,
            sequencing_profile: profile,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        let data_sim = NoiseEngine::new(config.clone()).simulate(&data_collection);
        let data_groups = grouped_reads(&data_sim, RECOVERY_COVERAGE_DEPTH, encoded.strands.len());

        let recovered_strands = if redundancy > 0 {
            let parity_sim = NoiseEngine::new(config).simulate(&parity_collection);
            let parity_groups = grouped_reads(&parity_sim, RECOVERY_COVERAGE_DEPTH, parity_strands.len());
            consensus_then_rs_decode(&data_groups, &parity_groups, RsConfig::new(redundancy))
        } else {
            consensus_then_rs_decode(&data_groups, &[], RsConfig::new(0))
        };

        let collection = StrandCollection::from_strands(recovered_strands, data.len());
        if let Ok(decoded) = codec.decode(&collection) {
            if decoded == data {
                successes += 1;
            }
        }
    }
    successes as f64 / trials as f64
}

pub fn run_benchmark(req: BenchmarkRequest) -> Result<BenchmarkResponse, String> {
    let profile = parse_hw_profile(&req.profile)?;
    let codec = make_codec(&req.codec)?;
    let data = req.data.unwrap_or_else(|| {
        "The quick brown fox jumps over the lazy dog. NucleOS benchmarks all available DNA codecs.".to_string()
    });
    let data = data.as_bytes();
    if data.is_empty() {
        return Err("input data must not be empty".to_string());
    }

    let bench = benchmark_codec(codec.as_ref(), data).map_err(|e| e.to_string())?;
    let recovery_probability = estimate_recovery_probability(codec.as_ref(), data, profile, req.redundancy, 20);

    // Redundancy adds parity nucleotides on top of the data strands the
    // codec-only benchmark measured, so cost must account for them too.
    let parity_nucleotides = if req.redundancy > 0 {
        let encoded = codec.encode(data).map_err(|e| e.to_string())?;
        let rs = ReedSolomon::new(RsConfig::new(req.redundancy));
        let strand_bytes: Vec<Vec<u8>> = encoded.strands.iter().map(dna_to_bytes).collect();
        let parity_bytes = rs.encode_block(&strand_bytes).map_err(|e| e.to_string())?;
        parity_bytes.iter().map(|p| bytes_to_dna(p).len()).sum::<usize>()
    } else {
        0
    };
    let total_nucleotides = bench.total_nucleotides + parity_nucleotides;
    let estimated_cost_usd = total_nucleotides as f64 * profile_cost_per_base(profile);

    Ok(BenchmarkResponse {
        codec_name: bench.codec_name,
        input_size: bench.input_size,
        strand_count: bench.strand_count + if req.redundancy > 0 { req.redundancy } else { 0 },
        total_nucleotides,
        bits_per_nucleotide: bench.bits_per_nucleotide,
        avg_gc_content: bench.avg_gc_content,
        max_homopolymer: bench.max_homopolymer,
        homopolymer_violation_count: bench.homopolymer_violation_count,
        constraint_violations: bench.constraint_violations,
        roundtrip_ok: bench.roundtrip_ok,
        gc_distribution: bench.gc_distribution,
        recovery_probability,
        estimated_cost_usd,
    })
}

// ---------------------------------------------------------------------------
// Feature 3: pipeline visualization
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PipelineRequest {
    pub data: String,
    pub profile: String,
    #[serde(default = "default_redundancy")]
    pub redundancy: usize,
}

fn default_redundancy() -> usize {
    2
}

#[derive(Debug, Serialize)]
pub struct StrandView {
    pub index: usize,
    pub is_parity: bool,
    pub original: String,
    pub after_noise: String,
    pub corrupted: bool,
    pub dropped: bool,
    pub error_count: usize,
}

#[derive(Debug, Serialize)]
pub struct RecoveryView {
    pub attempted: bool,
    pub success: bool,
    pub recovered_preview: String,
    pub is_text: bool,
}

#[derive(Debug, Serialize)]
pub struct PipelineResponse {
    pub profile: String,
    pub redundancy: usize,
    pub original_size: usize,
    pub survival_rate: f64,
    pub avg_error_rate: f64,
    pub strands: Vec<StrandView>,
    pub recovery: RecoveryView,
}

pub fn run_pipeline_demo(req: PipelineRequest) -> Result<PipelineResponse, String> {
    let profile = parse_hw_profile(&req.profile)?;
    let data = req.data.as_bytes();
    if data.is_empty() {
        return Err("input data must not be empty".to_string());
    }
    if data.len() > 2000 {
        return Err("input capped at 2000 bytes for the visualizer -- try a smaller sample".to_string());
    }

    let codec = TernaryCodec::new(TernaryConfig::no_overlap());
    let encoded: StrandCollection = codec.encode(data).map_err(|e| e.to_string())?;
    let data_strand_count = encoded.strands.len();

    // Reed-Solomon parity strands, same construction nucle_vfs::dna_write uses.
    let mut all_strands: Vec<DnaStrand> = encoded.strands.clone();
    let parity_start = all_strands.len();
    if req.redundancy > 0 {
        let rs = ReedSolomon::new(RsConfig::new(req.redundancy));
        let strand_bytes: Vec<Vec<u8>> = encoded.strands.iter().map(dna_to_bytes).collect();
        let parity_bytes = rs.encode_block(&strand_bytes).map_err(|e| e.to_string())?;
        for parity in &parity_bytes {
            all_strands.push(bytes_to_dna(parity));
        }
    }

    let combined = StrandCollection::from_strands(all_strands.clone(), data.len());

    let config = SimulationConfig {
        seed: 42,
        coverage_depth: 1,
        synthesis_profile: profile,
        sequencing_profile: profile,
        simulate_decay: false,
        decay_rate: 0.0,
        storage_time: 0.0,
    };
    let sim_result = NoiseEngine::new(config).simulate(&combined);
    let survival_rate = sim_result.survival_rate();
    let avg_error_rate = sim_result.avg_error_rate();

    let strands: Vec<StrandView> = sim_result
        .pool
        .strands
        .iter()
        .enumerate()
        .map(|(i, s)| StrandView {
            index: i,
            is_parity: i >= parity_start,
            original: s.original.as_ref().map(|o| o.to_string()).unwrap_or_default(),
            after_noise: s.sequence.to_string(),
            corrupted: s.has_errors(),
            dropped: !s.is_intact,
            error_count: s.error_count.total(),
        })
        .collect();

    // Attempt real recovery: RS-decode using post-noise strands (dropped -> erasure), then codec-decode.
    let received: Vec<Option<Vec<u8>>> = sim_result
        .pool
        .strands
        .iter()
        .take(data_strand_count)
        .map(|s| if s.is_intact { Some(dna_to_bytes(&s.sequence)) } else { None })
        .collect();
    let parity_received: Vec<Vec<u8>> = sim_result
        .pool
        .strands
        .iter()
        .skip(data_strand_count)
        .filter(|s| s.is_intact)
        .map(|s| dna_to_bytes(&s.sequence))
        .collect();

    let recovery = if req.redundancy > 0 && !parity_received.is_empty() {
        let rs = ReedSolomon::new(RsConfig::new(req.redundancy));
        match rs.decode_block(&received, &parity_received) {
            Ok(recovered_bytes) => {
                let recovered_strands: Vec<DnaStrand> = recovered_bytes.iter().map(|b| bytes_to_dna(b)).collect();
                let collection = StrandCollection::from_strands(recovered_strands, data.len());
                attempt_final_decode(&codec, &collection)
            }
            Err(_) => RecoveryView { attempted: true, success: false, recovered_preview: String::new(), is_text: false },
        }
    } else {
        // No parity requested/available -- decode directly from whatever survived.
        let direct: Vec<DnaStrand> = sim_result.pool.to_strand_collection(data.len()).strands;
        let collection = StrandCollection::from_strands(direct, data.len());
        attempt_final_decode(&codec, &collection)
    };

    Ok(PipelineResponse {
        profile: req.profile,
        redundancy: req.redundancy,
        original_size: data.len(),
        survival_rate,
        avg_error_rate,
        strands,
        recovery,
    })
}

fn attempt_final_decode(codec: &TernaryCodec, collection: &StrandCollection) -> RecoveryView {
    match codec.decode(collection) {
        Ok(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(text) => RecoveryView { attempted: true, success: true, recovered_preview: text, is_text: true },
            Err(_) => RecoveryView {
                attempted: true,
                success: true,
                recovered_preview: bytes.iter().map(|b| format!("{:02x}", b)).collect(),
                is_text: false,
            },
        },
        Err(_) => RecoveryView { attempted: true, success: false, recovered_preview: String::new(), is_text: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_runs_end_to_end_for_default_input() {
        let result = run_benchmark(BenchmarkRequest {
            codec: "ternary".into(),
            profile: "pristine".into(),
            redundancy: 2,
            data: None,
        })
        .unwrap();
        assert!(result.roundtrip_ok);
        assert_eq!(result.recovery_probability, 1.0);
    }

    /// The interactive Benchmark Explorer's recovery estimate should reflect
    /// the real engine's fixed behavior under Illumina noise: consensus
    /// voting across simulated sequencing coverage corrects substitution
    /// errors that Reed-Solomon alone cannot, so recovery is now reliably
    /// high instead of near-zero (which is what it was before consensus was
    /// wired in -- a corrupted-but-present strand used to fail the codec's
    /// decode outright, redundancy or not).
    #[test]
    fn benchmark_recovery_is_reliably_high_under_illumina_noise_with_consensus() {
        let codec = make_codec("ternary").unwrap();
        let data = b"Consensus voting across simulated sequencing coverage \
            corrects substitution errors that Reed-Solomon alone cannot.";

        let recovery = estimate_recovery_probability(codec.as_ref(), data, HardwareProfile::Illumina, 4, 20);
        assert!(recovery > 0.5, "with real coverage + consensus + RS, recovery should be reliably high, got {recovery}");
    }

    #[test]
    fn benchmark_rejects_unknown_codec() {
        let err = run_benchmark(BenchmarkRequest {
            codec: "not-a-codec".into(),
            profile: "illumina".into(),
            redundancy: 0,
            data: Some("hello".into()),
        })
        .unwrap_err();
        assert!(err.contains("unknown codec"));
    }

    #[test]
    fn pipeline_demo_recovers_under_pristine_profile() {
        let result = run_pipeline_demo(PipelineRequest {
            data: "hello nucle".into(),
            profile: "pristine".into(),
            redundancy: 2,
        })
        .unwrap();
        assert!(result.recovery.success);
        assert_eq!(result.recovery.recovered_preview, "hello nucle");
    }

    #[test]
    fn pipeline_demo_rejects_oversized_input() {
        let big = "x".repeat(2001);
        let err = run_pipeline_demo(PipelineRequest {
            data: big,
            profile: "illumina".into(),
            redundancy: 2,
        })
        .unwrap_err();
        assert!(err.contains("capped at 2000"));
    }
}
