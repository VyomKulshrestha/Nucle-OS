//! # Codec Benchmarking Framework
//!
//! Compares the performance of different DNA codecs across key metrics:
//!
//! - **Density**: bits per nucleotide achieved
//! - **Constraint compliance**: GC content, homopolymer runs
//! - **Strand count**: how many strands needed for given data
//! - **Roundtrip integrity**: encode → decode produces original data
//!
//! This is a contribution to the field — no standardised benchmark
//! suite exists for DNA storage codecs.

use crate::base::{DnaCodec, DnaError, StrandCollection};
use crate::constraints::{ConstraintConfig, ConstraintValidator};
use std::fmt;
use web_time::Instant;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Benchmark Result
// ---------------------------------------------------------------------------

/// Results from benchmarking a single codec on a single dataset.
#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkResult {
    /// Name of the codec.
    pub codec_name: String,
    /// Size of input data in bytes.
    pub input_size: usize,
    /// Number of strands produced.
    pub strand_count: usize,
    /// Total nucleotides across all strands.
    pub total_nucleotides: usize,
    /// Bits per nucleotide (higher is better, max 2.0).
    pub bits_per_nucleotide: f64,
    /// Average GC content across all strands (ideal: 0.50).
    pub avg_gc_content: f64,
    /// Maximum homopolymer run across all strands (lower is better).
    pub max_homopolymer: usize,
    /// Number of strands that violate default constraints.
    pub constraint_violations: usize,
    /// Total number of homopolymer-run violations across all strands
    /// (distinct from `max_homopolymer`, which is just the single longest run).
    pub homopolymer_violation_count: usize,
    /// Whether encode → decode roundtrip succeeded.
    pub roundtrip_ok: bool,
    /// Encoding time in microseconds.
    pub encode_time_us: u128,
    /// Decoding time in microseconds.
    pub decode_time_us: u128,
    /// Encoding throughput in bytes/second.
    pub encode_throughput: f64,
    /// Decoding throughput in bytes/second.
    pub decode_throughput: f64,
    /// GC content distribution (10 bins: 0-10%, 10-20%, ..., 90-100%).
    pub gc_distribution: Vec<usize>,
    /// Estimated recovery probability (0.0 to 1.0) under simulated noise.
    /// Always `None` from [`benchmark_codec`] itself: computing this needs
    /// `nucle_synth`'s noise model, and `nucle_codec` cannot depend on
    /// `nucle_synth` (which depends on `nucle_codec`) without a cycle.
    /// Callers with access to both crates (e.g. `nucle_cli`) should fill
    /// this in after calling `benchmark_codec`.
    pub recovery_probability: Option<f64>,
    /// Estimated synthesis cost in USD. Same caveat as `recovery_probability`:
    /// left `None` here, populated by callers that know the target hardware
    /// profile's cost-per-base.
    pub estimated_cost_usd: Option<f64>,
}

impl fmt::Display for BenchmarkResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "┌─ {} ─────────────────────────────", self.codec_name)?;
        writeln!(f, "│ Input:        {} bytes", self.input_size)?;
        writeln!(f, "│ Strands:      {}", self.strand_count)?;
        writeln!(f, "│ Nucleotides:  {}", self.total_nucleotides)?;
        writeln!(f, "│ Density:      {:.3} bits/nt", self.bits_per_nucleotide)?;
        writeln!(f, "│ GC content:   {:.1}%", self.avg_gc_content * 100.0)?;
        writeln!(f, "│ Max homopoly: {}", self.max_homopolymer)?;
        writeln!(f, "│ Violations:   {}", self.constraint_violations)?;
        writeln!(f, "│ Hpol viol:    {}", self.homopolymer_violation_count)?;
        writeln!(f, "│ Roundtrip:    {}", if self.roundtrip_ok { "✓ PASS" } else { "✗ FAIL" })?;
        writeln!(f, "│ Encode:       {} μs ({:.0} KB/s)", self.encode_time_us, self.encode_throughput / 1024.0)?;
        writeln!(f, "│ Decode:       {} μs ({:.0} KB/s)", self.decode_time_us, self.decode_throughput / 1024.0)?;
        if let Some(p) = self.recovery_probability {
            writeln!(f, "│ Recovery:     {:.1}%", p * 100.0)?;
        }
        if let Some(c) = self.estimated_cost_usd {
            writeln!(f, "│ Est. cost:    ${:.4}", c)?;
        }
        write!(f, "└────────────────────────────────────")
    }
}

// ---------------------------------------------------------------------------
// Benchmark Runner
// ---------------------------------------------------------------------------

/// Run a benchmark on a single codec with given input data.
pub fn benchmark_codec(
    codec: &dyn DnaCodec,
    data: &[u8],
) -> Result<BenchmarkResult, DnaError> {
    // Encode
    let encode_start = Instant::now();
    let encoded = codec.encode(data)?;
    let encode_time = encode_start.elapsed();

    // Decode
    let decode_start = Instant::now();
    let decoded = codec.decode(&encoded)?;
    let decode_time = decode_start.elapsed();

    // Check roundtrip
    let roundtrip_ok = decoded == data;

    // Check constraints (standard synthesis requirements)
    let validator = ConstraintValidator::new(ConstraintConfig::default());
    let constraint_violations = encoded
        .strands
        .iter()
        .filter(|s| !validator.is_valid(s))
        .count();
    let homopolymer_violation_count: usize = encoded
        .strands
        .iter()
        .map(|s| validator.check_homopolymers(s).len())
        .sum();

    let encode_us = encode_time.as_micros();
    let decode_us = decode_time.as_micros();

    let encode_throughput = if encode_us > 0 {
        data.len() as f64 / (encode_us as f64 / 1_000_000.0)
    } else {
        f64::INFINITY
    };

    let decode_throughput = if decode_us > 0 {
        data.len() as f64 / (decode_us as f64 / 1_000_000.0)
    } else {
        f64::INFINITY
    };

    let mut gc_distribution = vec![0; 10];
    for strand in &encoded.strands {
        let gc = strand.gc_content();
        let bin = ((gc * 10.0).floor() as usize).min(9);
        gc_distribution[bin] += 1;
    }

    Ok(BenchmarkResult {
        codec_name: codec.name().to_string(),
        input_size: data.len(),
        strand_count: encoded.strand_count(),
        total_nucleotides: encoded.total_nucleotides(),
        bits_per_nucleotide: encoded.bits_per_nucleotide(),
        avg_gc_content: encoded.avg_gc_content(),
        max_homopolymer: encoded.max_homopolymer(),
        constraint_violations,
        homopolymer_violation_count,
        roundtrip_ok,
        encode_time_us: encode_us,
        decode_time_us: decode_us,
        encode_throughput,
        decode_throughput,
        gc_distribution,
        recovery_probability: None,
        estimated_cost_usd: None,
    })
}

/// Comparison report across multiple codecs.
#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    pub results: Vec<BenchmarkResult>,
}

impl ComparisonReport {
    /// Create a comparison report from benchmark results.
    pub fn new(results: Vec<BenchmarkResult>) -> Self {
        Self { results }
    }

    /// Get the codec with the best density.
    pub fn best_density(&self) -> Option<&BenchmarkResult> {
        self.results
            .iter()
            .max_by(|a, b| a.bits_per_nucleotide.partial_cmp(&b.bits_per_nucleotide).unwrap())
    }

    /// Get the codec with the fastest encoding.
    pub fn fastest_encoder(&self) -> Option<&BenchmarkResult> {
        self.results.iter().min_by_key(|r| r.encode_time_us)
    }

    /// Get the codec with the fewest constraint violations.
    pub fn most_compliant(&self) -> Option<&BenchmarkResult> {
        self.results.iter().min_by_key(|r| r.constraint_violations)
    }
}

impl fmt::Display for ComparisonReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "╔══════════════════════════════════════════════════════════════════╗")?;
        writeln!(f, "║               DNA Codec Benchmark Comparison                    ║")?;
        writeln!(f, "╠══════════════════════════════════════════════════════════════════╣")?;
        writeln!(f, "║ {:20} │ {:>8} │ {:>6} │ {:>4} │ {:>3} │ {:>4} ║",
            "Codec", "bits/nt", "GC %", "Hpol", "Bio", "R/T")?;
        writeln!(f, "╟──────────────────────┼──────────┼────────┼──────┼─────┼──────╢")?;

        for r in &self.results {
            let bio_ok = r.constraint_violations == 0;
            writeln!(f, "║ {:20} │ {:>8.3} │ {:>5.1}% │ {:>4} │  {}  │  {}   ║",
                r.codec_name,
                r.bits_per_nucleotide,
                r.avg_gc_content * 100.0,
                r.max_homopolymer,
                if bio_ok { "✓" } else { "✗" },
                if r.roundtrip_ok { "✓" } else { "✗" },
            )?;
        }

        writeln!(f, "╚══════════════════════════════════════════════════════════════════╝")?;

        if let Some(best) = self.best_density() {
            writeln!(f, "  Best density:    {} ({:.3} bits/nt)", best.codec_name, best.bits_per_nucleotide)?;
        }
        if let Some(fast) = self.fastest_encoder() {
            writeln!(f, "  Fastest encode:  {} ({} μs)", fast.codec_name, fast.encode_time_us)?;
        }
        writeln!(f)?;
        writeln!(f, "  Bio = all strands pass biological constraints (GC 40–60%, homopolymer ≤ 3)")?;
        writeln!(f, "  R/T = encode → decode roundtrip produces identical data")?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience: run all codecs
// ---------------------------------------------------------------------------

/// Benchmark all available codecs on the given data and return a comparison.
pub fn benchmark_all_codecs(data: &[u8]) -> ComparisonReport {
    use crate::fountain::{FountainCodec, FountainConfig};
    use crate::ternary::{TernaryCodec, TernaryConfig};
    use crate::yinyang::{YinYangCodec, YinYangConfig};

    let mut results = Vec::new();

    // Ternary — no overlap
    let ternary_no = TernaryCodec::new(TernaryConfig::no_overlap());
    if let Ok(r) = benchmark_codec(&ternary_no, data) {
        results.push(r);
    }

    // Ternary — default overlap
    let ternary_def = TernaryCodec::new(TernaryConfig::default());
    if let Ok(r) = benchmark_codec(&ternary_def, data) {
        results.push(r);
    }

    // Yin-Yang — GC-balanced by construction (2 bits/nt theoretical)
    let yinyang = YinYangCodec::new(YinYangConfig::default());
    if let Ok(r) = benchmark_codec(&yinyang, data) {
        results.push(r);
    }

    // Fountain — with constraint screening (biologically valid output).
    // Needs enough data variation for screening to find valid strands;
    // if the input is too small/uniform, screening rejects everything
    // and this benchmark is silently skipped.
    let fountain_screened = FountainCodec::new(FountainConfig::default());
    if let Ok(r) = benchmark_codec(&fountain_screened, data) {
        results.push(r);
    }

    // Fountain — unscreened (raw codec, shows theoretical density
    // but strands may violate biological constraints).
    let fountain_raw = FountainCodec::new(FountainConfig {
        overhead: 1.50,
        ..FountainConfig::unscreened()
    });
    if let Ok(r) = benchmark_codec(&fountain_raw, data) {
        results.push(r);
    }

    ComparisonReport::new(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ternary::{TernaryCodec, TernaryConfig};
    use crate::fountain::{FountainCodec, FountainConfig};

    #[test]
    fn test_benchmark_ternary() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Benchmark test data for ternary codec";

        let result = benchmark_codec(&codec, data).unwrap();
        assert!(result.roundtrip_ok);
        assert!(result.bits_per_nucleotide > 0.0);
        assert!(result.max_homopolymer <= 1); // Ternary guarantees no homopolymers
    }

    #[test]
    fn test_benchmark_fountain() {
        let codec = FountainCodec::new(FountainConfig {
            segment_size: 4,
            overhead: 2.0,
            seed: 42,
            ..FountainConfig::unscreened()
        });
        let data = b"Benchmark test data!";

        let result = benchmark_codec(&codec, data).unwrap();
        assert!(result.roundtrip_ok);
        assert!(result.bits_per_nucleotide > 0.0);
    }

    #[test]
    fn test_comparison_report() {
        let data = b"Compare these codecs on identical data for a fair benchmark";
        let report = benchmark_all_codecs(data);

        assert!(!report.results.is_empty());
        assert!(report.best_density().is_some());

        // All should roundtrip successfully
        for r in &report.results {
            assert!(r.roundtrip_ok, "{} failed roundtrip", r.codec_name);
        }
    }

    #[test]
    fn test_benchmark_display() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Display test";

        let result = benchmark_codec(&codec, data).unwrap();
        let display = format!("{}", result);
        assert!(display.contains("ternary"));
        assert!(display.contains("bits/nt"));
    }
}
