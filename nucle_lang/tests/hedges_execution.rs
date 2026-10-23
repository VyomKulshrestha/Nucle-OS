//! `codec: Hedges` pools and `encode ... using Hedges` pipelines run through
//! the real VFS.

use nucle_lang::{compile, execute_program};
use nucle_vfs::syscall::NucleOS;
use std::path::PathBuf;

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nucle_lang_hedges_{label}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn hedges_pools_and_pipelines_execute_and_round_trip() {
    let dir = scratch_dir("store");
    std::fs::write(dir.join("direct.bin"), b"stored into a HEDGES pool").unwrap();
    std::fs::write(dir.join("piped.bin"), b"encoded by a HEDGES pipeline").unwrap();
    let source = r#"
pool archive: DnaPool {
    codec: Hedges,
    redundancy: 3x,
    profile: Illumina
}

let saved: Result<DnaFile, Str> = store "direct.bin" into archive

pipeline backup {
    encode "piped.bin" using hedges,
    protect with redundancy 2x,
    store into archive,
    verify roundtrip
}
"#;
    let mut plan = compile(source).expect("Hedges pools and pipelines must compile");
    assert!(plan.type_report.diagnostics.iter().all(|d| !d.code.contains("CODEC")), "{:?}", plan.type_report.diagnostics);
    let mut os = NucleOS::new(10);
    execute_program(&mut os, &mut plan, &dir).expect("Hedges programs must execute");
    for (name, expected) in [("archive/direct.bin", &b"stored into a HEDGES pool"[..]), ("archive/piped.bin", &b"encoded by a HEDGES pipeline"[..])] {
        assert_eq!(os.catalog.get_by_name(name).expect(name).codec, "hedges");
        assert_eq!(os.dna_read(name).unwrap(), expected);
    }

    // Pool codecs format as `Hedges`; pipeline codecs keep their spelling.
    let formatted = nucle_lang::formatter::format_source(source).unwrap();
    assert!(formatted.contains("codec: Hedges") && formatted.contains("using hedges"), "{formatted}");

    std::fs::remove_dir_all(dir).unwrap();
}
