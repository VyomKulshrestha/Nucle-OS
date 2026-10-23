//! `nucle bootstrap export|inspect`: write a pool's strands with its
//! self-describing header, and recover the header from any strand dump.

use nucle_codec::base::DnaStrand;
use nucle_codec::bootstrap::{codec_name, ArchiveHeader};

pub fn cmd_export(out: &str, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let os = crate::open_pool(pool_dir, passphrase);
    let strands = os.strands_with_header().unwrap_or_else(|e| {
        eprintln!("Could not build the archive header: {e}");
        std::process::exit(1);
    });
    let header_strands = os.archive_header().to_strands().map(|s| s.len()).unwrap_or(0);
    let mut fasta = String::new();
    for (i, s) in strands.iter().enumerate() {
        let label = if i < header_strands { "header" } else { "data" };
        fasta.push_str(&format!(">strand_{i} {label}\n{s}\n"));
    }
    if let Err(e) = std::fs::write(out, fasta) {
        eprintln!("Error writing '{out}': {e}");
        std::process::exit(1);
    }
    if json {
        println!("{}", serde_json::json!({ "out": out, "header_strands": header_strands, "total_strands": strands.len() }));
    } else {
        println!("Wrote {} strands to {} ({} header + {} data).", strands.len(), out, header_strands, strands.len() - header_strands);
        println!("`nucle bootstrap inspect {out}` recovers the file table from the strands alone.");
    }
}

/// Sequences from FASTA (`>` headers) or plain one-sequence-per-line text.
/// Lines with anything other than A/C/G/T are skipped.
fn read_sequences(text: &str) -> Vec<DnaStrand> {
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<DnaStrand>| {
        if let Ok(s) = DnaStrand::from_str(current.trim()) {
            out.push(s);
        }
        current.clear();
    };
    let fasta = text.trim_start().starts_with('>');
    for line in text.lines() {
        let line = line.trim();
        if fasta {
            if line.starts_with('>') {
                flush(&mut current, &mut out);
            } else {
                current.push_str(line);
            }
        } else if !line.is_empty() {
            current.push_str(line);
            flush(&mut current, &mut out);
        }
    }
    flush(&mut current, &mut out);
    out
}

pub fn cmd_inspect(file: &str, json: bool) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Error reading '{file}': {e}");
        std::process::exit(1);
    });
    let strands = read_sequences(&text);
    let header = match ArchiveHeader::from_strands(&strands) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("No usable archive header in {} strands: {e}", strands.len());
            std::process::exit(1);
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&header).unwrap());
        return;
    }
    println!("Archive header recovered from {} strands", strands.len());
    println!("  default codec:  {}", codec_name(header.codec));
    println!("  strand length:  {} nt", header.strand_len);
    println!("  RS parity:      {} strands per stripe", header.rs_parity);
    println!("  files:          {}", header.files.len());
    for f in &header.files {
        let hash: String = f.hash_prefix.iter().map(|b| format!("{b:02x}")).collect();
        println!(
            "    {}  v{}  {} B  {}  {}d+{}p strands  sha256 {}…",
            f.name,
            f.version,
            f.size,
            codec_name(f.codec),
            f.data_strands,
            f.parity_strands,
            hash
        );
    }
}

#[cfg(test)]
mod tests {
    use super::read_sequences;

    #[test]
    fn reads_fasta_and_plain_lists_and_skips_junk() {
        let fasta = ">a\nACGT\nACGT\n>b note\nGGCC\n";
        let s = read_sequences(fasta);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].to_string(), "ACGTACGT");
        let plain = "ACGT\n\nnot dna\nTTAA\n";
        assert_eq!(read_sequences(plain).len(), 2);
    }
}
