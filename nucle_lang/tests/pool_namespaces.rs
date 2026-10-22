//! Each declared pool is its own namespace in the VFS.
//!
//! Storing one file into several pools must leave one independent replica
//! per pool. Before pools were namespaced, the second `store` of the same
//! file name silently became version 2 of the first, so a tiered archive
//! kept a single copy, and `delete`/`retrieve` ignored the pool they named.

use nucle_lang::{compile, execute_program};
use nucle_synth::profiles::HardwareProfile;
use nucle_vfs::syscall::NucleOS;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn scratch_with(files: &[(&str, &[u8])]) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nucle_lang_pools_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, data) in files {
        std::fs::write(dir.join(name), data).unwrap();
    }
    dir
}

const POOLS: &str = "\
pool hot: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
pool cold: DnaPool { codec: YinYang, redundancy: 3x, profile: Twist }
";

fn run(source: &str, dir: &Path) -> (NucleOS, Vec<String>) {
    let mut plan = compile(source).unwrap_or_else(|e| panic!("compile: {e}"));
    let mut os = NucleOS::new(16);
    let report = execute_program(&mut os, &mut plan, dir).unwrap_or_else(|e| panic!("execute: {e}"));
    (os, report.steps)
}

#[test]
fn the_same_file_in_two_pools_is_two_independent_replicas() {
    let dir = scratch_with(&[("report.csv", b"id,value\n1,42\n2,43\n")]);
    let (mut os, steps) = run(
        &format!("{POOLS}store \"report.csv\" into hot\nstore \"report.csv\" into cold\n"),
        &dir,
    );
    assert!(steps.iter().any(|s| s.contains("Stored 'hot/report.csv'")), "{steps:?}");
    assert!(steps.iter().any(|s| s.contains("Stored 'cold/report.csv'")), "{steps:?}");

    for path in ["hot/report.csv", "cold/report.csv"] {
        let file = os.catalog.get_by_name(path).unwrap_or_else(|| panic!("{path} missing"));
        assert_eq!(file.version, 1, "{path} must be its own first version");
        assert_eq!(os.dna_read(path).unwrap(), b"id,value\n1,42\n2,43\n");
    }
    assert_eq!(os.catalog.get_by_name("hot/report.csv").unwrap().codec, "ternary-rotating-cipher");
    assert_eq!(os.catalog.get_by_name("cold/report.csv").unwrap().codec, "yin-yang");
}

#[test]
fn delete_only_touches_the_named_pool() {
    let dir = scratch_with(&[("a.txt", b"alpha")]);
    let (mut os, _) = run(
        &format!(
            "{POOLS}store \"a.txt\" into hot\nstore \"a.txt\" into cold\ndelete \"a.txt\" from hot confirm physical_key\n"
        ),
        &dir,
    );
    assert!(os.catalog.get_by_name("hot/a.txt").is_none());
    assert_eq!(os.dna_read("cold/a.txt").unwrap(), b"alpha");
}

#[test]
fn retrieve_only_returns_files_from_the_named_pool() {
    let dir = scratch_with(&[("a.txt", b"alpha"), ("b.txt", b"bravo")]);
    let (_, steps) = run(
        &format!(
            "{POOLS}store \"a.txt\" into hot\nstore \"b.txt\" into cold\nretrieve from cold where {{ name contains \"txt\" }}\n"
        ),
        &dir,
    );
    let hits: Vec<&String> = steps.iter().filter(|s| s.starts_with("  - ")).collect();
    assert_eq!(hits.len(), 1, "{steps:?}");
    assert!(hits[0].contains("cold/b.txt"), "{steps:?}");
}

#[test]
fn retrieve_in_expression_position_runs_the_search() {
    let dir = scratch_with(&[("a.txt", b"alpha")]);
    let (_, steps) = run(
        &format!(
            "{POOLS}store \"a.txt\" into hot\nfn find() returns Void {{\n    let found: Void = retrieve from hot where {{ name contains \"a\" }}\n}}\nlet x: Void = find()\n"
        ),
        &dir,
    );
    assert!(steps.iter().any(|s| s.starts_with("✓ retrieve from hot")), "{steps:?}");
}

#[test]
fn simulate_store_into_a_sequencer_pool_uses_twist_synthesis() {
    let dir = scratch_with(&[("a.txt", b"alpha bravo charlie")]);
    let (os, _) = run(
        "pool reads: DnaPool { codec: Ternary, redundancy: 3x, profile: Nanopore }\nsimulate store \"a.txt\" into reads { coverage: 12x }\n",
        &dir,
    );
    assert!(os.simulate_noise);
    assert_eq!(os.noise_config.synthesis_profile, HardwareProfile::TwistBioscience);
    assert_eq!(os.noise_config.sequencing_profile, HardwareProfile::OxfordNanopore);
}
