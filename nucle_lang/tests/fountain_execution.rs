use nucle_lang::{compile, execute_program};
use nucle_vfs::syscall::NucleOS;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn scratch_dir(label: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nucle_lang_fountain_{label}_{}_{}",
        std::process::id(),
        id
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn assert_fountain_file(os: &mut NucleOS, filename: &str, expected: &[u8]) {
    let file = os
        .catalog
        .get_by_name(filename)
        .expect("Fountain store must create a VFS file");
    assert_eq!(file.codec, "dna-fountain");
    assert!(
        file.manifest
            .as_ref()
            .and_then(|manifest| manifest.fountain_config.as_ref())
            .is_some(),
        "Fountain parameters must be persisted with the representation"
    );
    assert_eq!(os.dna_read(filename).unwrap(), expected);
}

fn assert_no_unsupported_codec_diagnostic(plan: &nucle_lang::CompiledPlan) {
    assert!(
        !plan
            .type_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E-POOL-UNSUPPORTED-CODEC"
                || diagnostic.code == "E-PIPELINE-UNSUPPORTED-CODEC"),
        "Fountain emitted an obsolete unsupported-codec diagnostic: {:?}",
        plan.type_report.diagnostics
    );
}

#[test]
fn fountain_pool_store_executes_and_round_trips_through_the_vfs() {
    let dir = scratch_dir("store");
    let data = b"NucleScript Fountain direct store roundtrip";
    std::fs::write(dir.join("direct.bin"), data).unwrap();
    let source = r#"
pool archive: DnaPool {
    codec: Fountain,
    redundancy: 4x,
    profile: Illumina
}

let saved: Result<DnaFile, Str> = store "direct.bin" into archive
"#;

    let mut plan = compile(source).expect("Fountain pools must compile");
    assert_no_unsupported_codec_diagnostic(&plan);
    let mut os = NucleOS::new(10);
    execute_program(&mut os, &mut plan, &dir).expect("Fountain store must execute");
    assert_fountain_file(&mut os, "direct.bin", data);

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fountain_pipeline_executes_store_and_verify_roundtrip() {
    let dir = scratch_dir("pipeline");
    let data = b"NucleScript Fountain pipeline roundtrip";
    std::fs::write(dir.join("pipeline.bin"), data).unwrap();
    let source = r#"
pool archive: DnaPool {
    codec: Fountain,
    redundancy: 4x,
    profile: Illumina
}

pipeline fountain_backup {
    encode "pipeline.bin" using Fountain,
    protect with redundancy 4x,
    store into archive,
    verify roundtrip
}
"#;

    let mut plan = compile(source).expect("Fountain pipelines must compile");
    assert_no_unsupported_codec_diagnostic(&plan);
    let mut os = NucleOS::new(10);
    let report = execute_program(&mut os, &mut plan, Path::new(&dir))
        .expect("Fountain pipeline must execute");
    assert!(
        report
            .steps
            .iter()
            .any(|step| step.contains("verify roundtrip") && step.contains("recovered exactly")),
        "pipeline did not report an exact roundtrip: {:?}",
        report.steps
    );
    assert_fountain_file(&mut os, "pipeline.bin", data);

    std::fs::remove_dir_all(dir).unwrap();
}
