//! End-to-end integration tests for the binary ↔ DNA ↔ binary pipeline.
//!
//! Tests encode a binary file → get DNA strands → pass through
//! synthesis simulator → decode back → verify data integrity.

#[cfg(test)]
mod tests {
    use nucle_codec::base::DnaCodec;
    use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
    use nucle_codec::fountain::{FountainCodec, FountainConfig};
    use nucle_codec::constraints::ConstraintConfig;
    use nucle_synth::noise::{NoiseEngine, SimulationConfig};
    use nucle_synth::profiles::HardwareProfile;

    // -----------------------------------------------------------------------
    // Ternary Codec E2E
    // -----------------------------------------------------------------------

    #[test]
    fn e2e_ternary_pristine_roundtrip() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Hello, DNA storage! This is an end-to-end test.";

        let encoded = codec.encode(data).unwrap();

        // Pass through pristine simulator (no errors)
        let engine = NoiseEngine::new(SimulationConfig::pristine());
        let result = engine.simulate(&encoded);

        let recovered = result.pool.to_strand_collection(data.len());
        let decoded = codec.decode(&recovered).unwrap();

        assert_eq!(decoded, data.to_vec(), "pristine roundtrip failed");
    }

    #[test]
    fn e2e_ternary_all_byte_values() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data: Vec<u8> = (0..=255).collect();

        let encoded = codec.encode(&data).unwrap();
        let engine = NoiseEngine::new(SimulationConfig::pristine());
        let result = engine.simulate(&encoded);

        let recovered = result.pool.to_strand_collection(data.len());
        let decoded = codec.decode(&recovered).unwrap();

        assert_eq!(decoded, data, "all 256 byte values roundtrip failed");
    }

    #[test]
    fn e2e_ternary_no_homopolymers_after_encoding() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = vec![0u8; 500]; // Worst case for homopolymers

        let encoded = codec.encode(&data).unwrap();

        for (i, strand) in encoded.strands.iter().enumerate() {
            let (_, max_run) = strand.max_homopolymer_run();
            // Ternary guarantees no homopolymers within data, but the 4-trit
            // index header can create a max-2 run at the header/payload junction
            assert!(
                max_run <= 2,
                "strand {} has homopolymer run of {} (expected max 2 with index header)",
                i, max_run
            );
        }
    }

    #[test]
    fn e2e_ternary_with_overlap_roundtrip() {
        let codec = TernaryCodec::new(TernaryConfig::default());
        let data = b"Testing overlapping segments for redundancy.";

        let encoded = codec.encode(data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded, data.to_vec());
    }

    // -----------------------------------------------------------------------
    // Fountain Codec E2E
    // -----------------------------------------------------------------------

    #[test]
    fn e2e_fountain_pristine_roundtrip() {
        let codec = FountainCodec::new(FountainConfig {
            segment_size: 4,
            overhead: 2.0,
            max_screening_attempts: 100,
            screen_constraints: false,
            constraint_config: ConstraintConfig::relaxed(),
            seed: 42,
        });
        let data = b"Fountain codes are rateless!";

        let encoded = codec.encode(data).unwrap();

        let engine = NoiseEngine::new(SimulationConfig::pristine());
        let result = engine.simulate(&encoded);

        let recovered = result.pool.to_strand_collection(data.len());
        let decoded = codec.decode(&recovered).unwrap();

        assert_eq!(decoded, data.to_vec(), "fountain pristine roundtrip failed");
    }

    #[test]
    fn e2e_fountain_binary_data() {
        let codec = FountainCodec::new(FountainConfig {
            segment_size: 4,
            overhead: 3.0,
            max_screening_attempts: 100,
            screen_constraints: false,
            constraint_config: ConstraintConfig::relaxed(),
            seed: 42,
        });
        let data: Vec<u8> = (0..32).collect();

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded, data, "fountain binary roundtrip failed");
    }

    // -----------------------------------------------------------------------
    // Synthesis Simulator Pipeline Tests
    // -----------------------------------------------------------------------

    #[test]
    fn e2e_simulator_reports_errors_correctly() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Testing error injection in the simulator pipeline.";

        let encoded = codec.encode(data).unwrap();

        // Use Nanopore (highest error rate) to ensure errors are injected
        let config = SimulationConfig {
            seed: 42,
            coverage_depth: 1,
            synthesis_profile: HardwareProfile::Pristine,
            sequencing_profile: HardwareProfile::OxfordNanopore,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        let engine = NoiseEngine::new(config);
        let result = engine.simulate(&encoded);

        // Nanopore should introduce some errors
        let total_errors = result.pool.total_errors();
        assert!(
            total_errors.total() > 0,
            "nanopore should introduce errors"
        );
        // Error count should include indels (nanopore's specialty)
        println!(
            "Nanopore errors: {} subs, {} ins, {} del",
            total_errors.substitutions,
            total_errors.insertions,
            total_errors.deletions
        );
    }

    #[test]
    fn e2e_coverage_depth_produces_multiple_copies() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = b"Coverage test";

        let encoded = codec.encode(data).unwrap();
        let strand_count = encoded.strand_count();

        let config = SimulationConfig::pristine().with_coverage(10);
        let engine = NoiseEngine::new(config);
        let result = engine.simulate(&encoded);

        assert_eq!(
            result.output_strand_count,
            strand_count * 10,
            "10x coverage should produce 10x strands"
        );
    }

    #[test]
    fn e2e_different_profiles_different_error_rates() {
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let data = vec![42u8; 200];
        let encoded = codec.encode(&data).unwrap();

        let profiles = vec![
            HardwareProfile::Illumina,
            HardwareProfile::OxfordNanopore,
            HardwareProfile::TwistBioscience,
        ];

        let mut error_rates: Vec<(String, f64)> = Vec::new();

        for profile in profiles {
            let result = NoiseEngine::simulate_single_profile(&encoded, profile, 42);
            error_rates.push((
                profile.name().to_string(),
                result.avg_error_rate(),
            ));
        }

        // Nanopore should have highest error rate
        let nanopore_rate = error_rates.iter()
            .find(|(n, _)| n.contains("Nanopore"))
            .unwrap().1;
        let illumina_rate = error_rates.iter()
            .find(|(n, _)| n.contains("Illumina"))
            .unwrap().1;

        assert!(
            nanopore_rate >= illumina_rate,
            "nanopore ({:.4}) should have >= errors than illumina ({:.4})",
            nanopore_rate, illumina_rate
        );
    }

    // -----------------------------------------------------------------------
    // Codec Comparison via Benchmark
    // -----------------------------------------------------------------------

    #[test]
    fn e2e_benchmark_all_codecs() {
        use nucle_codec::benchmark::benchmark_all_codecs;

        let data = b"Benchmarking all codecs on identical data for fair comparison.";
        let report = benchmark_all_codecs(data);

        // All codecs should pass roundtrip
        for r in &report.results {
            assert!(
                r.roundtrip_ok,
                "codec {} failed roundtrip",
                r.codec_name
            );
            assert!(r.bits_per_nucleotide > 0.0);
        }

        // Print the comparison
        println!("{}", report);
    }
}
