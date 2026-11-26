//! `nucle restore`: the files a pool held, rebuilt from sequencing reads
//! alone (see `nucle_vfs::restore`). Each file is written under `--out` by
//! the name the archive header gives it.

use nucle_codec::base::DnaStrand;
use nucle_ecc::realdata::parse_sequences;
use nucle_hardware::order_sheet::Flanks;
use nucle_index::primer::PrimerPair;
use nucle_vfs::restore::restore_from_reads;
use std::path::{Component, Path, PathBuf};

pub fn cmd_restore(reads_path: &str, out: &str, flank: Option<&str>, json: bool) {
    let fail = |message: String| -> ! {
        eprintln!("{message}");
        std::process::exit(1);
    };
    let text = std::fs::read_to_string(reads_path).unwrap_or_else(|e| fail(format!("Error reading '{reads_path}': {e}")));
    let (reads, skipped) = parse_sequences(&text, true).unwrap_or_else(|e| fail(format!("Error parsing '{reads_path}': {e}")));
    let flanks = flank.map(|text| {
        let f = Flanks::parse(text).unwrap_or_else(|e| fail(e));
        let strand = |s: &str| DnaStrand::from_str(s).unwrap_or_else(|e| fail(format!("primer '{s}': {e}")));
        PrimerPair::new("flank", strand(&f.forward), strand(&f.reverse))
    });
    let report = restore_from_reads(&reads, flanks.as_ref()).unwrap_or_else(|e| fail(e));

    let mut written = Vec::new();
    for file in &report.files {
        let path = file.bytes.as_ref().map(|bytes| {
            let path = safe_path(Path::new(out), &file.name).unwrap_or_else(|| fail(format!("refusing to write '{}': the name leaves the output directory", file.name)));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap_or_else(|e| fail(format!("Error creating '{}': {e}", parent.display())));
            }
            std::fs::write(&path, bytes).unwrap_or_else(|e| fail(format!("Error writing '{}': {e}", path.display())));
            path
        });
        written.push(path);
    }
    let restored = written.iter().filter(|p| p.is_some()).count();

    if json {
        let files: Vec<serde_json::Value> = report
            .files
            .iter()
            .zip(&written)
            .map(|(f, path)| {
                serde_json::json!({
                    "name": f.name,
                    "size": f.size,
                    "codec": f.codec,
                    "reads": f.reads,
                    "restored": path.is_some(),
                    "path": path.as_ref().map(|p| p.display().to_string()),
                    "problem": f.problem,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "reads": report.reads,
                "reads_skipped": skipped,
                "reads_inside_flanks": report.reads_inside_flanks,
                "files": files,
            })
        );
    } else {
        println!("Read {} reads ({} with other bases left out).", report.reads, skipped);
        if flanks.is_some() {
            println!("{} carried the outer primer sites.", report.reads_inside_flanks);
        }
        println!("The archive header lists {} file(s); {} rebuilt:", report.files.len(), restored);
        for (f, path) in report.files.iter().zip(&written) {
            match path {
                Some(p) => println!("  ✓ {} ({} B, {}) from {} reads -> {}", f.name, f.size, f.codec, f.reads, p.display()),
                None => println!("  ✗ {} ({} B, {}): {}", f.name, f.size, f.codec, f.problem.as_deref().unwrap_or("not rebuilt")),
            }
        }
    }
    if restored < report.files.len() {
        std::process::exit(1);
    }
}

/// `name` under `root`, or `None` if it is absolute or climbs out with `..`:
/// the name comes from DNA, so it is not trusted.
fn safe_path(root: &Path, name: &str) -> Option<PathBuf> {
    let relative = Path::new(name);
    if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_leave_the_output_directory_are_refused() {
        let root = Path::new("out");
        assert_eq!(safe_path(root, "pilot/pilot.txt"), Some(root.join("pilot").join("pilot.txt")));
        assert_eq!(safe_path(root, "../escape.txt"), None);
        assert_eq!(safe_path(root, "pilot/../../escape.txt"), None);
        assert_eq!(safe_path(root, "/etc/passwd"), None);
        assert_eq!(safe_path(root, "./pilot.txt"), None);
    }
}
