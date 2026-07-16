//! # NucleOS CLI — Full Stack DNA Storage Interface
//!
//! Unified command-line tool tying all layers together.

use clap::{Parser, Subcommand};
use nucle_hardware::JobHandle;
use nucle_vfs::syscall::NucleOS;
use nucle_agent::executor::Executor;
use nucle_agent::tools;
use nucle_codec::base::DnaCodec;
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::fountain::{FountainCodec, FountainConfig};
use nucle_codec::yinyang::{YinYangCodec, YinYangConfig};
use nucle_codec::constraints::{ConstraintValidator, ConstraintConfig};
use nucle_codec::benchmark::{benchmark_codec, BenchmarkResult, ComparisonReport};
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

    /// Directory the DNA storage pool persists to. Defaults to
    /// NUCLEOS_POOL_DIR if set, else a project-local .nucleos/ directory
    /// (created on first use, like .git/).
    #[arg(long, global = true)]
    pool_dir: Option<String>,

    /// Passphrase to unlock an encrypted pool (see `nucle encrypt-pool`).
    /// Defaults to NUCLEOS_POOL_PASSPHRASE if set. Ignored (with a note)
    /// if the pool isn't actually encrypted. Note: passing this directly
    /// on the command line can leak via shell history or a process
    /// listing (`ps`) -- the environment variable is marginally safer,
    /// though not perfectly so either.
    #[arg(long, global = true)]
    pool_key: Option<String>,

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

    /// Migrate a file to new storage parameters (e.g. redundancy, codec)
    Migrate {
        /// Filename to migrate
        name: String,
        /// New number of RS parity strands
        #[arg(short, long)]
        redundancy: Option<usize>,
        /// New codec name (only 'ternary-rotating-cipher' is supported today)
        #[arg(short, long)]
        codec: Option<String>,
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

    /// List stored files, optionally filtered to a name prefix (e.g. "docs/")
    List {
        /// Only list files whose name starts with this prefix
        prefix: Option<String>,
    },

    /// Show or set the pool's capacity limit (in total nucleotides)
    Capacity {
        /// New capacity limit to set. Omit to just show the current limit and usage.
        max_nucleotides: Option<usize>,
        /// Clear the capacity limit (unlimited growth) -- takes precedence over max_nucleotides
        #[arg(long)]
        unlimited: bool,
    },

    /// Show the pool's audit log -- every store/retrieve/delete event,
    /// including failed ones, oldest first
    Audit {
        /// Only show the last N events (omit to show the whole log)
        #[arg(long)]
        tail: Option<usize>,
    },

    /// Show or edit the OS usernames allowed to pass --confirm for
    /// cost-bearing/destructive `nucle hardware export` requests against
    /// this pool. Omit both flags to just show the current allowlist --
    /// empty means unrestricted, matching today's default.
    ConfirmUsers {
        /// Add a username to the allowlist
        #[arg(long)]
        add: Option<String>,
        /// Remove a username from the allowlist
        #[arg(long)]
        remove: Option<String>,
    },

    /// Enable encryption at rest for this pool's stored state, protected
    /// by a passphrase (--pool-key or NUCLEOS_POOL_PASSPHRASE). Errors if
    /// the pool is already encrypted.
    EncryptPool,

    /// Disable encryption at rest for this pool, decrypting its state
    /// back to plaintext. Requires the pool's current passphrase
    /// (--pool-key or NUCLEOS_POOL_PASSPHRASE). Errors if the pool isn't
    /// encrypted.
    DecryptPool,

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
        /// Hardware profile used to estimate recovery probability and cost:
        /// illumina, nanopore, twist, idt, column-synthesis, pristine
        #[arg(short, long, default_value = "illumina")]
        profile: String,
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

    /// Run compile-only validation on a NucleScript source file
    Check {
        /// NucleScript source file to check
        source: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Explain safety confirmations and optimizer notes for a NucleScript source file
    Explain {
        /// NucleScript source file to explain
        source: String,
    },

    /// Format a NucleScript source file in its one canonical style
    Fmt {
        /// NucleScript source file to format, or '-' to read from stdin
        /// (stdin mode always prints to stdout; --check/--write don't apply)
        source: String,
        /// Exit non-zero if the file isn't already canonically formatted,
        /// without changing it -- for CI
        #[arg(long)]
        check: bool,
        /// Rewrite the file in place instead of printing to stdout
        #[arg(long)]
        write: bool,
    },

    /// Run `test "..." { ... }` blocks in a NucleScript source file
    Test {
        /// NucleScript source file whose test blocks to run
        source: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Run a NucleScript source file (.nsl)
    Run {
        /// NucleScript source file to compile and execute
        source: String,
    },

    /// Generate Markdown documentation from a NucleScript source file's `///` doc comments
    Doc {
        /// NucleScript source file to document
        source: String,
        /// Write the generated Markdown to this file instead of stdout
        #[arg(long)]
        output: Option<String>,
    },

    /// Scaffold a new NucleScript project directory
    New {
        /// Directory to create (must not already exist)
        name: String,
    },

    /// Compile a NucleScript source file into a no-hardware simulation plan
    Plan {
        /// NucleScript source file to compile
        source: String,
    },

    /// List released NucleScript packages bundled with this repository
    Packages,

    /// Manage NucleScript packages (install, verify)
    Package {
        #[command(subcommand)]
        action: PackageAction,
    },

    /// Manage laboratory DNA synthesis/sequencing hardware bridge
    Hardware {
        #[command(subcommand)]
        subcommand: HardwareSubcommand,
    },

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

#[derive(Subcommand, Debug, Clone)]
enum PackageAction {
    /// List every package known to packages/registry.json
    List,
    /// Install a package by name from packages/registry.json (e.g. @nuclescript/presets)
    Install {
        /// Package name (e.g. @nuclescript/presets) or import source (e.g. nuclescript/presets)
        name: String,
    },
    /// Verify a registered package's manifest and, if nucle.lock exists, its checksum
    Verify {
        /// Package name (e.g. @nuclescript/presets) or import source
        name: String,
    },
    /// Inspect a package manifest and its exports
    Inspect {
        /// Package name (e.g. @nuclescript/presets) or import source
        name: String,
    },
    /// Write/update nucle.lock with checksums for every package in the registry
    Lock,
}

#[derive(Subcommand, Debug, Clone)]
enum HardwareSubcommand {
    /// Export batch requests from one or more NucleScript files to a JSON file
    Export {
        /// NucleScript file(s) to compile and extract requests from. Passing
        /// more than one submits every file's batch concurrently instead of
        /// one at a time.
        #[arg(required = true, num_args = 1..)]
        source: Vec<String>,
        /// Output path for the exported JSON batch file (used by the
        /// file-export provider). With multiple sources, each gets its own
        /// path (an index is inserted before the extension).
        #[arg(short, long, default_value = "batch.json")]
        output: String,
        /// Provider to submit the batch to: 'file-export' (default), 'mock',
        /// or 'mock-delayed' (simulates real hardware latency on a
        /// background thread — see --simulated-delay-ms). Vendor names (e.g.
        /// 'twist') are accepted but no vendor-specific adapter exists yet,
        /// so they fall back to file-export.
        #[arg(short, long, default_value = "file-export")]
        provider: String,
        /// Required whenever the batch contains a Synthesis, Sequencing, or
        /// Destructive request — acknowledges the operator reviewed a
        /// cost-bearing or destructive submission before it proceeds.
        #[arg(long)]
        confirm: bool,
        /// Simulated hardware latency in milliseconds, only used by
        /// '--provider mock-delayed'.
        #[arg(long, default_value_t = 500)]
        simulated_delay_ms: u64,
    },
}

fn main() {
    let cli = Cli::parse();
    let pool_dir = resolve_pool_dir(cli.pool_dir.as_deref());
    let passphrase = resolve_passphrase(cli.pool_key.as_deref());

    match cli.command {
        Commands::Encode { file, output } => cmd_encode(&file, output.as_deref()),
        Commands::Decode { file, output, size } => cmd_decode(&file, output.as_deref(), size),
        Commands::Store { file, redundancy } => cmd_store(&file, redundancy, &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Retrieve { name } => cmd_retrieve(&name, &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Migrate { name, redundancy, codec } => cmd_migrate(&name, redundancy, codec.as_deref(), &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Search { query, top_k } => cmd_search(&query, top_k, &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Pool => cmd_pool(&pool_dir, passphrase.as_deref(), cli.json),
        Commands::List { prefix } => cmd_list(prefix.as_deref().unwrap_or(""), &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Capacity { max_nucleotides, unlimited } => cmd_capacity(max_nucleotides, unlimited, &pool_dir, passphrase.as_deref(), cli.json),
        Commands::Audit { tail } => cmd_audit(tail, &pool_dir, cli.json),
        Commands::ConfirmUsers { add, remove } => cmd_confirm_users(add, remove, &pool_dir, cli.json),
        Commands::EncryptPool => cmd_encrypt_pool(&pool_dir, passphrase.as_deref(), cli.json),
        Commands::DecryptPool => cmd_decrypt_pool(&pool_dir, passphrase.as_deref(), cli.json),
        Commands::Simulate { file, profile, coverage } => cmd_simulate(&file, &profile, coverage, cli.json),
        Commands::Bench { file, profile } => cmd_bench(file.as_deref(), &profile, cli.json),
        Commands::Benchmark { file, profile, redundancy } => cmd_benchmark(file.as_deref(), &profile, redundancy, cli.json),
        Commands::Stress { size } => cmd_stress(size, cli.json),
        Commands::Pipeline { files, size, profile, coverage, redundancy } => {
            cmd_pipeline(files, size, &profile, coverage, redundancy, cli.json)
        }
        Commands::Check { source, json } => cmd_check(&source, cli.json || json),
        Commands::Explain { source } => cmd_explain(&source),
        Commands::Fmt { source, check, write } => cmd_fmt(&source, check, write),
        Commands::Test { source, json } => cmd_test(&source, cli.json || json),
        Commands::Run { source } => cmd_run(&source),
        Commands::Doc { source, output } => cmd_doc(&source, output.as_deref()),
        Commands::New { name } => cmd_new(&name),
        Commands::Plan { source } => cmd_plan(&source),
        Commands::Packages => cmd_packages(),
        Commands::Package { action } => cmd_package(action, cli.json),
        Commands::Hardware { subcommand } => cmd_hardware(subcommand, &pool_dir, cli.json),
        Commands::Doctor => cmd_doctor(cli.json),
        Commands::Agent { command } => cmd_agent(&command.join(" "), &pool_dir, passphrase.as_deref()),
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

/// Resolves where a pool's durable state lives, in priority order: an
/// explicit `--pool-dir` flag, then `NUCLEOS_POOL_DIR`, then a
/// project-local `.nucleos/` directory next to wherever the command runs
/// (created on first use, mirroring `.git/`) -- see `NucleOS::open`/
/// `persist` for what actually gets written there.
fn resolve_pool_dir(explicit: Option<&str>) -> std::path::PathBuf {
    if let Some(path) = explicit {
        return std::path::PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("NUCLEOS_POOL_DIR") {
        return std::path::PathBuf::from(path);
    }
    std::path::PathBuf::from(".nucleos")
}

/// Resolves the passphrase for an encrypted pool, in priority order: an
/// explicit `--pool-key` flag, then `NUCLEOS_POOL_PASSPHRASE`. `None` if
/// neither is set -- fine for an unencrypted pool, an error (from
/// `NucleOS::open`'s own clear message) for an encrypted one.
fn resolve_passphrase(explicit: Option<&str>) -> Option<String> {
    explicit.map(|s| s.to_string()).or_else(|| std::env::var("NUCLEOS_POOL_PASSPHRASE").ok())
}

/// Opens the pool at `pool_dir`, exiting with a clear error if it can't be
/// read -- shared by every command that touches the real, durable pool.
/// `passphrase` unlocks an encrypted pool (see `nucle encrypt-pool`); a
/// passphrase given for a pool that isn't actually encrypted is a no-op,
/// noted rather than silently ignored, in case the caller expected it to
/// matter.
fn open_pool(pool_dir: &std::path::Path, passphrase: Option<&str>) -> NucleOS {
    if passphrase.is_some() && !nucle_vfs::crypto::is_encrypted(pool_dir) {
        eprintln!("Note: --pool-key/NUCLEOS_POOL_PASSPHRASE given but this pool isn't encrypted; ignoring.");
    }
    let result = match passphrase {
        Some(p) => NucleOS::open_encrypted(pool_dir, 100, p),
        None => NucleOS::open(pool_dir, 100),
    };
    match result {
        Ok(os) => os,
        Err(e) => {
            eprintln!("Failed to open pool at '{}': {}", pool_dir.display(), e);
            std::process::exit(1);
        }
    }
}

/// Persists `os` back to `pool_dir`, exiting with a clear error if it
/// fails -- a persist failure means the operation that just ran (however
/// successful in memory) won't actually be visible to a later invocation,
/// so it's treated as seriously as the operation itself failing.
fn persist_pool(os: &mut NucleOS, pool_dir: &std::path::Path) {
    if let Err(e) = os.persist(pool_dir) {
        eprintln!("Operation succeeded but failed to persist pool state to '{}': {}", pool_dir.display(), e);
        std::process::exit(1);
    }
}

fn cmd_store(file: &str, redundancy: usize, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let data = match fs::read(file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", file, e);
            std::process::exit(1);
        }
    };

    // A relative source path (e.g. "docs/readme.txt") is stored under that
    // same path-like name, giving the pool's own flat namespace a
    // directory-like structure -- "docs/readme.txt" and
    // "downloads/readme.txt" don't collide, and `nucle list docs/` finds
    // just the first. An absolute path strips to just the leaf name, since
    // the local filesystem's own absolute structure isn't something a
    // caller usually wants literally preserved in pool metadata.
    let path = std::path::Path::new(file);
    let filename: &str = if path.is_absolute() {
        path.file_name().and_then(|n| n.to_str()).unwrap_or(file)
    } else {
        file
    };

    let mut os = open_pool(pool_dir, passphrase);
    match os.dna_write(filename, &data, redundancy) {
        Ok(result) => {
            persist_pool(&mut os, pool_dir);
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

fn cmd_retrieve(name: &str, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let mut os = open_pool(pool_dir, passphrase);
    match os.dna_read(name) {
        Ok(data) => {
            // dna_read updates the file's recovery manifest -- persist so
            // that update (not just the original write) survives.
            persist_pool(&mut os, pool_dir);
            let manifest_opt = os.catalog.get_by_name(name)
                .and_then(|f| f.manifest.as_ref())
                .and_then(|m| m.recovery_manifest.clone());
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
                    if !manifest.observed_error_distribution.is_empty() {
                        let flagged = manifest.observed_error_distribution.iter().filter(|(_, r)| *r > 0.0).count();
                        eprintln!("Positions w/ errors: {} of {}", flagged, manifest.observed_error_distribution.len());
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Retrieve failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_migrate(name: &str, redundancy: Option<usize>, codec: Option<&str>, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let mut os = open_pool(pool_dir, passphrase);
    match nucle_vfs::migrate::migrate_object(&mut os, name, redundancy, codec) {
        Ok(manifest) => {
            persist_pool(&mut os, pool_dir);
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

fn cmd_search(query: &str, top_k: usize, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let os = open_pool(pool_dir, passphrase);
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

fn cmd_pool(pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let os = open_pool(pool_dir, passphrase);
    let status = os.dna_stat();
    if json {
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    } else {
        println!("{}", status);
    }
}

fn cmd_list(prefix: &str, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let os = open_pool(pool_dir, passphrase);
    let files = os.dna_list(prefix);
    if json {
        println!("{}", serde_json::to_string_pretty(&files).unwrap());
    } else if files.is_empty() {
        if prefix.is_empty() {
            println!("No files stored.");
        } else {
            println!("No files under '{}'.", prefix);
        }
    } else {
        for f in &files {
            println!(
                "  {} ({} B, {}d+{}p strands, {:.1}×)",
                f.filename, f.size, f.data_strands, f.parity_strands, f.redundancy
            );
        }
    }
}

fn cmd_capacity(max_nucleotides: Option<usize>, unlimited: bool, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let mut os = open_pool(pool_dir, passphrase);

    // Any explicit setting request (either --unlimited or a value)
    // persists the new configuration; omitting both just reports status.
    if unlimited {
        os.set_max_nucleotides(None);
        persist_pool(&mut os, pool_dir);
    } else if let Some(max) = max_nucleotides {
        os.set_max_nucleotides(Some(max));
        persist_pool(&mut os, pool_dir);
    }

    let used = os.dna_stat().total_nucleotides;
    let limit = os.max_nucleotides();
    if json {
        let json_val = serde_json::json!({
            "max_nucleotides": limit,
            "used_nucleotides": used,
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
    } else {
        match limit {
            Some(max) => println!("Capacity: {} / {} nucleotides used", used, max),
            None => println!("Capacity: {} nucleotides used (unlimited)", used),
        }
    }
}

fn cmd_audit(tail: Option<usize>, pool_dir: &std::path::Path, json: bool) {
    let events = match nucle_vfs::audit::read_events(pool_dir) {
        Ok(events) => events,
        Err(e) => {
            eprintln!("Failed to read audit log at '{}': {}", pool_dir.display(), e);
            std::process::exit(1);
        }
    };

    let shown: Vec<_> = match tail {
        Some(n) => {
            let start = events.len().saturating_sub(n);
            events[start..].to_vec()
        }
        None => events,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&shown).unwrap());
    } else if shown.is_empty() {
        println!("No audit events recorded yet.");
    } else {
        for e in &shown {
            let status = if e.success { "OK" } else { "FAILED" };
            let archive = e.archive_id.as_deref().unwrap_or("-");
            println!(
                "  [{}] {:<6} {:<8} {} ({}) -- {}",
                e.timestamp, status, e.operation, e.filename, archive, e.detail
            );
        }
    }
}

fn cmd_confirm_users(add: Option<String>, remove: Option<String>, pool_dir: &std::path::Path, json: bool) {
    let mut policy = match nucle_vfs::confirm_policy::load(pool_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to read pool config at '{}': {}", pool_dir.display(), e);
            std::process::exit(1);
        }
    };

    let mut changed = false;
    if let Some(user) = add {
        if !policy.allowed_users.contains(&user) {
            policy.allowed_users.push(user);
        }
        changed = true;
    }
    if let Some(user) = remove {
        policy.allowed_users.retain(|u| u != &user);
        changed = true;
    }

    if changed {
        if let Err(e) = nucle_vfs::confirm_policy::save(pool_dir, &policy) {
            eprintln!("Failed to save pool config to '{}': {}", pool_dir.display(), e);
            std::process::exit(1);
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&policy).unwrap());
    } else if policy.allowed_users.is_empty() {
        println!("No confirm-allowlist configured -- any OS user may pass --confirm.");
    } else {
        println!("Confirm-allowlist: {}", policy.allowed_users.join(", "));
    }
}

fn cmd_encrypt_pool(pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let Some(passphrase) = passphrase else {
        eprintln!("Encrypting a pool requires a passphrase: pass --pool-key <passphrase> or set NUCLEOS_POOL_PASSPHRASE.");
        std::process::exit(1);
    };

    // Opened with no passphrase: if the pool is already encrypted, this
    // fails with open()'s own clear "is encrypted" error, which is
    // exactly the right refusal here too.
    let mut os = open_pool(pool_dir, None);
    match os.enable_encryption(pool_dir, passphrase) {
        Ok(()) => {
            if json {
                println!("{}", serde_json::json!({"status": "encrypted"}));
            } else {
                println!("✓ Pool at '{}' is now encrypted at rest.", pool_dir.display());
            }
        }
        Err(e) => {
            eprintln!("Failed to encrypt pool: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_decrypt_pool(pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let Some(passphrase) = passphrase else {
        eprintln!("Decrypting a pool requires its current passphrase: pass --pool-key <passphrase> or set NUCLEOS_POOL_PASSPHRASE.");
        std::process::exit(1);
    };

    let mut os = open_pool(pool_dir, Some(passphrase));
    match os.disable_encryption(pool_dir) {
        Ok(()) => {
            if json {
                println!("{}", serde_json::json!({"status": "decrypted"}));
            } else {
                println!("✓ Pool at '{}' is no longer encrypted.", pool_dir.display());
            }
        }
        Err(e) => {
            eprintln!("Failed to decrypt pool: {}", e);
            std::process::exit(1);
        }
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

/// Parse a hardware profile name shared across bench/simulate/pipeline commands.
fn parse_hw_profile(profile: &str) -> HardwareProfile {
    match profile.to_lowercase().as_str() {
        "illumina" => HardwareProfile::Illumina,
        "nanopore" => HardwareProfile::OxfordNanopore,
        "twist" => HardwareProfile::TwistBioscience,
        "idt" => HardwareProfile::Idt,
        "column-synthesis" => HardwareProfile::ColumnSynthesis,
        "pristine" => HardwareProfile::Pristine,
        _ => {
            eprintln!("Unknown profile: {}. Use: illumina, nanopore, twist, idt, column-synthesis, pristine", profile);
            std::process::exit(1);
        }
    }
}

/// Estimated synthesis cost per nucleotide (USD) for a hardware profile.
/// A lookup, not a simulation output — cost isn't derivable from the noise
/// model, but the function is profile-aware so callers stay centralized.
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

/// Monte-Carlo estimate of encode→noise→decode roundtrip success under a
/// hardware profile. Real signal (not a placeholder): each trial actually
/// runs the noise engine and the codec's decoder.
fn estimate_recovery_probability(codec: &dyn DnaCodec, data: &[u8], profile: HardwareProfile, trials: u64) -> f64 {
    if profile == HardwareProfile::Pristine {
        return 1.0;
    }
    let Ok(encoded) = codec.encode(data) else {
        return 0.0;
    };
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
        let engine = NoiseEngine::new(config);
        let sim_result = engine.simulate(&encoded);
        let noisy = sim_result.pool.to_strand_collection(data.len());
        if let Ok(decoded) = codec.decode(&noisy) {
            if decoded == data {
                successes += 1;
            }
        }
    }
    successes as f64 / trials as f64
}

/// Fill in the `recovery_probability`/`estimated_cost_usd` fields that
/// `nucle_codec::benchmark::benchmark_codec` intentionally leaves `None`
/// (it can't depend on `nucle_synth`). This is the one place with access
/// to codec + synth + ecc together, so it does the real computation.
fn enrich_benchmark_result(result: &mut BenchmarkResult, codec: &dyn DnaCodec, data: &[u8], profile: HardwareProfile) {
    result.recovery_probability = Some(estimate_recovery_probability(codec, data, profile, 20));
    result.estimated_cost_usd = Some(result.total_nucleotides as f64 * profile_cost_per_base(profile));
}

fn cmd_bench(file: Option<&str>, profile: &str, json: bool) {
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

    let hw_profile = parse_hw_profile(profile);

    let codecs: Vec<Box<dyn DnaCodec>> = vec![
        Box::new(TernaryCodec::new(TernaryConfig::no_overlap())),
        Box::new(TernaryCodec::new(TernaryConfig::default())),
        Box::new(YinYangCodec::new(YinYangConfig::default())),
        Box::new(FountainCodec::new(FountainConfig::default())),
        Box::new(FountainCodec::new(FountainConfig {
            overhead: 1.50,
            ..FountainConfig::unscreened()
        })),
    ];

    let mut results = Vec::new();
    for codec in &codecs {
        if let Ok(mut result) = benchmark_codec(codec.as_ref(), &data) {
            enrich_benchmark_result(&mut result, codec.as_ref(), &data, hw_profile);
            results.push(result);
        }
    }
    let report = ComparisonReport::new(results);

    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("Benchmarking codecs on {} bytes of data (profile: {})...\n", data.len(), profile);
        println!("{}", report);
    }
}

fn cmd_benchmark(file: Option<&str>, profile: &str, redundancy: usize, json: bool) {
    let hw_profile = parse_hw_profile(profile);

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

        // Real codec-level metrics (GC distribution, homopolymer violations,
        // density) instead of re-deriving them ad hoc — reuses the same
        // benchmark_codec() the `bench` command uses, avoiding duplicate logic.
        let codec_bench = benchmark_codec(&codec, data).ok();
        let total_nt = codec_bench.as_ref()
            .map(|b| b.total_nucleotides)
            .unwrap_or(write_result.total_strand_count * 150);
        let estimated_cost = total_nt as f64 * profile_cost_per_base(hw_profile);

        results.push(serde_json::json!({
            "file": filename,
            "size_bytes": data.len(),
            "strands": write_result.total_strand_count,
            "observed_error_rate": observed_error_rate,
            "recovery_ok": recovery_ok,
            "estimated_cost_usd": estimated_cost,
            "avg_gc_content": codec_bench.as_ref().map(|b| b.avg_gc_content),
            "gc_distribution": codec_bench.as_ref().map(|b| b.gc_distribution.clone()),
            "max_homopolymer": codec_bench.as_ref().map(|b| b.max_homopolymer),
            "homopolymer_violation_count": codec_bench.as_ref().map(|b| b.homopolymer_violation_count),
        }));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        println!("╔══════════════════════════════════════════════════════════════════════════════════════════════════╗");
        println!("║                              NucleOS Full-Pipeline Benchmark                                      ║");
        println!("╠══════════════════════════════════════════════════════════════════════════════════════════════════╣");
        println!("║ {:<18} │ {:>7} │ {:>7} │ {:>10} │ {:>7} │ {:>9} │ {:>6} │ {:>6} ║",
            "File", "Size(B)", "Strands", "Error Rate", "Recover", "Cost(USD)", "GC%", "HpolV");
        println!("╟────────────────────┼─────────┼─────────┼────────────┼─────────┼───────────┼────────┼────────╢");
        for r in &results {
            println!("║ {:<18} │ {:>7} │ {:>7} │ {:>9.2}% │ {:>7} │ ${:>8.4} │ {:>5.1}% │ {:>6} ║",
                r["file"].as_str().unwrap(),
                r["size_bytes"].as_u64().unwrap(),
                r["strands"].as_u64().unwrap(),
                r["observed_error_rate"].as_f64().unwrap() * 100.0,
                if r["recovery_ok"].as_bool().unwrap() { "PASS" } else { "FAIL" },
                r["estimated_cost_usd"].as_f64().unwrap(),
                r["avg_gc_content"].as_f64().unwrap_or(0.0) * 100.0,
                r["homopolymer_violation_count"].as_u64().unwrap_or(0),
            );
        }
        println!("╚══════════════════════════════════════════════════════════════════════════════════════════════════╝");
    }
}

/// Warn (never hard-fail) if any package a `.nsl` source file imports has
/// drifted from what `nucle.lock` recorded. Best-effort: if the source
/// can't be read/parsed, or there's no lock file, this silently does
/// nothing — the real compile step reports the actual error either way.
fn warn_on_lock_mismatch(source_path: &str) {
    let Some(lock) = load_lock_file() else {
        return;
    };
    let Ok(content) = fs::read_to_string(source_path) else {
        return;
    };
    let Ok(tokens) = nucle_lang::Lexer::new(&content).tokenize() else {
        return;
    };
    let Ok(program) = nucle_lang::Parser::new(tokens).parse_program() else {
        return;
    };

    let mut checked = std::collections::HashSet::new();
    for decl in &program.declarations {
        let nucle_lang::Declaration::Import(import) = decl else {
            continue;
        };
        if !checked.insert(import.source.clone()) {
            continue;
        }
        let Some(locked) = lock.find(&import.source) else {
            continue;
        };
        let Some(manifest_json) = nucle_lang::package::find_package_manifest_json(&import.source) else {
            continue;
        };
        let sources = nucle_lang::package::checksum_sources(&import.source);
        let actual = nucle_lang::lockfile::compute_checksum(manifest_json, &sources);
        if actual != locked.checksum {
            eprintln!(
                "Warning: package '{}' has drifted from {} (locked={}, actual={}). Run 'nucle package lock' to refresh.",
                import.source, nucle_lang::lockfile::LOCK_FILE_NAME, locked.checksum, actual
            );
        }
    }
}

fn cmd_check(source: &str, json: bool) {
    warn_on_lock_mismatch(source);
    let report = match nucle_lang::check_source_file(source) {
        Ok(report) => report,
        Err(e) => {
            if json {
                let err_report = serde_json::json!({
                    "ok": false,
                    "diagnostics": [{
                        "level": "error",
                        "message": e.to_string(),
                    }]
                });
                println!("{}", serde_json::to_string_pretty(&err_report).unwrap());
            } else {
                eprintln!("NucleScript check failed: {}", e);
            }
            std::process::exit(1);
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        if report.ok {
            println!("Check status: OK (no errors or warnings)");
        } else {
            let source_text = std::fs::read_to_string(source).unwrap_or_default();
            for diagnostic in &report.diagnostics {
                println!("{}", nucle_lang::diagnostics::render_snippet(source, &source_text, diagnostic));
            }
        }
    }

    if !report.ok {
        std::process::exit(1);
    }
}

fn cmd_explain(source: &str) {
    let source_content = match std::fs::read_to_string(source) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", source, e);
            std::process::exit(1);
        }
    };

    let tokens = match nucle_lang::lexer::Lexer::new(&source_content).tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lex error: {}", e);
            std::process::exit(1);
        }
    };

    let program = match nucle_lang::parser::Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    };

    let summary = nucle_lang::effects::effect_summary(&program);
    let mir_program = nucle_lang::middle::lower_program(&program);
    let notes = mir_program.notes;

    let explanation = nucle_lang::diagnostics::generate_explanation(&notes, &summary);
    println!("{}", explanation);
}

fn cmd_fmt(source: &str, check: bool, write: bool) {
    // `-` reads the buffer to format from stdin instead of a file --
    // what the VS Code extension's format-on-save provider uses, since
    // it must format the in-memory (possibly unsaved) document content,
    // not whatever is currently on disk. There is no file to write back
    // to in that mode.
    let is_stdin = source == "-";
    if is_stdin && write {
        eprintln!("--write cannot be used when formatting stdin ('-'): there is no file to write to");
        std::process::exit(1);
    }

    let source_content = if is_stdin {
        let mut buffer = String::new();
        if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer) {
            eprintln!("Error reading stdin: {}", e);
            std::process::exit(1);
        }
        buffer
    } else {
        match std::fs::read_to_string(source) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", source, e);
                std::process::exit(1);
            }
        }
    };

    let formatted = match nucle_lang::format_source(&source_content) {
        Ok(formatted) => formatted,
        Err(e) => {
            eprintln!("Format failed: {}", e);
            std::process::exit(1);
        }
    };

    if is_stdin {
        print!("{}", formatted);
        return;
    }

    if check {
        if formatted == source_content {
            println!("{} is already formatted", source);
        } else {
            eprintln!("{} is not formatted; run `nucle fmt --write {}` to fix", source, source);
            std::process::exit(1);
        }
        return;
    }

    if write {
        if formatted == source_content {
            println!("{} is already formatted", source);
        } else if let Err(e) = std::fs::write(source, &formatted) {
            eprintln!("Error writing file '{}': {}", source, e);
            std::process::exit(1);
        } else {
            println!("Formatted {}", source);
        }
        return;
    }

    print!("{}", formatted);
}

fn cmd_test(source: &str, json: bool) {
    let source_content = match std::fs::read_to_string(source) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", source, e);
            std::process::exit(1);
        }
    };

    let tokens = match nucle_lang::Lexer::new(&source_content).tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lex error: {}", e);
            std::process::exit(1);
        }
    };
    let program = match nucle_lang::Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    };

    let base_dir = std::path::Path::new(source).parent().unwrap_or_else(|| std::path::Path::new("."));
    let report = nucle_lang::run_tests(&program, base_dir);

    if json {
        let json_report = serde_json::json!({
            "ok": report.all_passed(),
            "compile_errors": report.compile_errors,
            "results": report.results.iter().map(|r| serde_json::json!({
                "name": r.name,
                "passed": r.passed,
                "failures": r.failures.iter().map(|f| serde_json::json!({
                    "message": f.message,
                    "line": f.span.line,
                    "column": f.span.column,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json_report).unwrap());
        if !report.all_passed() {
            std::process::exit(1);
        }
        return;
    }

    if !report.compile_errors.is_empty() {
        for diagnostic in &report.compile_errors {
            println!("{}", nucle_lang::diagnostics::render_snippet(source, &source_content, diagnostic));
        }
        std::process::exit(1);
    }

    if report.results.is_empty() {
        println!("no test blocks found in {}", source);
        return;
    }

    for result in &report.results {
        if result.passed {
            println!("test {} ... ok", result.name);
        } else {
            println!("test {} ... FAILED", result.name);
        }
    }

    let failed: Vec<_> = report.results.iter().filter(|r| !r.passed).collect();
    if !failed.is_empty() {
        println!("\nfailures:");
        for result in &failed {
            println!("\n---- {} ----", result.name);
            for failure in &result.failures {
                println!("{}:{}: {}", source, failure.span.line, failure.message);
            }
        }
    }

    println!(
        "\ntest result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        report.results.len() - failed.len(),
        failed.len()
    );

    if !failed.is_empty() {
        std::process::exit(1);
    }
}

fn cmd_run(source: &str) {
    warn_on_lock_mismatch(source);
    match nucle_lang::run_source_file(source) {
        Ok(report) => println!("{}", report),
        Err(e) => {
            eprintln!("NucleScript failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_doc(source: &str, output: Option<&str>) {
    let source_content = match fs::read_to_string(source) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", source, e);
            std::process::exit(1);
        }
    };
    let tokens = match nucle_lang::Lexer::new(&source_content).tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lex error: {}", e);
            std::process::exit(1);
        }
    };
    let program = match nucle_lang::Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    };

    let docs = nucle_lang::generate_docs(&program);
    match output {
        Some(path) => {
            if let Err(e) = fs::write(path, &docs) {
                eprintln!("Error writing '{}': {}", path, e);
                std::process::exit(1);
            }
            println!("Wrote documentation to {}", path);
        }
        None => print!("{}", docs),
    }
}

/// Scaffolds a minimal, immediately-runnable NucleScript project --
/// `main.nsl` deliberately needs no external sample file (no `store` of a
/// real path), so `nucle check`/`nucle run` succeed against it completely
/// unmodified, matching `cargo new`/`npm init`'s "it just works" bar.
fn cmd_new(name: &str) {
    let dir = std::path::Path::new(name);
    if dir.exists() {
        eprintln!("Error: '{}' already exists", name);
        std::process::exit(1);
    }
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("Error creating directory '{}': {}", name, e);
        std::process::exit(1);
    }

    let project_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or(name);

    let main_nsl = r#"/// Entry point for this NucleScript project.
pool archive: DnaPool {
    codec: Ternary,
    redundancy: 3x,
    profile: Illumina
}

let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
"#;

    let readme = format!(
        r#"# {project_name}

A NucleScript project, scaffolded by `nucle new`.

## Getting started

```bash
nucle check main.nsl          # compile-only validation, no hardware/execution
nucle run main.nsl            # compile and execute
nucle test main.nsl           # run any `test {{ ... }}` blocks
nucle fmt main.nsl --write    # format in place
nucle doc main.nsl            # generate Markdown docs from `///` comments
```

`nucle.lock` records checksums for any packages you install with `nucle
package install <name>` -- see the main NucleOS repository's
`docs/packages.md` for the bundled `@nuclescript/*` presets.
"#
    );

    let lock = nucle_lang::lockfile::LockFile::default().to_json().unwrap_or_else(|_| "{}".to_string());

    for (path, content) in [
        (dir.join("main.nsl"), main_nsl.to_string()),
        (dir.join("README.md"), readme),
        (dir.join(nucle_lang::lockfile::LOCK_FILE_NAME), lock),
    ] {
        if let Err(e) = fs::write(&path, &content) {
            eprintln!("Error writing '{}': {}", path.display(), e);
            std::process::exit(1);
        }
    }

    println!("Created NucleScript project '{}' in ./{}", project_name, dir.display());
    println!("\nNext steps:");
    println!("  cd {}", dir.display());
    println!("  nucle run main.nsl");
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
fn load_lock_file() -> Option<nucle_lang::lockfile::LockFile> {
    let content = fs::read_to_string(nucle_lang::lockfile::LOCK_FILE_NAME).ok()?;
    match nucle_lang::lockfile::LockFile::from_json(&content) {
        Ok(lock) => Some(lock),
        Err(e) => {
            eprintln!("Warning: {} is not valid JSON: {}", nucle_lang::lockfile::LOCK_FILE_NAME, e);
            None
        }
    }
}

fn cmd_package(action: PackageAction, json: bool) {
    match action {
        PackageAction::List => {
            let index = nucle_lang::package::registry_index();
            if json {
                println!("{}", serde_json::to_string_pretty(&index).unwrap());
            } else {
                println!("Packages in {}:\n", nucle_lang::package::REGISTRY_INDEX_PATH);
                for entry in &index.packages {
                    println!("  {} {} ({})", entry.name, entry.version, entry.import);
                    println!("    {}", entry.description);
                }
            }
        }
        PackageAction::Install { name } => {
            let Some(manifest) = nucle_lang::package::find_package(&name) else {
                eprintln!("Package '{}' not found in {}.", name, nucle_lang::package::REGISTRY_INDEX_PATH);
                std::process::exit(1);
            };
            let resolved_name = manifest.name.clone();
            nucle_lang::package::register_package(manifest);

            if json {
                let json_val = serde_json::json!({
                    "status": "Installed",
                    "package": resolved_name
                });
                println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
            } else {
                println!("✓ Installed package '{}' successfully.", resolved_name);
            }
        }
        PackageAction::Verify { name } => {
            let Some(manifest) = nucle_lang::package::find_package(&name) else {
                eprintln!("Package '{}' not found in {}.", name, nucle_lang::package::REGISTRY_INDEX_PATH);
                std::process::exit(1);
            };

            let mut errors = nucle_lang::package::validate_manifest(&manifest);

            let checksum_status = match (load_lock_file(), nucle_lang::package::find_package_manifest_json(&name)) {
                (Some(lock), Some(manifest_json)) => match lock.find(&manifest.import_source) {
                    Some(locked) => {
                        let sources = nucle_lang::package::checksum_sources(&name);
                        let actual = nucle_lang::lockfile::compute_checksum(manifest_json, &sources);
                        if actual == locked.checksum {
                            "match".to_string()
                        } else {
                            errors.push(format!(
                                "checksum mismatch against {}: locked={}, actual={}",
                                nucle_lang::lockfile::LOCK_FILE_NAME, locked.checksum, actual
                            ));
                            "mismatch".to_string()
                        }
                    }
                    None => format!("not present in {}", nucle_lang::lockfile::LOCK_FILE_NAME),
                },
                _ => format!("no {} found — run 'nucle package lock' to create one", nucle_lang::lockfile::LOCK_FILE_NAME),
            };

            let verified = errors.is_empty();

            if json {
                let json_val = serde_json::json!({
                    "verified": verified,
                    "errors": errors,
                    "package": manifest.name,
                    "checksum_status": checksum_status,
                });
                println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
            } else {
                println!("Checksum: {}", checksum_status);
                if verified {
                    println!("✓ Package '{}' verified successfully.", manifest.name);
                } else {
                    println!("✗ Package verification failed for '{}':", manifest.name);
                    for e in &errors {
                        println!("  - {}", e);
                    }
                    std::process::exit(1);
                }
            }
        }
        PackageAction::Inspect { name } => {
            let Some(manifest) = nucle_lang::package::find_package(&name) else {
                eprintln!("Package '{}' not found in {}.", name, nucle_lang::package::REGISTRY_INDEX_PATH);
                std::process::exit(1);
            };
            let errors = nucle_lang::package::validate_manifest(&manifest);
            if json {
                let json_val = serde_json::json!({
                    "manifest": manifest,
                    "validation_errors": errors,
                    "valid": errors.is_empty()
                });
                println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
            } else {
                if !errors.is_empty() {
                    println!("⚠ Manifest validation errors found for '{}':", manifest.name);
                    for e in &errors {
                        println!("  - {}", e);
                    }
                    println!();
                }
                println!("Package:      {}", manifest.name);
                println!("Version:      {}", manifest.version);
                println!("Import:       {}", manifest.import_source);
                println!("License:      {}", manifest.license);
                println!("Description:  {}", manifest.description);
                println!("Repository:   {}", manifest.repository.url);
                println!("\nExports:");
                for export in &manifest.exports {
                    println!("  - {} [{}] {}", export.name, export.kind, export.description);
                }
            }
        }
        PackageAction::Lock => {
            let mut lock = load_lock_file().unwrap_or_default();
            let index = nucle_lang::package::registry_index();
            for entry in &index.packages {
                let (Some(manifest), Some(manifest_json)) = (
                    nucle_lang::package::find_package(&entry.name),
                    nucle_lang::package::find_package_manifest_json(&entry.name),
                ) else {
                    continue;
                };
                let sources = nucle_lang::package::checksum_sources(&entry.name);
                lock.upsert(nucle_lang::lockfile::generate(&manifest, manifest_json, &sources));
            }
            let content = lock.to_json().unwrap();
            if let Err(e) = fs::write(nucle_lang::lockfile::LOCK_FILE_NAME, &content) {
                eprintln!("Failed to write {}: {}", nucle_lang::lockfile::LOCK_FILE_NAME, e);
                std::process::exit(1);
            }
            if json {
                println!("{}", content);
            } else {
                println!("✓ Wrote {} with {} package(s).", nucle_lang::lockfile::LOCK_FILE_NAME, lock.packages.len());
            }
        }
    }
}

/// Compiles one NucleScript source file down to its hardware requests —
/// shared by both the single- and multi-source export paths so the
/// lex/parse/typecheck/collect sequence (in particular, the typecheck
/// confirm-gate) can't silently diverge between them.
fn compile_source(source: &str) -> Result<Vec<nucle_lang::hardware::HardwareRequest>, String> {
    let content = fs::read_to_string(source).map_err(|e| format!("Error reading source file '{}': {}", source, e))?;

    let tokens = nucle_lang::Lexer::new(&content).tokenize().map_err(|e| format!("Lexing error: {}", e))?;
    let program = nucle_lang::Parser::new(tokens).parse_program().map_err(|e| format!("Parsing error: {}", e))?;

    // Reuse the compiler's own effect/confirmation check — a program missing
    // `confirm hardware`/`confirm physical_key` in source must never reach
    // the export step.
    let report = nucle_lang::typeck::check_program(&program);
    if report.has_errors() {
        return Err(format!("NucleScript type check failed:\n{}", report));
    }

    Ok(nucle_lang::hardware::collect_hardware_requests(&program))
}

/// Inserts `_<index>` before a path's extension, so multiple concurrent
/// file-export submissions in one invocation never race on the same path
/// (e.g. `batch.json` -> `batch_1.json`, `batch_2.json`, ...).
fn indexed_output_path(base: &str, index: usize) -> String {
    let path = std::path::Path::new(base);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("batch");
    let ext = path.extension().and_then(|s| s.to_str());
    let file_name = match ext {
        Some(ext) => format!("{}_{}.{}", stem, index, ext),
        None => format!("{}_{}", stem, index),
    };
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent.join(file_name).to_string_lossy().into_owned(),
        None => file_name,
    }
}

fn cmd_hardware(subcommand: HardwareSubcommand, pool_dir: &std::path::Path, json: bool) {
    match subcommand {
        HardwareSubcommand::Export { source, output, provider, confirm, simulated_delay_ms } => {
            if source.len() == 1 {
                export_single(&source[0], &output, &provider, confirm, simulated_delay_ms, pool_dir, json);
            } else {
                export_multiple(&source, &output, &provider, confirm, simulated_delay_ms, pool_dir, json);
            }
        }
    }
}

/// The original, single-file export path — unchanged output shape (a flat
/// JSON object on success) so no existing consumer of `nucle hardware
/// export <file>` breaks.
fn export_single(source: &str, output: &str, provider: &str, confirm: bool, simulated_delay_ms: u64, pool_dir: &std::path::Path, json: bool) {
    let requests = match compile_source(source) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let effectful_count = nucle_hardware::count_effectful(&requests);
    let delay = std::time::Duration::from_millis(simulated_delay_ms);

    // An additional, pool-scoped gate on top of nucle_hardware::confirm's
    // own bare bool check: if this pool has a confirm-allowlist configured
    // (nucle confirm-users --add), the invoking OS user must be on it, or
    // this refuses before the provider ever sees the batch -- same as
    // nucle_hardware::confirm's own "refuse before submission" shape, just
    // one layer earlier. A no-op for every pool that hasn't configured one.
    if effectful_count > 0 && confirm {
        if let Err(e) = nucle_vfs::confirm_policy::check(pool_dir) {
            eprintln!("Refusing to submit: {}", e);
            std::process::exit(1);
        }
    }

    // The confirmation gate itself lives in nucle_hardware::confirm, not
    // here, so every consumer of Provider gets the same safety check — not
    // just this CLI command.
    let (used_provider, result): (&str, Result<String, String>) = match provider {
        "mock" => ("mock", nucle_hardware::submit_with_confirmation(&nucle_hardware::MockProvider, &requests, confirm)),
        "mock-delayed" => (
            "mock-delayed",
            nucle_hardware::submit_with_confirmation(&nucle_hardware::DelayedMockProvider::new(delay), &requests, confirm),
        ),
        "file-export" => {
            let p = nucle_hardware::FileExportProvider::new(std::path::PathBuf::from(output));
            ("file-export", nucle_hardware::submit_with_confirmation(&p, &requests, confirm))
        }
        other => {
            eprintln!(
                "Note: no vendor adapter implemented for provider '{}' yet; falling back to file-export.",
                other
            );
            let p = nucle_hardware::FileExportProvider::new(std::path::PathBuf::from(output));
            ("file-export", nucle_hardware::submit_with_confirmation(&p, &requests, confirm))
        }
    };

    match result {
        Ok(msg) => {
            if json {
                let json_val = serde_json::json!({
                    "status": "Success",
                    "provider": used_provider,
                    "exported_file": output,
                    "requests_count": requests.len(),
                    "effectful_requests_confirmed": effectful_count > 0,
                });
                println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
            } else {
                println!("✓ {}", msg);
            }
        }
        Err(e) => {
            eprintln!("Export failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// One compiled source's hardware requests plus the resolved output path
/// (already index-suffixed) it'll be exported to.
struct PendingSource {
    source: String,
    output_path: String,
    requests: Vec<nucle_lang::hardware::HardwareRequest>,
}

/// The new concurrent path for 2+ sources in one invocation: compiles every
/// source, refuses the whole command upfront if any needs `--confirm` (so no
/// job ever starts if the command is going to be rejected), then submits all
/// batches back-to-back — each `submit()` call returns immediately, so N
/// submissions never block on each other, even against `mock-delayed`'s real
/// background-thread latency — and finally waits on all of them and reports
/// per-source success/failure.
fn export_multiple(sources: &[String], output: &str, provider: &str, confirm: bool, simulated_delay_ms: u64, pool_dir: &std::path::Path, json: bool) {
    let mut submissions = Vec::new();
    for (i, src) in sources.iter().enumerate() {
        let requests = match compile_source(src) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to compile '{}': {}", src, e);
                std::process::exit(1);
            }
        };
        submissions.push(PendingSource {
            source: src.clone(),
            output_path: indexed_output_path(output, i + 1),
            requests,
        });
    }

    let mut any_effectful = false;
    for sub in &submissions {
        let effectful = nucle_hardware::count_effectful(&sub.requests);
        if effectful > 0 {
            any_effectful = true;
            if !confirm {
                eprintln!(
                    "Refusing to submit: '{}' contains {} cost-bearing/destructive request(s) without confirmation.",
                    sub.source, effectful
                );
                std::process::exit(1);
            }
        }
    }

    // Same pool-scoped OS-user allowlist as export_single, checked once
    // for the whole batch of sources rather than per-source (it's the
    // same invoking user and the same pool_dir for all of them).
    if any_effectful && confirm {
        if let Err(e) = nucle_vfs::confirm_policy::check(pool_dir) {
            eprintln!("Refusing to submit: {}", e);
            std::process::exit(1);
        }
    }

    let delay = std::time::Duration::from_millis(simulated_delay_ms);
    let mut warned_fallback = false;
    let mut pending: Vec<(&PendingSource, &str, Box<dyn JobHandle>)> = Vec::new();

    for sub in &submissions {
        let (used_provider, handle_result): (&str, Result<Box<dyn JobHandle>, String>) = match provider {
            "mock" => (
                "mock",
                nucle_hardware::submit_with_confirmation_async(&nucle_hardware::MockProvider, &sub.requests, confirm),
            ),
            "mock-delayed" => (
                "mock-delayed",
                nucle_hardware::submit_with_confirmation_async(
                    &nucle_hardware::DelayedMockProvider::new(delay),
                    &sub.requests,
                    confirm,
                ),
            ),
            "file-export" => {
                let p = nucle_hardware::FileExportProvider::new(std::path::PathBuf::from(&sub.output_path));
                ("file-export", nucle_hardware::submit_with_confirmation_async(&p, &sub.requests, confirm))
            }
            other => {
                if !warned_fallback {
                    eprintln!(
                        "Note: no vendor adapter implemented for provider '{}' yet; falling back to file-export.",
                        other
                    );
                    warned_fallback = true;
                }
                let p = nucle_hardware::FileExportProvider::new(std::path::PathBuf::from(&sub.output_path));
                ("file-export", nucle_hardware::submit_with_confirmation_async(&p, &sub.requests, confirm))
            }
        };

        match handle_result {
            Ok(handle) => pending.push((sub, used_provider, handle)),
            Err(e) => {
                eprintln!("Export failed for '{}': {}", sub.source, e);
                std::process::exit(1);
            }
        }
    }

    let mut any_failed = false;
    let mut results: Vec<(&PendingSource, &str, Result<String, String>)> = Vec::new();
    for (sub, used_provider, handle) in pending {
        let outcome = handle.wait();
        if outcome.is_err() {
            any_failed = true;
        }
        results.push((sub, used_provider, outcome));
    }

    if json {
        let arr: Vec<_> = results
            .iter()
            .map(|(sub, used_provider, res)| match res {
                Ok(msg) => serde_json::json!({
                    "source": sub.source,
                    "status": "Success",
                    "provider": used_provider,
                    "message": msg,
                    "exported_file": sub.output_path,
                    "requests_count": sub.requests.len(),
                    "effectful_requests_confirmed": nucle_hardware::count_effectful(&sub.requests) > 0,
                }),
                Err(e) => serde_json::json!({
                    "source": sub.source,
                    "status": "Failed",
                    "provider": used_provider,
                    "error": e,
                }),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        for (sub, _used_provider, res) in &results {
            match res {
                Ok(msg) => println!("✓ [{}] {}", sub.source, msg),
                Err(e) => eprintln!("✗ [{}] Export failed: {}", sub.source, e),
            }
        }
    }

    if any_failed {
        std::process::exit(1);
    }
}

/// One named pass/warn/fail check in the doctor report. `detail` carries
/// context (missing files, parse errors, etc.) — empty when the check passed
/// clean. `skipped` marks a check that couldn't run at all (e.g. a directory
/// doesn't exist from this cwd) so it degrades gracefully instead of
/// pretending to have verified something it didn't.
struct DoctorCheck {
    name: &'static str,
    ok: bool,
    skipped: bool,
    detail: Vec<String>,
}

/// Every crate whose `Cargo.toml` should inherit `version.workspace = true`
/// rather than a hardcoded override — this is the actual mechanism keeping
/// workspace crate versions consistent, so checking it is a real signal, not
/// a tautological runtime comparison of already-guaranteed-equal values.
const WORKSPACE_CRATE_MANIFESTS: &[&str] = &[
    "nucle_codec/Cargo.toml",
    "nucle_synth/Cargo.toml",
    "nucle_ecc/Cargo.toml",
    "nucle_index/Cargo.toml",
    "nucle_vfs/Cargo.toml",
    "nucle_agent/Cargo.toml",
    "nucle_lang/Cargo.toml",
    "nucle_hardware/Cargo.toml",
    "nucle_cli/Cargo.toml",
];

fn check_workspace_versions() -> DoctorCheck {
    let mut detail = Vec::new();
    let mut found_any = false;
    for path in WORKSPACE_CRATE_MANIFESTS {
        match fs::read_to_string(path) {
            Ok(content) => {
                found_any = true;
                if !content.contains("version.workspace = true") {
                    detail.push(format!("{} does not inherit version.workspace = true", path));
                }
            }
            Err(_) => detail.push(format!("{} not found (skipped)", path)),
        }
    }
    DoctorCheck {
        name: "Workspace crate versions",
        ok: detail.is_empty(),
        skipped: !found_any,
        detail,
    }
}

fn check_package_manifest() -> DoctorCheck {
    let manifest = nucle_lang::package::presets_manifest();
    let errors = nucle_lang::package::validate_manifest(&manifest);
    DoctorCheck {
        name: "Presets package manifest",
        ok: errors.is_empty(),
        skipped: false,
        detail: errors,
    }
}

fn check_fixtures() -> DoctorCheck {
    let fixtures = [
        "docs/examples/fixtures/small_text.txt",
        "docs/examples/fixtures/archive.bin",
        "docs/examples/fixtures/sample.fasta",
        "docs/examples/fixtures/image.png",
        "docs/examples/fixtures/project_tree",
    ];
    let missing: Vec<String> = fixtures.iter()
        .filter(|f| !std::path::Path::new(f).exists())
        .map(|f| f.to_string())
        .collect();
    DoctorCheck {
        name: "Standard fixtures present",
        ok: missing.is_empty(),
        skipped: false,
        detail: missing,
    }
}

/// Actually lexes and parses every `.nsl` file directly under docs/examples/
/// (not a mere existence check) — a syntax error here means an example that
/// ships with the repo is broken. Programs under docs/examples/failures/ are
/// intentionally excluded from this: they're supposed to fail *type checking*,
/// but must still be syntactically valid, so a separate check below covers them.
fn check_examples_parse() -> DoctorCheck {
    let dir = std::path::Path::new("docs/examples");
    let Ok(entries) = fs::read_dir(dir) else {
        return DoctorCheck { name: "Example programs parse", ok: true, skipped: true, detail: vec![] };
    };
    let mut detail = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("nsl") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else { continue };
        let result = nucle_lang::Lexer::new(&source).tokenize()
            .map_err(|e| e.to_string())
            .and_then(|tokens| nucle_lang::Parser::new(tokens).parse_program().map_err(|e| e.to_string()));
        if let Err(e) = result {
            detail.push(format!("{}: {}", path.display(), e));
        }
    }
    DoctorCheck { name: "Example programs parse", ok: detail.is_empty(), skipped: false, detail }
}

/// docs/examples/failures/ programs are supposed to fail *type checking* by
/// design, but must still be syntactically valid NucleScript — this checks
/// exactly that, separately from check_examples_parse()'s exclusion of them.
fn check_failure_examples_parse() -> DoctorCheck {
    let dir = std::path::Path::new("docs/examples/failures");
    let Ok(entries) = fs::read_dir(dir) else {
        return DoctorCheck { name: "Failure-mode examples parse", ok: true, skipped: true, detail: vec![] };
    };
    let mut detail = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("nsl") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else { continue };
        let result = nucle_lang::Lexer::new(&source).tokenize()
            .map_err(|e| e.to_string())
            .and_then(|tokens| nucle_lang::Parser::new(tokens).parse_program().map_err(|e| e.to_string()));
        if let Err(e) = result {
            detail.push(format!("{}: {}", path.display(), e));
        }
    }
    DoctorCheck { name: "Failure-mode examples parse", ok: detail.is_empty(), skipped: false, detail }
}

/// Runs a real dna_write → dna_read roundtrip against a scratch, in-memory
/// NucleOS instance -- deliberately not the real pool directory (NucleOS::
/// new, not open), so a doctor run never touches or pollutes real stored
/// data with its throwaway probe file.
fn check_vfs_roundtrip() -> DoctorCheck {
    let mut os = NucleOS::new(4);
    let probe = b"nucle doctor VFS roundtrip probe";
    let detail = match os.dna_write("__doctor_probe__.tmp", probe, 1) {
        Ok(_) => match os.dna_read("__doctor_probe__.tmp") {
            Ok(recovered) if recovered == probe => vec![],
            Ok(_) => vec!["roundtrip data mismatch".to_string()],
            Err(e) => vec![format!("read failed: {}", e)],
        },
        Err(e) => vec![format!("write failed: {}", e)],
    };
    DoctorCheck { name: "VFS write/read roundtrip", ok: detail.is_empty(), skipped: false, detail }
}

fn cmd_doctor(json: bool) {
    let profiles = HardwareProfile::all().iter().map(|p| p.name().to_string()).collect::<Vec<_>>();

    let checks = vec![
        check_workspace_versions(),
        check_package_manifest(),
        check_fixtures(),
        check_examples_parse(),
        check_failure_examples_parse(),
        check_vfs_roundtrip(),
    ];

    let overall_status = if checks.iter().all(|c| c.ok || c.skipped) {
        "Healthy"
    } else {
        "Degraded"
    };

    if json {
        let checks_json: Vec<_> = checks.iter().map(|c| serde_json::json!({
            "name": c.name,
            "ok": c.ok,
            "skipped": c.skipped,
            "detail": c.detail,
        })).collect();
        let json_val = serde_json::json!({
            "synthesis_profiles": profiles,
            "checks": checks_json,
            "status": overall_status
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
    } else {
        println!("# NucleOS Diagnostics Report");
        println!("\n## Environment Capabilities");
        println!("- **Synthesis Profiles Available**: {:?}", profiles);

        println!("\n## Checks");
        for c in &checks {
            let mark = if c.skipped { "⚠ SKIPPED" } else if c.ok { "✓ PASS" } else { "✗ FAILED" };
            println!("- **{}**: {}", c.name, mark);
            for line in &c.detail {
                println!("    - {}", line);
            }
        }

        println!("\n## Overall Status");
        println!("- **System Health**: **{}**", overall_status);
    }
}

fn cmd_agent(command: &str, pool_dir: &std::path::Path, passphrase: Option<&str>) {
    if command.is_empty() {
        println!("Usage: nucle agent <natural language command>");
        println!("\nExamples:");
        println!("  nucle agent store readme.txt with 3x redundancy");
        println!("  nucle agent retrieve readme.txt");
        println!("  nucle agent search for text files");
        println!("  nucle agent pool status");
        return;
    }

    let mut os = open_pool(pool_dir, passphrase);
    match Executor::run(&mut os, command) {
        Ok(report) => {
            // A single agent command could be any tool, including several
            // that mutate (store/retrieve/delete/migrate) -- persist
            // unconditionally rather than trying to guess from the report.
            persist_pool(&mut os, pool_dir);
            println!("{}", report);
        }
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
    println!("  nucle list [prefix]                       List stored files, optionally by name prefix");
    println!("  nucle capacity [max] [--unlimited]        Show or set the pool's capacity limit");
    println!("  nucle audit [--tail N]                     Show the pool's audit log");
    println!("  nucle confirm-users [--add|--remove USER]  Show or edit who may pass --confirm for hardware export");
    println!("  nucle encrypt-pool                        Enable encryption at rest (needs --pool-key)");
    println!("  nucle decrypt-pool                        Disable encryption at rest (needs --pool-key)");
    println!("  nucle simulate <file> -p <profile>        Simulate synthesis noise");
    println!("  nucle bench [file]                        Benchmark all codecs");
    println!("  nucle benchmark [file] [-p profile]       Full-pipeline benchmark");
    println!("  nucle stress [-s size]                    Stress test all codecs");
    println!("  nucle pipeline [-f N] [-s size] [-p prof]  Full-pipeline stress test");
    println!("  nucle run <source.nsl>                    Run NucleScript source file");
    println!("  nucle plan <source.nsl>                   Show no-hardware NucleScript plan");
    println!("  nucle packages                            List released NucleScript packages");
    println!("  nucle package install <manifest_path>     Install a package from manifest");
    println!("  nucle package verify <manifest_path>      Verify package manifest integrity");
    println!("  nucle hardware export <src.nsl> [-o out]  Export batch requests to a JSON file");
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
