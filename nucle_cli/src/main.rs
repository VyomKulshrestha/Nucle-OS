//! # NucleOS CLI — Full Stack DNA Storage Interface
//!
//! Unified command-line tool tying all layers together.

use clap::{Parser, Subcommand};
use nucle_vfs::syscall::NucleOS;
use nucle_agent::executor::Executor;
use nucle_agent::tools;
use nucle_codec::base::DnaCodec;
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::benchmark::benchmark_all_codecs;
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;
use std::fs;

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
    println!("  nucle agent <command>                     Natural language agent");
    println!("\n{}", tools::tools_help());
}
