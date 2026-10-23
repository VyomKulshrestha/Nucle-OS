//! `nucle cost <file>`: what storing a file in DNA costs, how dense it is,
//! and how much it weighs, computed from the strands Nucle-OS actually
//! writes for it (codec, Reed-Solomon parity and primers included).

use nucle_synth::cost::{estimate, ArchiveShape, Chemistry, CostAssumptions, Sequencer, HDD_KG_CO2E_PER_TB_YEAR, TAPE_KG_CO2E_PER_TB_YEAR};
use nucle_vfs::syscall::{CodecKind, NucleOS};

pub struct CostArgs {
    pub file: String,
    pub codec: String,
    pub redundancy: usize,
    pub synthesis_usd_per_nt: f64,
    pub sequencing_usd_per_gbase: f64,
    pub read_coverage: f64,
    pub copies: f64,
    pub sequencer: String,
    pub chemistry: String,
    pub synthesis_kg_co2e_per_nt: Option<f64>,
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
    let Some(sequencer) = Sequencer::parse(&args.sequencer) else {
        let known: Vec<&str> = Sequencer::ALL.iter().map(|s| s.name()).collect();
        eprintln!("Unknown sequencer '{}'; expected one of {}", args.sequencer, known.join(", "));
        std::process::exit(1);
    };
    let Some(chemistry) = Chemistry::parse(&args.chemistry) else {
        let known: Vec<&str> = Chemistry::ALL.iter().map(|c| c.name()).collect();
        eprintln!("Unknown synthesis chemistry '{}'; expected one of {}", args.chemistry, known.join(", "));
        std::process::exit(1);
    };
    for (name, value) in [
        ("--synthesis-usd-per-nt", args.synthesis_usd_per_nt),
        ("--sequencing-usd-per-gbase", args.sequencing_usd_per_gbase),
        ("--coverage", args.read_coverage),
        ("--copies", args.copies),
        ("--synthesis-kg-co2e-per-nt", args.synthesis_kg_co2e_per_nt.unwrap_or(0.0)),
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
        sequencer,
        chemistry,
        synthesis_kg_co2e_per_nt: args.synthesis_kg_co2e_per_nt,
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
            "synthesis_kg_co2e_low": r.synthesis_kg_co2e_low,
            "synthesis_kg_co2e_high": r.synthesis_kg_co2e_high,
            "sequencing_kg_co2e": r.sequencing_kg_co2e,
            "lifecycle_kg_co2e_per_year": r.lifecycle_kg_co2e_per_year,
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
    if args.synthesis_kg_co2e_per_nt.is_some() {
        println!("  carbon, write: {:.3e} kg CO2e (at the given kg CO2e per nucleotide)", r.synthesis_kg_co2e_low);
    } else {
        println!(
            "  carbon, write: {:.3e} to {:.3e} kg CO2e (synthesis solvents only, light-directed array run: full chip to 16,383 sequences)",
            r.synthesis_kg_co2e_low, r.synthesis_kg_co2e_high
        );
    }
    println!("  carbon, read: {:.3e} kg CO2e for one read-back ({}, {} kg CO2e per Gbase)", r.sequencing_kg_co2e, sequencer.name(), sequencer.kg_co2e_per_gbase());
    println!(
        "  carbon, kept: {:.3e} kg CO2e per year, whole life cycle ({}: {} kg per TB-year; hard disk {}, tape {})",
        r.lifecycle_kg_co2e_per_year,
        chemistry.name(),
        chemistry.kg_co2e_per_tb_year(),
        HDD_KG_CO2E_PER_TB_YEAR,
        TAPE_KG_CO2E_PER_TB_YEAR
    );
    println!();
    println!("Prices are assumptions, not quotes; published estimates range from about $100/MB");
    println!("(Blocks & Files, 2026) to $3,500/MB (US GAO, 2022) to write. Override them with");
    println!("--synthesis-usd-per-nt and --sequencing-usd-per-gbase.");
    println!();
    println!("Read-side carbon is Illumina's own reagent-kit life-cycle assessment per gigabase");
    println!("(2025 Corporate Responsibility Report); a small archive on its own run uses a whole kit.");
    println!("Life-cycle carbon is read from the chart in Nguyen et al. 2020 (1 TB kept a year,");
    println!("10% read). Write-side carbon counts only the acetonitrile and ethanol of one lab-scale");
    println!("light-directed synthesis run (Antkowiak et al. 2020) at published emission factors; the");
    println!("amidites, DMSO and electricity have none, so it is a floor.");
}
