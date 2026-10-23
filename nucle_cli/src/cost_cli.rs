//! `nucle cost <file>`: what storing a file in DNA costs, how dense it is,
//! and how much it weighs, computed from the strands Nucle-OS actually
//! writes for it (codec, Reed-Solomon parity and primers included).

use nucle_synth::cost::{estimate, ArchiveShape, CostAssumptions};
use nucle_vfs::syscall::{CodecKind, NucleOS};

pub struct CostArgs {
    pub file: String,
    pub codec: String,
    pub redundancy: usize,
    pub synthesis_usd_per_nt: f64,
    pub sequencing_usd_per_gbase: f64,
    pub read_coverage: f64,
    pub copies: f64,
}

pub fn cmd_cost(args: CostArgs, json: bool) {
    let data = std::fs::read(&args.file).unwrap_or_else(|e| {
        eprintln!("Error reading '{}': {}", args.file, e);
        std::process::exit(1);
    });
    let Some(codec) = CodecKind::from_codec_name(&args.codec) else {
        eprintln!("Unknown codec '{}'; expected ternary, fountain, yin-yang, or hedges", args.codec);
        std::process::exit(1);
    };
    for (name, value) in [
        ("--synthesis-usd-per-nt", args.synthesis_usd_per_nt),
        ("--sequencing-usd-per-gbase", args.sequencing_usd_per_gbase),
        ("--coverage", args.read_coverage),
        ("--copies", args.copies),
    ] {
        if !(value.is_finite() && value >= 0.0) {
            eprintln!("{name} must be a non-negative number, got {value}");
            std::process::exit(1);
        }
    }

    // Encode for real in a throwaway in-memory pool.
    let mut os = NucleOS::new(4);
    let name = std::path::Path::new(&args.file).file_name().and_then(|n| n.to_str()).unwrap_or("payload");
    let written = os.dna_write_with_codec(name, &data, args.redundancy, codec).unwrap_or_else(|e| {
        eprintln!("Encoding failed: {e}");
        std::process::exit(1);
    });
    let status = os.dna_stat();
    let shape = ArchiveShape {
        payload_bytes: data.len(),
        nucleotides: status.total_nucleotides,
        strands: written.total_strand_count,
    };
    let assumptions = CostAssumptions {
        synthesis_usd_per_nt: args.synthesis_usd_per_nt,
        sequencing_usd_per_gbase: args.sequencing_usd_per_gbase,
        read_coverage: args.read_coverage,
        copies_per_strand: args.copies,
    };
    let r = estimate(shape, assumptions);

    if json {
        let out = serde_json::json!({
            "file": args.file,
            "codec": written.codec,
            "payload_bytes": shape.payload_bytes,
            "data_strands": written.data_strand_count,
            "parity_strands": written.parity_strand_count,
            "nucleotides": shape.nucleotides,
            "bits_per_nt": r.bits_per_nt,
            "synthesis_usd": r.synthesis_usd,
            "sequencing_usd": r.sequencing_usd,
            "write_usd_per_mb": r.write_usd_per_mb,
            "read_usd_per_mb": r.read_usd_per_mb,
            "mass_grams": r.mass_grams,
            "bytes_per_gram": r.bytes_per_gram,
            "assumptions": r.assumptions,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("DNA storage cost for '{}'", args.file);
    println!("  encoding:     {} bytes, {} ({} data + {} parity strands, {} nt)", shape.payload_bytes, written.codec, written.data_strand_count, written.parity_strand_count, shape.nucleotides);
    println!("  density:      {:.3} payload bits per synthesized nucleotide", r.bits_per_nt);
    println!("  write:        ${:.2}  (${:.2}/MB at ${}/nt)", r.synthesis_usd, r.write_usd_per_mb, args.synthesis_usd_per_nt);
    println!("  read:         ${:.4}  (${:.4}/MB at {}x coverage, ${}/Gbase)", r.sequencing_usd, r.read_usd_per_mb, args.read_coverage, args.sequencing_usd_per_gbase);
    println!("  mass:         {:.3e} g at {} copies per strand ({:.3e} bytes per gram)", r.mass_grams, args.copies, r.bytes_per_gram);
    println!();
    println!("Prices are assumptions, not quotes; published estimates range from about $100/MB");
    println!("(Blocks & Files, 2026) to $3,500/MB (US GAO, 2022) to write. Override them with");
    println!("--synthesis-usd-per-nt and --sequencing-usd-per-gbase.");
}
