use tiny_http::{Server, Response, Header, Method};
use serde::{Deserialize, Serialize};

use nucle_codec::base::{DnaCodec, DnaStrand, Nucleotide, StrandCollection};
use nucle_codec::benchmark::benchmark_codec;
use nucle_codec::fountain::{FountainCodec, FountainConfig};
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::yinyang::{YinYangCodec, YinYangConfig};
use nucle_ecc::reed_solomon::{ReedSolomon, RsConfig};
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;

fn main() {
    let host = "127.0.0.1:8080";
    let server = Server::http(host).unwrap();
    println!("NucleScript Playground Server running at http://{}", host);

    // Embed the static HTML directly so that running the binary from any Cwd works
    let index_html = include_str!("../static/index.html");

    for request in server.incoming_requests() {
        match (request.method(), request.url()) {
            (&Method::Get, "/" | "/index.html") => {
                let response = Response::from_string(index_html)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
                let _ = request.respond(response);
            }
            (&Method::Post, "/analyze") => {
                handle_json(request, |body: AnalyzeRequest| {
                    Ok(nucle_lang::playground::analyze_source(&body.source))
                });
            }
            (&Method::Post, "/benchmark") => {
                handle_json(request, run_benchmark);
            }
            (&Method::Post, "/pipeline-demo") => {
                handle_json(request, run_pipeline_demo);
            }
            _ => {
                let response = Response::from_string("Not Found")
                    .with_status_code(404);
                let _ = request.respond(response);
            }
        }
    }
}

/// Read a JSON request body, deserialize it, run `handler`, and write back
/// a JSON response — shared by all three endpoints so each handler only
/// deals with its own request/response shape.
fn handle_json<Req, Res>(
    mut request: tiny_http::Request,
    handler: impl FnOnce(Req) -> Result<Res, String>,
) where
    Req: for<'de> Deserialize<'de>,
    Res: Serialize,
{
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        let response = Response::from_string(format!("Error reading body: {}", e)).with_status_code(400);
        let _ = request.respond(response);
        return;
    }

    let parsed: Req = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            let response = Response::from_string(format!("JSON parse error: {}", e)).with_status_code(400);
            let _ = request.respond(response);
            return;
        }
    };

    match handler(parsed) {
        Ok(result) => {
            let json = serde_json::to_string(&result).unwrap();
            let response = Response::from_string(json)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            let _ = request.respond(response);
        }
        Err(e) => {
            let json = serde_json::json!({ "error": e }).to_string();
            let response = Response::from_string(json)
                .with_status_code(400)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            let _ = request.respond(response);
        }
    }
}

#[derive(Deserialize)]
struct AnalyzeRequest {
    source: String,
}

// ---------------------------------------------------------------------------
// Feature 2: interactive codec benchmark
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BenchmarkRequest {
    codec: String,
    profile: String,
    redundancy: usize,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Serialize)]
struct BenchmarkResponse {
    codec_name: String,
    input_size: usize,
    strand_count: usize,
    total_nucleotides: usize,
    bits_per_nucleotide: f64,
    avg_gc_content: f64,
    max_homopolymer: usize,
    homopolymer_violation_count: usize,
    constraint_violations: usize,
    roundtrip_ok: bool,
    gc_distribution: Vec<usize>,
    recovery_probability: f64,
    estimated_cost_usd: f64,
}

fn parse_hw_profile(profile: &str) -> Result<HardwareProfile, String> {
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
fn profile_cost_per_base(profile: HardwareProfile) -> f64 {
    match profile {
        HardwareProfile::TwistBioscience => 0.00015,
        HardwareProfile::Illumina => 0.0001,
        HardwareProfile::OxfordNanopore => 0.00005,
        HardwareProfile::Idt => 0.00012,
        HardwareProfile::ColumnSynthesis => 0.00008,
        HardwareProfile::Pristine => 0.00001,
    }
}

fn make_codec(name: &str) -> Result<Box<dyn DnaCodec>, String> {
    match name.to_lowercase().as_str() {
        "ternary" => Ok(Box::new(TernaryCodec::new(TernaryConfig::no_overlap()))),
        "ternary-overlap" => Ok(Box::new(TernaryCodec::new(TernaryConfig::default()))),
        "yinyang" | "yin-yang" => Ok(Box::new(YinYangCodec::new(YinYangConfig::default()))),
        "fountain" => Ok(Box::new(FountainCodec::new(FountainConfig::default()))),
        other => Err(format!("unknown codec '{}'", other)),
    }
}

/// Monte-Carlo estimate of encode -> [+RS parity] -> noise -> recover
/// roundtrip success — the redundancy slider actually changes this number
/// because higher redundancy means more RS parity strands are available to
/// reconstruct dropped/corrupted data strands during each trial.
fn estimate_recovery_probability(
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
    let data_strand_count = encoded.strands.len();

    let parity_bytes: Vec<Vec<u8>> = if redundancy > 0 {
        let rs = ReedSolomon::new(RsConfig::new(redundancy));
        let strand_bytes: Vec<Vec<u8>> = encoded.strands.iter().map(dna_to_bytes).collect();
        rs.encode_block(&strand_bytes).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut all_strands = encoded.strands.clone();
    for parity in &parity_bytes {
        all_strands.push(bytes_to_dna(parity));
    }
    let combined = StrandCollection::from_strands(all_strands, data.len());

    let mut successes = 0u64;
    for t in 0..trials {
        let config = SimulationConfig {
            seed: 9000 + t,
            coverage_depth: 1,
            synthesis_profile: profile,
            sequencing_profile: profile,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        let sim_result = NoiseEngine::new(config).simulate(&combined);

        let final_strands: Vec<DnaStrand> = if redundancy > 0 && !parity_bytes.is_empty() {
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
            let rs = ReedSolomon::new(RsConfig::new(redundancy));
            match rs.decode_block(&received, &parity_received) {
                Ok(recovered) => recovered.iter().map(|b| bytes_to_dna(b)).collect(),
                Err(_) => continue,
            }
        } else {
            sim_result.pool.to_strand_collection(data.len()).strands
        };

        let collection = StrandCollection::from_strands(final_strands, data.len());
        if let Ok(decoded) = codec.decode(&collection) {
            if decoded == data {
                successes += 1;
            }
        }
    }
    successes as f64 / trials as f64
}

fn run_benchmark(req: BenchmarkRequest) -> Result<BenchmarkResponse, String> {
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
struct PipelineRequest {
    data: String,
    profile: String,
    #[serde(default = "default_redundancy")]
    redundancy: usize,
}

fn default_redundancy() -> usize {
    2
}

#[derive(Serialize)]
struct StrandView {
    index: usize,
    is_parity: bool,
    original: String,
    after_noise: String,
    corrupted: bool,
    dropped: bool,
    error_count: usize,
}

#[derive(Serialize)]
struct RecoveryView {
    attempted: bool,
    success: bool,
    recovered_preview: String,
    is_text: bool,
}

#[derive(Serialize)]
struct PipelineResponse {
    profile: String,
    redundancy: usize,
    original_size: usize,
    survival_rate: f64,
    avg_error_rate: f64,
    strands: Vec<StrandView>,
    recovery: RecoveryView,
}

fn dna_to_bytes(strand: &DnaStrand) -> Vec<u8> {
    strand.bases().iter().map(|n| n.to_bits()).collect()
}

fn bytes_to_dna(bytes: &[u8]) -> DnaStrand {
    let bases: Vec<Nucleotide> = bytes.iter().filter_map(|&b| Nucleotide::from_bits(b).ok()).collect();
    DnaStrand::new(bases)
}

fn run_pipeline_demo(req: PipelineRequest) -> Result<PipelineResponse, String> {
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
