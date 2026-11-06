//! `nucle hardware order`: the strands a program stores, written as an
//! oligo-pool order sheet for Twist or IDT and checked against the
//! vendor's limits. Nothing is ordered; the sheet is for a person to
//! upload (see `nucle_hardware::order_sheet`).

use nucle_hardware::order_sheet::{check, oligo_name, render, Oligo, Vendor};
use nucle_lang::middle::{lower_program, MirOp};
use nucle_lang::Codec;
use nucle_vfs::syscall::{CodecKind, NucleOS};
use std::path::Path;

pub fn cmd_order(source: &str, vendor: &str, out: &str, json: bool) {
    let fail = |message: String| -> ! {
        eprintln!("{message}");
        std::process::exit(1);
    };
    let vendor = Vendor::parse(vendor).unwrap_or_else(|| fail(format!("unknown vendor '{vendor}'; expected twist or idt")));
    let format = vendor.format_for(out).unwrap_or_else(|e| fail(e));
    let oligos = oligos_for_program(source).unwrap_or_else(|e| fail(e));
    let report = check(vendor, &oligos);
    if !report.problems.is_empty() {
        fail(format!("{} would not take this order; no sheet written:\n  {}", vendor.name(), report.problems.join("\n  ")));
    }
    if let Err(e) = std::fs::write(out, render(format, &oligos)) {
        fail(format!("Error writing '{out}': {e}"));
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "vendor": vendor.name(),
                "out": out,
                "oligos": report.oligos,
                "shortest": report.shortest,
                "longest": report.longest,
                "idt_scales": report.idt_scales,
            })
        );
        return;
    }
    println!("Wrote {} oligos ({}-{} nt) for {} to {}.", report.oligos, report.shortest, report.longest, vendor.name(), out);
    if vendor == Vendor::Idt {
        println!("IDT oPools scales that take a pool this size: {}.", report.idt_scales.join(", "));
        println!("IDT's oPools page takes Excel uploads: paste the Name and Sequence columns into its template.");
    }
    println!("Nothing was ordered.");
}

/// Encodes every real (not simulated) store and pipeline at the top of
/// the program at `path`, as `nucle run` would but with no noise, into a
/// fresh pool, and returns what synthesis would have to make: the archive
/// header's strands, then each file's data and parity strands with their
/// primers. Stores inside functions are not collected, the same as for
/// `nucle hardware export`.
pub fn oligos_for_program(path: &str) -> Result<Vec<Oligo>, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("Error reading source file '{path}': {e}"))?;
    let plan = nucle_lang::compile(&source).map_err(|e| e.to_string())?;
    let base = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    let mut os = NucleOS::new(100);
    for op in lower_program(&plan.program).ops {
        let MirOp::Store { file, pool, codec, redundancy, simulate: false, .. } = op else { continue };
        let data = std::fs::read(base.join(&file)).map_err(|e| format!("Error reading '{file}': {e}"))?;
        let name = Path::new(&file).file_name().and_then(|n| n.to_str()).unwrap_or(&file);
        os.dna_write_with_codec(&format!("{pool}/{name}"), &data, redundancy, codec_kind(codec))?;
    }
    if os.catalog.list().is_empty() {
        return Err("the program stores nothing to synthesize: no store or pipeline outside a function writes for real".to_string());
    }

    let header = os.archive_header().to_strands().map_err(|e| e.to_string())?;
    let mut oligos: Vec<Oligo> = header.iter().enumerate().map(|(i, s)| Oligo { name: format!("header_{i}"), sequence: s.to_string() }).collect();
    for file in os.catalog.list() {
        for entry in os.pool.get_file_strands(&file.file_id) {
            let kind = if entry.is_parity { "p" } else { "d" };
            oligos.push(Oligo { name: format!("{}_{kind}{}", oligo_name(&file.filename), entry.strand_index), sequence: entry.strand.to_string() });
        }
    }
    Ok(oligos)
}

fn codec_kind(codec: Codec) -> CodecKind {
    match codec {
        Codec::Ternary => CodecKind::Ternary,
        Codec::YinYang => CodecKind::YinYang,
        Codec::Fountain => CodecKind::Fountain,
        Codec::Hedges => CodecKind::Hedges,
    }
}
