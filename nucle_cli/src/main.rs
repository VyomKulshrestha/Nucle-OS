//! # NucleOS CLI — Full Stack DNA Storage Interface
//!
//! Unified command-line tool tying all layers together.

use clap::{Parser, Subcommand};
use nucle_vfs::syscall::NucleOS;
use nucle_agent::executor::Executor;
use nucle_agent::tools;
use nucle_codec::base::DnaCodec;
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::fountain::{FountainCodec, FountainConfig};
use nucle_codec::constraints::{ConstraintValidator, ConstraintConfig};
use nucle_codec::benchmark::benchmark_all_codecs;
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs;
use std::time::Instant;

/// NucleOS — DNA Storage Engine CLI
#[derive(Parser)]
#[command(name = "nucle")]
#[command(version = "0.1.0")]
#[command(about = "A full software stack for molecular DNA data storage")]
#[command(long_about = "NucleOS provides encode/decode, error correction, primer-based \
    addressing, CRISPR random access, and a virtual filesystem for DNA storage.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Encode a file to DNA strands
    Encode {
        /// Input file path
        file: String,
        /// Output file for DNA strands (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Decode DNA strands back to binary
    Decode {
        /// Input file containing DNA strands
        file: String,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Original file size in bytes (required for decoding)
        #[arg(short, long)]
        size: usize,
    },

    /// Store a file in the DNA storage pool
    Store {
        /// File to store
        file: String,
        /// Number of RS parity strands for error correction
        #[arg(short, long, default_value = "2")]
        redundancy: usize,
    },

    /// Retrieve a file from the DNA storage pool
    Retrieve {
        /// Filename to retrieve
        name: String,
    },

    /// Search for files in the storage pool
    Search {
        /// Search query (supports name:, type:, size: filters)
        query: String,
        /// Maximum results
        #[arg(short = 'k', long, default_value = "5")]
        top_k: usize,
    },

    /// Show DNA storage pool status
    #[command(name = "pool")]
    Pool,

    /// Simulate synthesis/sequencing noise on data
    Simulate {
        /// Input file to simulate
        file: String,
        /// Hardware profile: illumina, nanopore, twist
        #[arg(short, long, default_value = "illumina")]
        profile: String,
        /// Coverage depth
        #[arg(short, long, default_value = "1")]
        coverage: usize,
    },

    /// Benchmark all available codecs
    Bench {
        /// Input file to benchmark (or use built-in test data)
        file: Option<String>,
    },

    /// Stress test all codecs against diverse data distributions
    Stress {
        /// Data size in bytes for stress testing
        #[arg(short, long, default_value = "256")]
        size: usize,
    },

    /// Full-pipeline stress test: encode → noise → ECC → recover across many files
    Pipeline {
        /// Number of files to generate and test
        #[arg(short, long, default_value = "100")]
        files: usize,
        /// Size of each test file in bytes
        #[arg(short, long, default_value = "1024")]
        size: usize,
        /// Hardware noise profile: illumina, nanopore, twist, pristine
        #[arg(short, long, default_value = "illumina")]
        profile: String,
        /// Sequencing coverage depth
        #[arg(short, long, default_value = "10")]
        coverage: usize,
        /// RS parity strands per file
        #[arg(short, long, default_value = "4")]
        redundancy: usize,
    },

    /// Run a NucleScript source file (.nsl)
    Run {
        /// NucleScript source file to compile and execute
        source: String,
    },

    /// Compile a NucleScript source file into a no-hardware simulation plan
    Plan {
        /// NucleScript source file to compile
        source: String,
    },

    /// List released NucleScript packages bundled with this repository
    Packages,

    /// Run a natural language command via the agent
    Agent {
        /// Natural language command
        command: Vec<String>,
    },

    /// Show available agent tools
    Tools,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Encode { file, output } => cmd_encode(&file, output.as_deref()),
        Commands::Decode { file, output, size } => cmd_decode(&file, output.as_deref(), size),
        Commands::Store { file, redundancy } => cmd_store(&file, redundancy),
        Commands::Retrieve { name } => cmd_retrieve(&name),
        Commands::Search { query, top_k } => cmd_search(&query, top_k),
        Commands::Pool => cmd_pool(),
        Commands::Simulate { file, profile, coverage } => cmd_simulate(&file, &profile, coverage),
        Commands::Bench { file } => cmd_bench(file.as_deref()),
        Commands::Stress { size } => cmd_stress(size),
        Commands::Pipeline { files, size, profile, coverage, redundancy } => {
            cmd_pipeline(files, size, &profile, coverage, redundancy)
        }
        Commands::Run { source } => cmd_run(&source),
        Commands::Plan { source } => cmd_plan(&source),
        Commands::Packages => cmd_packages(),
        Commands::Agent { command } => cmd_agent(&command.join(" ")),
        Commands::Tools => cmd_help(),
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_encode(file: &str, output: Option<&str>) {
    let data = match fs::read(file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", file, e);
            std::process::exit(1);
        }
    };

    let codec = TernaryCodec::new(TernaryConfig::no_overlap());
    let encoded = match codec.encode(&data) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Encoding failed: {}", e);
            std::process::exit(1);
        }
    };

    let mut result = String::new();
    result.push_str(&format!("# NucleOS DNA Encoding\n"));
    result.push_str(&format!("# Source: {}\n", file));
    result.push_str(&format!("# Size: {} bytes\n", data.len()));
    result.push_str(&format!("# Strands: {}\n", encoded.strand_count()));
    result.push_str(&format!("# Codec: ternary-rotating-cipher\n\n"));

    for (i, strand) in encoded.strands.iter().enumerate() {
        result.push_str(&format!(">{:04}\n{}\n", i, strand));
    }

    if let Some(out) = output {
        if let Err(e) = fs::write(out, &result) {
            eprintln!("Error writing '{}': {}", out, e);
            std::process::exit(1);
        }
        println!("✓ Encoded {} → {} ({} strands)", file, out, encoded.strand_count());
    } else {
        print!("{}", result);
    }
}

fn cmd_decode(file: &str, output: Option<&str>, size: usize) {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading '{}': {}", file, e);
            std::process::exit(1);
        }
    };

    // Parse FASTA-like format
    let mut strands = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('>') {
            continue;
        }
        match nucle_codec::base::DnaStrand::from_str(trimmed) {
            Ok(strand) => strands.push(strand),
            Err(e) => {
                eprintln!("Warning: skipping invalid strand: {}", e);
            }
        }
    }

    let collection = nucle_codec::base::StrandCollection::from_strands(strands, size);
    let codec = TernaryCodec::new(TernaryConfig::no_overlap());

    match codec.decode(&collection) {
        Ok(data) => {
            if let Some(out) = output {
                if let Err(e) = fs::write(out, &data) {
                    eprintln!("Error writing '{}': {}", out, e);
                    std::process::exit(1);
                }
                println!("✓ Decoded {} → {} ({} bytes)", file, out, data.len());
            } else {
                // Try printing as UTF-8, fall back to hex
                match String::from_utf8(data.clone()) {
                    Ok(text) => print!("{}", text),
                    Err(_) => {
                        for byte in &data {
                            print!("{:02x}", byte);
                        }
                        println!();
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Decoding failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_store(file: &str, redundancy: usize) {
    let data = match fs::read(file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", file, e);
            std::process::exit(1);
        }
    };

    let filename = std::path::Path::new(file)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file);

    let mut os = NucleOS::new(100);
    match os.dna_write(filename, &data, redundancy) {
        Ok(result) => {
            println!("✓ {}", result);
            println!("\n{}", os.dna_stat());
        }
        Err(e) => {
            eprintln!("Store failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_retrieve(name: &str) {
    let os = NucleOS::new(100);
    match os.dna_read(name) {
        Ok(data) => {
            match String::from_utf8(data.clone()) {
                Ok(text) => println!("{}", text),
                Err(_) => {
                    println!("Binary data ({} bytes)", data.len());
                }
            }
        }
        Err(e) => {
            eprintln!("Retrieve failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_search(query: &str, top_k: usize) {
    let os = NucleOS::new(100);
    let results = os.dna_search(query, top_k);
    if results.is_empty() {
        println!("No matching files found.");
    } else {
        println!("Search results for '{}':", query);
        for (i, r) in results.iter().enumerate() {
            println!("  {}. {}", i + 1, r);
        }
    }
}

fn cmd_pool() {
    let os = NucleOS::new(100);
    println!("{}", os.dna_stat());
}

fn cmd_simulate(file: &str, profile: &str, coverage: usize) {
    let data = match fs::read(file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", file, e);
            std::process::exit(1);
        }
    };

    let codec = TernaryCodec::new(TernaryConfig::no_overlap());
    let encoded = match codec.encode(&data) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Encoding failed: {}", e);
            std::process::exit(1);
        }
    };

    let hw_profile = match profile.to_lowercase().as_str() {
        "illumina" => HardwareProfile::Illumina,
        "nanopore" => HardwareProfile::OxfordNanopore,
        "twist" => HardwareProfile::TwistBioscience,
        "pristine" => HardwareProfile::Pristine,
        _ => {
            eprintln!("Unknown profile: {}. Use: illumina, nanopore, twist, pristine", profile);
            std::process::exit(1);
        }
    };

    let config = SimulationConfig {
        seed: 42,
        coverage_depth: coverage as u32,
        synthesis_profile: hw_profile,
        sequencing_profile: hw_profile,
        simulate_decay: false,
        decay_rate: 0.0,
        storage_time: 0.0,
    };

    let engine = NoiseEngine::new(config);
    let result = engine.simulate(&encoded);

    println!("╔══════════════════════════════════════╗");
    println!("║     Synthesis Simulation Results     ║");
    println!("╠══════════════════════════════════════╣");
    println!("║ Profile:    {:>24} ║", profile);
    println!("║ Coverage:   {:>24}×║", coverage);
    println!("║ Input:      {:>20} strands ║", encoded.strand_count());
    println!("║ Output:     {:>20} strands ║", result.output_strand_count);
    println!("║ Error rate: {:>23.4}% ║", result.avg_error_rate() * 100.0);
    println!("║ Surviving:  {:>22.1}%  ║", result.survival_rate() * 100.0);
    println!("╚══════════════════════════════════════╝");
}

fn cmd_bench(file: Option<&str>) {
    let data = if let Some(f) = file {
        match fs::read(f) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error reading '{}': {}", f, e);
                std::process::exit(1);
            }
        }
    } else {
        b"The quick brown fox jumps over the lazy dog. \
          NucleOS benchmarks all available DNA codecs.".to_vec()
    };

    println!("Benchmarking codecs on {} bytes of data...\n", data.len());
    let report = benchmark_all_codecs(&data);
    println!("{}", report);
}

fn cmd_run(source: &str) {
    match nucle_lang::run_source_file(source) {
        Ok(report) => println!("{}", report),
        Err(e) => {
            eprintln!("NucleScript failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_plan(source: &str) {
    let text = match fs::read_to_string(source) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("Error reading '{}': {}", source, e);
            std::process::exit(1);
        }
    };
    match nucle_lang::compile_for_simulation(&text) {
        Ok(plan) => println!("{}", plan),
        Err(e) => {
            eprintln!("NucleScript failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_packages() {
    let manifest = nucle_lang::package::presets_manifest();
    println!("NucleScript packages\n");
    println!("{} {} ({})", manifest.name, manifest.version, manifest.import_source);
    println!("{}", manifest.description);
    println!("\nExports:");
    for export in manifest.exports {
        println!("  - {} [{}] {}", export.name, export.kind, export.description);
    }
}

fn cmd_agent(command: &str) {
    if command.is_empty() {
        println!("Usage: nucle agent <natural language command>");
        println!("\nExamples:");
        println!("  nucle agent store readme.txt with 3x redundancy");
        println!("  nucle agent retrieve readme.txt");
        println!("  nucle agent search for text files");
        println!("  nucle agent pool status");
        return;
    }

    let mut os = NucleOS::new(100);
    match Executor::run(&mut os, command) {
        Ok(report) => println!("{}", report),
        Err(e) => eprintln!("Agent error: {}", e),
    }
}

fn cmd_help() {
    println!("NucleOS — DNA Storage Engine v0.1.0\n");
    println!("Commands:");
    println!("  nucle encode <file> [-o output]           Encode a file to DNA strands");
    println!("  nucle decode <file> [-o output] -s <size> Decode DNA strands to binary");
    println!("  nucle store <file> [-r redundancy]        Store a file in DNA pool");
    println!("  nucle retrieve <name>                     Retrieve a file from DNA pool");
    println!("  nucle search <query> [-k top_k]           Search for files");
    println!("  nucle pool                                Show pool status");
    println!("  nucle simulate <file> -p <profile>        Simulate synthesis noise");
    println!("  nucle bench [file]                        Benchmark all codecs");
    println!("  nucle stress [-s size]                    Stress test all codecs");
    println!("  nucle pipeline [-f N] [-s size] [-p prof]  Full-pipeline stress test");
    println!("  nucle run <source.nsl>                    Run NucleScript source file");
    println!("  nucle plan <source.nsl>                   Show no-hardware NucleScript plan");
    println!("  nucle packages                            List released NucleScript packages");
    println!("  nucle agent <command>                     Natural language agent");
    println!("\n{}", tools::tools_help());
}

fn cmd_stress(size: usize) {
    // -----------------------------------------------------------------------
    // Data distributions
    // -----------------------------------------------------------------------
    let pangram = "The quick brown fox jumps over the lazy dog. ";
    let text_data: Vec<u8> = pangram.bytes().cycle().take(size).collect();

    let mut rng = StdRng::seed_from_u64(42);
    let random_data: Vec<u8> = (0..size).map(|_| rng.gen()).collect();

    let distributions: Vec<(&str, Vec<u8>)> = vec![
        ("all-zero",     vec![0u8; size]),
        ("all-0xFF",     vec![0xFFu8; size]),
        ("sequential",   (0..size).map(|i| i as u8).collect()),
        ("random",       random_data),
        ("text",         text_data),
        ("low-entropy",  vec![0xAAu8; size]),
    ];

    // -----------------------------------------------------------------------
    // Codecs
    // -----------------------------------------------------------------------
    let codecs: Vec<(&str, Box<dyn DnaCodec>)> = vec![
        ("ternary", Box::new(TernaryCodec::new(TernaryConfig::no_overlap()))),
        ("yin-yang", Box::new(nucle_codec::yinyang::YinYangCodec::new(
            nucle_codec::yinyang::YinYangConfig::default(),
        ))),
        ("fountain-raw", Box::new(FountainCodec::new(FountainConfig::unscreened()))),
        ("fountain-screened", Box::new(FountainCodec::new({
            let mut cfg = FountainConfig::default();
            cfg.max_screening_attempts = 100;
            cfg
        }))),
    ];

    let num_codecs = codecs.len();
    let num_dists = distributions.len();

    println!(
        "NucleOS Codec Stress Test — {} bytes × {} distributions × {} codecs\n",
        size, num_dists, num_codecs
    );

    // -----------------------------------------------------------------------
    // Table header
    // -----------------------------------------------------------------------
    println!("╔═══════════════════╤══════════════╤═════╤═════╤════════╤═══════╤══════╤══════╗");
    println!("║ Codec             │ Distribution │ Enc │ R/T │ bits/nt│  GC%  │ Hpol │ Viol ║");
    println!("╟───────────────────┼──────────────┼─────┼─────┼────────┼───────┼──────┼──────╢");

    let validator = ConstraintValidator::new(ConstraintConfig::default());

    let mut total_encode_failures = 0usize;
    let mut total_roundtrip_failures = 0usize;
    let mut total_violation_strands = 0usize;
    let mut violation_pairs = 0usize;

    // -----------------------------------------------------------------------
    // Run every (codec, distribution) combination
    // -----------------------------------------------------------------------
    for (codec_name, codec) in &codecs {
        for (dist_name, data) in &distributions {
            let start = Instant::now();
            let encode_result = codec.encode(data);
            let _encode_us = start.elapsed().as_micros();

            match encode_result {
                Err(_) => {
                    total_encode_failures += 1;
                    println!(
                        "║ {:<17} │ {:<12} │  ✗  │  —  │      — │     — │    — │    — ║",
                        codec_name, dist_name
                    );
                }
                Ok(ref collection) => {
                    // Roundtrip
                    let roundtrip_ok = codec
                        .decode(collection)
                        .map(|decoded| decoded == *data)
                        .unwrap_or(false);
                    if !roundtrip_ok {
                        total_roundtrip_failures += 1;
                    }

                    // Metrics
                    let bpn = collection.bits_per_nucleotide();
                    let gc = collection.avg_gc_content() * 100.0;
                    let hpol = collection.max_homopolymer();

                    // Constraint violations
                    let mut strand_violations = 0usize;
                    for strand in &collection.strands {
                        let result = validator.validate(strand);
                        if !result.is_valid() {
                            strand_violations += 1;
                        }
                    }
                    if strand_violations > 0 {
                        total_violation_strands += strand_violations;
                        violation_pairs += 1;
                    }

                    println!(
                        "║ {:<17} │ {:<12} │  {}  │  {}  │ {:>6.3} │ {:>4.1}% │ {:>4} │ {:>4} ║",
                        codec_name,
                        dist_name,
                        if true { "✓" } else { "✗" },
                        if roundtrip_ok { "✓" } else { "✗" },
                        bpn,
                        gc,
                        hpol,
                        strand_violations,
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Footer
    // -----------------------------------------------------------------------
    println!("╚═══════════════════╧══════════════╧═════╧═════╧════════╧═══════╧══════╧══════╝");
    println!();
    println!("Summary:");
    println!("  Codecs tested: {}", num_codecs);
    println!("  Distributions: {}", num_dists);
    println!("  Total encode failures: {}", total_encode_failures);
    println!("  Total roundtrip failures: {}", total_roundtrip_failures);
    println!(
        "  Total constraint violations: {} strands across {} codec/distribution pairs",
        total_violation_strands, violation_pairs
    );
}

fn cmd_pipeline(files: usize, size: usize, profile: &str, coverage: usize, redundancy: usize) {
    let hw_profile = match profile.to_lowercase().as_str() {
        "illumina" => HardwareProfile::Illumina,
        "nanopore" => HardwareProfile::OxfordNanopore,
        "twist" => HardwareProfile::TwistBioscience,
        "pristine" => HardwareProfile::Pristine,
        _ => {
            eprintln!("Unknown profile: {}. Use: illumina, nanopore, twist, pristine", profile);
            std::process::exit(1);
        }
    };

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           NucleOS Full-Pipeline Stress Test                 ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ Files:      {:>6}   Size: {:>6} B   Profile: {:>12} ║", files, size, profile);
    println!("║ Coverage:   {:>5}×   ECC parity: {:>2} strands               ║", coverage, redundancy);
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let mut rng = StdRng::seed_from_u64(42);

    let mut recovered = 0usize;
    let mut failed = 0usize;
    let mut total_strands = 0usize;
    let mut total_nucleotides = 0usize;
    let mut total_bytes_stored = 0usize;
    let mut failure_details: Vec<(usize, String)> = Vec::new();
    let total_start = Instant::now();

    // Progress bar width
    let bar_width = 40;

    for i in 0..files {
        // Generate unique random data per file
        let data: Vec<u8> = (0..size).map(|_| rng.gen()).collect();
        let filename = format!("stress_{:04}.bin", i);

        // Build a fresh NucleOS with noise enabled
        let noise_cfg = SimulationConfig {
            seed: 42 + i as u64,
            coverage_depth: coverage as u32,
            synthesis_profile: hw_profile,
            sequencing_profile: hw_profile,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        let mut os = NucleOS::new(10).with_noise(noise_cfg);

        // Write through full pipeline
        let write_ok = os.dna_write(&filename, &data, redundancy);

        match write_ok {
            Ok(_result) => {
                // Gather strand stats before read attempt
                let stats = os.dna_stat();
                total_strands += stats.total_strands;
                total_nucleotides += stats.total_nucleotides;
                total_bytes_stored += size;

                // Read back through full pipeline
                match os.dna_read(&filename) {
                    Ok(read_data) if read_data == data => {
                        recovered += 1;
                    }
                    Ok(_) => {
                        failed += 1;
                        failure_details.push((i, "data mismatch".into()));
                    }
                    Err(e) => {
                        failed += 1;
                        failure_details.push((i, e.to_string()));
                    }
                }
            }
            Err(e) => {
                failed += 1;
                failure_details.push((i, format!("write: {}", e)));
            }
        }

        // Progress bar
        let done = i + 1;
        let pct = done * 100 / files;
        let filled = done * bar_width / files;
        let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);
        eprint!("\r  [{}] {:>3}% ({}/{})", bar, pct, done, files);
    }
    eprintln!(); // newline after progress bar

    let elapsed = total_start.elapsed();
    let recovery_rate = if files > 0 { recovered as f64 / files as f64 * 100.0 } else { 0.0 };
    let throughput_bps = if elapsed.as_secs_f64() > 0.0 {
        total_bytes_stored as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    // Results
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                    Results                                  ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ Files tested:     {:>6}                                    ║", files);
    println!("║ Recovered:        {:>6}  ({:>5.1}%)                          ║", recovered, recovery_rate);
    println!("║ Failed:           {:>6}                                    ║", failed);
    println!("║ Total strands:    {:>6}                                    ║", total_strands);
    println!("║ Total nucleotides:{:>6}                                    ║", total_nucleotides);
    println!("║ Bytes stored:     {:>6}                                    ║", total_bytes_stored);
    println!("║ Elapsed:          {:>5.2}s                                   ║", elapsed.as_secs_f64());
    println!("║ Throughput:       {:>5.0} B/s                                ║", throughput_bps);
    println!("╚══════════════════════════════════════════════════════════════╝");

    if !failure_details.is_empty() {
        println!();
        println!("Failure details (first 10):");
        for (idx, reason) in failure_details.iter().take(10) {
            println!("  File {:>4}: {}", idx, reason);
        }
        if failure_details.len() > 10 {
            println!("  ... and {} more", failure_details.len() - 10);
        }
    }

    // Exit with error code if any failures
    if failed > 0 {
        std::process::exit(1);
    }
}
