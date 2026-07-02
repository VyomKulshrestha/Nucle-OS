//! In-browser build of the NucleScript Playground.
//!
//! Exposes the same three operations as `nucle_playground`'s HTTP endpoints
//! (`/analyze`, `/benchmark`, `/pipeline-demo`), as plain
//! JSON-string-in/JSON-string-out functions, so the frontend can call
//! straight into WASM instead of `fetch()`-ing a server. The response shape
//! matches the server exactly (including `{"error": "..."}` on failure) so
//! the same JS rendering code works against either backend.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(serde::Deserialize)]
struct AnalyzeRequest {
    source: String,
}

#[wasm_bindgen]
pub fn analyze(request_json: &str) -> String {
    let req: AnalyzeRequest = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => return error_json(&format!("JSON parse error: {}", e)),
    };
    let report = nucle_lang::playground::analyze_source(&req.source);
    serde_json::to_string(&report).unwrap_or_else(|e| error_json(&e.to_string()))
}

#[wasm_bindgen]
pub fn benchmark(request_json: &str) -> String {
    run_json(request_json, nucle_demo_core::run_benchmark)
}

// TEMPORARY bisection probe -- calls benchmark_codec alone with no
// redundancy/recovery logic, to isolate which part of run_benchmark panics
// on wasm32. Remove once the wasm32 time panic is diagnosed.
#[wasm_bindgen]
pub fn debug_probe(step: u32) -> String {
    let data = b"The quick brown fox jumps over the lazy dog. NucleOS benchmarks all available DNA codecs.";
    let codec = match nucle_demo_core::make_codec("ternary") {
        Ok(c) => c,
        Err(e) => return format!("step0-make_codec-err: {}", e),
    };
    if step == 0 {
        return "step0-ok".into();
    }
    let bench = match nucle_codec::benchmark::benchmark_codec(codec.as_ref(), data) {
        Ok(b) => b,
        Err(e) => return format!("step1-benchmark_codec-err: {}", e),
    };
    if step == 1 {
        return format!("step1-ok bits_per_nt={}", bench.bits_per_nucleotide);
    }
    let profile = match nucle_demo_core::parse_hw_profile("illumina") {
        Ok(p) => p,
        Err(e) => return format!("step2-parse_hw_profile-err: {}", e),
    };
    if step == 2 {
        return "step2-ok".into();
    }
    let recovery = nucle_demo_core::estimate_recovery_probability(codec.as_ref(), data, profile, 3, 20);
    format!("step3-ok recovery={}", recovery)
}

#[wasm_bindgen]
pub fn pipeline_demo(request_json: &str) -> String {
    run_json(request_json, nucle_demo_core::run_pipeline_demo)
}

fn run_json<Req, Res>(request_json: &str, handler: impl FnOnce(Req) -> Result<Res, String>) -> String
where
    Req: for<'de> serde::Deserialize<'de>,
    Res: serde::Serialize,
{
    let parsed: Req = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => return error_json(&format!("JSON parse error: {}", e)),
    };
    match handler(parsed) {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| error_json(&e.to_string())),
        Err(e) => error_json(&e),
    }
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
