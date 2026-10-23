//! Shared playground demo logic: the interactive codec benchmark, the
//! encode -> noise -> recovery pipeline visualizer, and the storage
//! shelf-life calculator.
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
        "array-hifi" => Ok(HardwareProfile::ArrayHighFidelity),
        "array-lofi" => Ok(HardwareProfile::ArrayLowFidelity),
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
        // Array oligo pools: chip synthesis, priced like Twist.
        HardwareProfile::ArrayHighFidelity | HardwareProfile::ArrayLowFidelity => 0.00015,
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
        // Parity symbols span the full 0-255 range, unlike data strand
        // bytes (always a single to_bits() value) -- pack 4 bases/byte.
        rs.encode_block(&strand_bytes).unwrap_or_default().iter().map(|p| DnaStrand::from_packed_bytes(p)).collect()
    } else {
        Vec::new()
    };

    let data_collection = StrandCollection::from_strands(encoded.strands.clone(), data.len());
    let parity_collection = StrandCollection::from_strands(parity_strands.clone(), 0);

    let mut successes = 0u64;
    for t in 0..trials {
        let config = SimulationConfig::realistic(profile, RECOVERY_COVERAGE_DEPTH, 9000 + t);
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
        parity_bytes.iter().map(|p| p.len() * 4).sum::<usize>()
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
            all_strands.push(DnaStrand::from_packed_bytes(parity));
        }
    }

    let combined = StrandCollection::from_strands(all_strands.clone(), data.len());

    let config = SimulationConfig::realistic(profile, 1, 42);
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
    // Keep a slot per parity strand (not a dense list of survivors) --
    // dropping a missing one would shift every later parity strand onto
    // the wrong codeword position and corrupt the whole stripe's math.
    let parity_received: Vec<Option<Vec<u8>>> = sim_result
        .pool
        .strands
        .iter()
        .skip(data_strand_count)
        .map(|s| if s.is_intact { Some(s.sequence.unpack_bytes()) } else { None })
        .collect();

    let recovery = if req.redundancy > 0 && parity_received.iter().any(Option::is_some) {
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

// ---------------------------------------------------------------------------
// Storage shelf life
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ShelfLifeRequest {
    /// `encapsulated` or `bone`.
    pub medium: String,
    pub celsius: f64,
    #[serde(default = "default_strand_len")]
    pub strand_len: usize,
    #[serde(default = "default_data_strands")]
    pub data_strands: usize,
    #[serde(default = "default_parity_strands")]
    pub parity_strands: usize,
    #[serde(default = "default_copies")]
    pub copies_per_strand: u32,
    #[serde(default = "default_target")]
    pub target_percent: f64,
}

fn default_strand_len() -> usize { 150 }
fn default_data_strands() -> usize { 100 }
fn default_parity_strands() -> usize { 20 }
fn default_copies() -> u32 { 10 }
fn default_target() -> f64 { 99.9 }

#[derive(Debug, Serialize)]
pub struct ShelfLifePoint {
    pub years: f64,
    pub recovery_probability: f64,
}

#[derive(Debug, Serialize)]
pub struct TemperatureRow {
    pub celsius: f64,
    pub strand_half_life_years: Option<f64>,
    pub shelf_life_years: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ShelfLifeResponse {
    pub medium: String,
    pub citation: String,
    pub activation_energy_kj_mol: f64,
    pub break_rate_per_nt_year: f64,
    pub strand_half_life_years: Option<f64>,
    /// `None` when recovery stays above target beyond 10^15 years.
    pub shelf_life_years: Option<f64>,
    /// Whole-file recovery probability against age, log-spaced, for a chart.
    pub curve: Vec<ShelfLifePoint>,
    pub by_temperature: Vec<TemperatureRow>,
}

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// How long an archive stays readable under Arrhenius strand decay (see
/// `nucle_synth::decay`), plus the recovery curve around that point and a
/// freezer/fridge/cellar/room comparison.
pub fn run_shelf_life(req: ShelfLifeRequest) -> Result<ShelfLifeResponse, String> {
    use nucle_synth::decay::{strand_half_life_years, ShelfLifeQuery, StorageMedium};
    let medium = StorageMedium::parse(&req.medium)
        .ok_or_else(|| format!("unknown storage medium '{}' (expected encapsulated or bone)", req.medium))?;
    if !(req.target_percent > 0.0 && req.target_percent < 100.0) {
        return Err("target_percent must be between 0 and 100".to_string());
    }
    if req.strand_len == 0 || req.data_strands == 0 || req.copies_per_strand == 0 {
        return Err("strand_len, data_strands and copies_per_strand must be at least 1".to_string());
    }
    if !(-80.0..=100.0).contains(&req.celsius) {
        return Err("celsius must be between -80 and 100".to_string());
    }
    let query = |celsius: f64| ShelfLifeQuery {
        medium,
        celsius,
        strand_len: req.strand_len,
        data_strands: req.data_strands,
        parity_strands: req.parity_strands,
        copies_per_strand: req.copies_per_strand,
        target_recovery: req.target_percent / 100.0,
    };
    let q = query(req.celsius);
    let rate = medium.break_rate_per_nt_year(req.celsius);
    let life = q.shelf_life_years();

    // Log-spaced from a hundredth to ten times the shelf life (or a fixed
    // span when it is effectively unbounded), 48 points.
    let (lo, hi) = if life.is_finite() && life > 0.0 { (life / 100.0, life * 10.0) } else { (1.0, 1e12) };
    let curve = (0..48)
        .map(|i| {
            let years = lo * (hi / lo).powf(i as f64 / 47.0);
            ShelfLifePoint { years, recovery_probability: q.recovery_probability(years) }
        })
        .collect();
    let by_temperature = [-20.0, 4.0, 10.0, 25.0]
        .iter()
        .map(|&c| TemperatureRow {
            celsius: c,
            strand_half_life_years: finite(strand_half_life_years(medium.break_rate_per_nt_year(c), req.strand_len)),
            shelf_life_years: finite(query(c).shelf_life_years()),
        })
        .collect();

    Ok(ShelfLifeResponse {
        medium: medium.name().to_string(),
        citation: medium.citation().to_string(),
        activation_energy_kj_mol: medium.kinetics().activation_energy_kj_mol,
        break_rate_per_nt_year: rate,
        strand_half_life_years: finite(strand_half_life_years(rate, req.strand_len)),
        shelf_life_years: finite(life),
        curve,
        by_temperature,
    })
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

        let recovery = estimate_recovery_probability(codec.as_ref(), data, HardwareProfile::Illumina, 2, 3);
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

    #[test]
    fn shelf_life_matches_the_published_half_life_and_falls_with_temperature() {
        let resp = run_shelf_life(ShelfLifeRequest {
            medium: "encapsulated".into(),
            celsius: 25.0,
            strand_len: 150,
            data_strands: 100,
            parity_strands: 20,
            copies_per_strand: 10,
            target_percent: 99.9,
        })
        .unwrap();
        let half = resp.strand_half_life_years.unwrap();
        assert!((half - 38_000.0).abs() < 1_000.0, "{half}");
        let life = resp.shelf_life_years.unwrap();
        assert_eq!(resp.curve.len(), 48);
        // Monotone up to floating-point rounding in the binomial sum.
        assert!(resp.curve.windows(2).all(|w| w[0].recovery_probability + 1e-9 >= w[1].recovery_probability));
        assert!(resp.curve.first().unwrap().years < life && resp.curve.last().unwrap().years > life);
        let lives: Vec<f64> = resp.by_temperature.iter().map(|r| r.shelf_life_years.unwrap()).collect();
        assert!(lives.windows(2).all(|w| w[0] > w[1]), "{lives:?}");
    }

    #[test]
    fn shelf_life_rejects_bad_input() {
        let req = |medium: &str, celsius: f64, target: f64| ShelfLifeRequest {
            medium: medium.into(),
            celsius,
            strand_len: 150,
            data_strands: 100,
            parity_strands: 20,
            copies_per_strand: 10,
            target_percent: target,
        };
        assert!(run_shelf_life(req("tape", 25.0, 99.9)).unwrap_err().contains("unknown storage medium"));
        assert!(run_shelf_life(req("bone", 500.0, 99.9)).is_err());
        assert!(run_shelf_life(req("bone", 25.0, 100.0)).is_err());
        let defaults: ShelfLifeRequest = serde_json::from_str(r#"{"medium":"bone","celsius":10}"#).unwrap();
        assert_eq!(defaults.strand_len, 150);
        assert!(run_shelf_life(defaults).is_ok());
    }
}
