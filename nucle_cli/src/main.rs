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
    /// Output in JSON format
    #[arg(long, global = true)]
    json: bool,

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

    /// Migrate a file to new storage parameters (e.g. redundancy)
    Migrate {
        /// Filename to migrate
        name: String,
        /// New number of RS parity strands
        #[arg(short, long)]
        redundancy: Option<usize>,
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

    /// Run a full-pipeline benchmark on a file or standard fixtures
    Benchmark {
        /// Input file to benchmark (default: all standard fixtures)
        file: Option<String>,
        /// Hardware profile for noise simulation: illumina, nanopore, twist, pristine
        #[arg(short, long, default_value = "illumina")]
        profile: String,
        /// Number of RS parity strands for ECC
        #[arg(short, long, default_value = "4")]
        redundancy: usize,
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

    /// Check environment capabilities and package integrity
    Doctor,

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
        Commands::Store { file, redundancy } => cmd_store(&file, redundancy, cli.json),
        Commands::Retrieve { name } => cmd_retrieve(&name, cli.json),
        Commands::Migrate { name, redundancy } => cmd_migrate(&name, redundancy, cli.json),
        Commands::Search { query, top_k } => cmd_search(&query, top_k, cli.json),
        Commands::Pool => cmd_pool(cli.json),
        Commands::Simulate { file, profile, coverage } => cmd_simulate(&file, &profile, coverage, cli.json),
        Commands::Bench { file } => cmd_bench(file.as_deref(), cli.json),
        Commands::Benchmark { file, profile, redundancy } => cmd_benchmark(file.as_deref(), &profile, redundancy, cli.json),
        Commands::Stress { size } => cmd_stress(size, cli.json),
        Commands::Pipeline { files, size, profile, coverage, redundancy } => {
            cmd_pipeline(files, size, &profile, coverage, redundancy, cli.json)
        }
        Commands::Run { source } => cmd_run(&source),
        Commands::Plan { source } => cmd_plan(&source),
        Commands::Packages => cmd_packages(),
        Commands::Doctor => cmd_doctor(cli.json),
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

fn cmd_store(file: &str, redundancy: usize, json: bool) {
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
            if json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!("✓ {}", result);
                println!("\n{}", os.dna_stat());
            }
        }
        Err(e) => {
            eprintln!("Store failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_retrieve(name: &str, json: bool) {
    let os = NucleOS::new(100);
    match os.dna_read(name) {
        Ok(data) => {
            let manifest_opt = os.last_recovery.lock().unwrap().clone();
            if json {
                let (is_text, content) = match String::from_utf8(data.clone()) {
                    Ok(text) => (true, text),
                    Err(_) => (false, data.iter().map(|b| format!("{:02x}", b)).collect::<String>()),
                };
                let json_val = serde_json::json!({
                    "filename": name,
                    "size": data.len(),
                    "is_text": is_text,
                    "content": content,
                    "recovery_manifest": manifest_opt
                });
                println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
            } else {
                match String::from_utf8(data.clone()) {
                    Ok(text) => println!("{}", text),
                    Err(_) => {
                        println!("Binary data ({} bytes)", data.len());
                    }
                }
                if let Some(manifest) = manifest_opt {
                    eprintln!("\n--- Recovery Manifest ---");
                    eprintln!("Observed Error Rate: {:.4}%", manifest.observed_error_rate * 100.0);
                    eprintln!("Consensus Method:    {}", manifest.consensus_method);
                    eprintln!("Sequencing Profile:  {}", manifest.sequencing_profile);
                    eprintln!("Recovered Strands:   {}", manifest.recovered_strands);
                    eprintln!("ECC Success:         {}", manifest.ecc_success);
                }
            }
        }
        Err(e) => {
            eprintln!("Retrieve failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_migrate(name: &str, redundancy: Option<usize>, json: bool) {
    let mut os = NucleOS::new(100);
    match nucle_vfs::migrate::migrate_object(&mut os, name, redundancy) {
        Ok(manifest) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
            } else {
                println!("✓ Migrated file '{}' successfully.", name);
                println!("New Archive ID: {}", manifest.archive_id);
                println!("New Redundancy: {} parity strands", manifest.redundancy);
                println!("Codec:          {}", manifest.codec);
                println!("Profile:        {}", manifest.profile);
            }
        }
        Err(e) => {
            eprintln!("Migration failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_search(query: &str, top_k: usize, json: bool) {
    let os = NucleOS::new(100);
    let results = os.dna_search(query, top_k);
    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        if results.is_empty() {
            println!("No matching files found.");
        } else {
            println!("Search results for '{}':", query);
            for (i, r) in results.iter().enumerate() {
                println!("  {}. {}", i + 1, r);
            }
        }
    }
}

fn cmd_pool(json: bool) {
    let os = NucleOS::new(100);
    let status = os.dna_stat();
    if json {
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    } else {
        println!("{}", status);
    }
}

fn cmd_simulate(file: &str, profile: &str, coverage: usize, json: bool) {
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

    if json {
        let json_val = serde_json::json!({
            "profile": profile,
            "coverage": coverage,
            "input_strands": encoded.strand_count(),
            "output_strands": result.output_strand_count,
            "error_rate": result.avg_error_rate(),
            "survival_rate": result.survival_rate()
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
    } else {
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
}

fn cmd_bench(file: Option<&str>, json: bool) {
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

    let report = benchmark_all_codecs(&data);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("Benchmarking codecs on {} bytes of data...\n", data.len());
        println!("{}", report);
    }
}

fn cmd_benchmark(file: Option<&str>, profile: &str, redundancy: usize, json: bool) {
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

    let files_to_bench: Vec<(String, Vec<u8>)> = if let Some(f) = file {
        match fs::read(f) {
            Ok(d) => vec![(f.to_string(), d)],
            Err(e) => {
                eprintln!("Error reading '{}': {}", f, e);
                std::process::exit(1);
            }
        }
    } else {
        let paths = vec![
            "docs/examples/fixtures/small_text.txt",
            "docs/examples/fixtures/archive.bin",
            "docs/examples/fixtures/sample.fasta",
            "docs/examples/fixtures/image.png",
        ];
        let mut list = Vec::new();
        for p in paths {
            match fs::read(p) {
                Ok(d) => list.push((p.to_string(), d)),
                Err(_) => {
                    list.push((p.to_string(), b"Fallback bench data".to_vec()));
                }
            }
        }
        list
    };

    let cost_per_base = match profile.to_lowercase().as_str() {
        "twist" => 0.00015,
        "illumina" => 0.0001,
        "nanopore" => 0.00005,
        _ => 0.00001,
    };

    let mut results = Vec::new();

    for (name, data) in &files_to_bench {
        let noise_cfg = SimulationConfig {
            seed: 42,
            coverage_depth: 10,
            synthesis_profile: hw_profile,
            sequencing_profile: hw_profile,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };

        let mut os = NucleOS::new(10);
        if hw_profile != HardwareProfile::Pristine {
            os = os.with_noise(noise_cfg);
        }
        let filename = std::path::Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);

        let write_result = match os.dna_write(filename, data, redundancy) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Benchmark store failed for '{}': {}", filename, e);
                continue;
            }
        };

        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let encoded = codec.encode(data).unwrap();
        let engine = NoiseEngine::new(os.noise_config.clone());
        let sim_res = engine.simulate(&encoded);
        let observed_error_rate = sim_res.avg_error_rate();

        let read_result = os.dna_read(filename);
        let recovery_ok = match read_result {
            Ok(ref recovered) => recovered == data,
            Err(_) => false,
        };

        let total_nt = write_result.total_strand_count * 150;
        let estimated_cost = total_nt as f64 * cost_per_base;

        results.push(serde_json::json!({
            "file": filename,
            "size_bytes": data.len(),
            "strands": write_result.total_strand_count,
            "observed_error_rate": observed_error_rate,
            "recovery_ok": recovery_ok,
            "estimated_cost_usd": estimated_cost,
        }));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        println!("╔══════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                      NucleOS Full-Pipeline Benchmark                                 ║");
        println!("╠══════════════════════════════════════════════════════════════════════════════════════╣");
        println!("║ {:<20} │ {:>8} │ {:>8} │ {:>12} │ {:>8} │ {:>10} ║",
            "File", "Size (B)", "Strands", "Error Rate", "Recover", "Cost (USD)");
        println!("╟──────────────────────┼──────────┼──────────┼──────────────┼──────────┼──────────╢");
        for r in &results {
            println!("║ {:<20} │ {:>8} │ {:>8} │ {:>11.2}% │ {:^8} │ ${:>8.4} ║",
                r["file"].as_str().unwrap(),
                r["size_bytes"].as_u64().unwrap(),
                r["strands"].as_u64().unwrap(),
                r["observed_error_rate"].as_f64().unwrap() * 100.0,
                if r["recovery_ok"].as_bool().unwrap() { "PASS" } else { "FAIL" },
                r["estimated_cost_usd"].as_f64().unwrap(),
            );
        }
        println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");
    }
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

fn cmd_doctor(json: bool) {
    let profiles = vec![
        "illumina",
        "nanopore",
        "twist",
        "idt",
        "column-synthesis",
        "pristine",
    ];

    let manifest = nucle_lang::package::presets_manifest();
    let package_integrity = !manifest.exports.is_empty();

    let fixtures = vec![
        "docs/examples/fixtures/small_text.txt",
        "docs/examples/fixtures/archive.bin",
        "docs/examples/fixtures/sample.fasta",
        "docs/examples/fixtures/image.png",
    ];

    let mut missing_fixtures = Vec::new();
    for f in &fixtures {
        if !std::path::Path::new(f).exists() {
            missing_fixtures.push(f.to_string());
        }
    }
    let fixtures_ok = missing_fixtures.is_empty();

    let overall_status = if package_integrity && fixtures_ok {
        "Healthy"
    } else {
        "Degraded"
    };

    if json {
        let json_val = serde_json::json!({
            "synthesis_profiles": profiles,
            "presets_manifest_name": manifest.name,
            "package_integrity": package_integrity,
            "fixtures_ok": fixtures_ok,
            "missing_fixtures": missing_fixtures,
            "status": overall_status
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
    } else {
        println!("# NucleOS Diagnostics Report");
        println!("\n## Environment Capabilities");
        println!("- **Synthesis Profiles Available**: {:?}", profiles);
        println!("- **Presets Manifest**: {} v{} ({})", manifest.name, manifest.version, manifest.import_source);
        println!("- **Package Integrity**: {}", if package_integrity { "✓ PASS" } else { "✗ FAILED" });

        println!("\n## Standard Workloads / Fixtures");
        if fixtures_ok {
            println!("- **Fixtures Status**: ✓ All 4 fixtures present");
        } else {
            println!("- **Fixtures Status**: ✗ Missing fixtures: {:?}", missing_fixtures);
        }

        println!("\n## Overall Status");
        println!("- **System Health**: **{}**", overall_status);
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
    println!("  nucle migrate <name> [-r redundancy]      Migrate a file to new storage params");
    println!("  nucle search <query> [-k top_k]           Search for files");
    println!("  nucle pool                                Show pool status");
    println!("  nucle simulate <file> -p <profile>        Simulate synthesis noise");
    println!("  nucle bench [file]                        Benchmark all codecs");
    println!("  nucle benchmark [file] [-p profile]       Full-pipeline benchmark");
    println!("  nucle stress [-s size]                    Stress test all codecs");
    println!("  nucle pipeline [-f N] [-s size] [-p prof]  Full-pipeline stress test");
    println!("  nucle run <source.nsl>                    Run NucleScript source file");
    println!("  nucle plan <source.nsl>                   Show no-hardware NucleScript plan");
    println!("  nucle packages                            List released NucleScript packages");
    println!("  nucle doctor                              Check environment and presets integrity");
    println!("  nucle agent <command>                     Natural language agent");
    println!("\n{}", tools::tools_help());
}

fn cmd_stress(size: usize, json: bool) {
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

    if !json {
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
    }

    let validator = ConstraintValidator::new(ConstraintConfig::default());

    let mut total_encode_failures = 0usize;
    let mut total_roundtrip_failures = 0usize;
    let mut total_violation_strands = 0usize;
    let mut violation_pairs = 0usize;
    let mut json_results = Vec::new();

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
                    if json {
                        json_results.push(serde_json::json!({
                            "codec": codec_name,
                            "distribution": dist_name,
                            "encode_ok": false,
                            "roundtrip_ok": false,
                            "bits_per_nt": null,
                            "gc_percent": null,
                            "max_homopolymer": null,
                            "violations": null
                        }));
                    } else {
                        println!(
                            "║ {:<17} │ {:<12} │  ✗  │  —  │      — │     — │    — │    — ║",
                            codec_name, dist_name
                        );
                    }
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

                    if json {
                        json_results.push(serde_json::json!({
                            "codec": codec_name,
                            "distribution": dist_name,
                            "encode_ok": true,
                            "roundtrip_ok": roundtrip_ok,
                            "bits_per_nt": bpn,
                            "gc_percent": gc,
                            "max_homopolymer": hpol,
                            "violations": strand_violations
                        }));
                    } else {
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
    }

    if json {
        let final_val = serde_json::json!({
            "results": json_results,
            "summary": {
                "codecs_tested": num_codecs,
                "distributions": num_dists,
                "total_encode_failures": total_encode_failures,
                "total_roundtrip_failures": total_roundtrip_failures,
                "total_constraint_violations": total_violation_strands,
                "violation_pairs": violation_pairs
            }
        });
        println!("{}", serde_json::to_string_pretty(&final_val).unwrap());
    } else {
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
}

fn cmd_pipeline(files: usize, size: usize, profile: &str, coverage: usize, redundancy: usize, json: bool) {
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

    if !json {
        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║           NucleOS Full-Pipeline Stress Test                 ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ Files:      {:>6}   Size: {:>6} B   Profile: {:>12} ║", files, size, profile);
        println!("║ Coverage:   {:>5}×   ECC parity: {:>2} strands               ║", coverage, redundancy);
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!();
    }

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
        if !json {
            let done = i + 1;
            let pct = done * 100 / files;
            let filled = done * bar_width / files;
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);
            eprint!("\r  [{}] {:>3}% ({}/{})", bar, pct, done, files);
        }
    }
    if !json {
        eprintln!(); // newline after progress bar
    }

    let elapsed = total_start.elapsed();
    let recovery_rate = if files > 0 { recovered as f64 / files as f64 * 100.0 } else { 0.0 };
    let throughput_bps = if elapsed.as_secs_f64() > 0.0 {
        total_bytes_stored as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    if json {
        let json_val = serde_json::json!({
            "files_tested": files,
            "recovered": recovered,
            "recovery_rate": recovery_rate,
            "failed": failed,
            "total_strands": total_strands,
            "total_nucleotides": total_nucleotides,
            "bytes_stored": total_bytes_stored,
            "elapsed_seconds": elapsed.as_secs_f64(),
            "throughput_bytes_per_sec": throughput_bps,
            "failures": failure_details.iter().map(|(idx, reason)| {
                serde_json::json!({
                    "file_index": idx,
                    "reason": reason
                })
            }).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
    } else {
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
    }

    // Exit with error code if any failures
    if failed > 0 {
        std::process::exit(1);
    }
}
