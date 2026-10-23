//! Pool storage conditions (`storage: encapsulated at 4C`) and store
//! retention requirements (`retain: 1000y`), checked against Arrhenius
//! strand decay (`nucle_synth::decay`).

use nucle_lang::ast::{Declaration, StorageMediumKind, StorageSpec};
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, execute_program, format_source, generate_docs};
use nucle_vfs::syscall::NucleOS;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn codes(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| d.code).collect()
}

fn scratch() -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nucle_lang_retention_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("records.csv"), b"id,value\n1,42\n2,43\n3,44\n").unwrap();
    dir
}

fn run(src: &str) -> Vec<String> {
    let dir = scratch();
    let mut plan = compile(src).unwrap_or_else(|e| panic!("compile: {e}"));
    let mut os = NucleOS::new(16);
    execute_program(&mut os, &mut plan, &dir).unwrap_or_else(|e| panic!("execute: {e}")).steps
}

#[test]
fn storage_property_parses_medium_and_signed_temperature() {
    let src = "pool vault: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: bone at -20C }";
    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse_program().unwrap();
    let Declaration::Pool(pool) = &program.declarations[0] else { panic!("expected a pool") };
    assert_eq!(pool.storage, Some(StorageSpec { medium: StorageMediumKind::Bone, celsius: -20.0 }));
}

#[test]
fn bad_storage_syntax_is_a_parse_error() {
    let unknown = check_source("pool p: DnaPool { codec: Ternary, profile: Twist, storage: tape at 4C }");
    assert!(unknown.diagnostics.iter().any(|d| d.message.contains("unknown storage medium 'tape'")), "{:?}", unknown.diagnostics);
    let no_unit = check_source("pool p: DnaPool { codec: Ternary, profile: Twist, storage: bone at 4 }");
    assert!(no_unit.diagnostics.iter().any(|d| d.message.contains("storage temperature")), "{:?}", no_unit.diagnostics);
}

#[test]
fn retain_needs_declared_storage_and_a_positive_duration() {
    let without = "pool p: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }\nstore \"records.csv\" into p { retain: 100y }";
    assert!(codes(without).contains(&"E-RETAIN-WITHOUT-STORAGE".to_string()));
    let zero = "pool p: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: bone at 4C }\nstore \"records.csv\" into p { retain: 0y }";
    assert!(codes(zero).contains(&"E-RETAIN-INVALID".to_string()));
    let fine = "pool p: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: bone at 4C }\nstore \"records.csv\" into p { retain: 100y }";
    assert!(!codes(fine).iter().any(|c| c.starts_with("E-RETAIN")));
}

#[test]
fn impossible_temperatures_are_diagnosed() {
    let below_zero_k = "pool p: DnaPool { codec: Ternary, profile: Twist, storage: bone at -300C }";
    assert!(codes(below_zero_k).contains(&"E-STORAGE-TEMPERATURE".to_string()));
    let report = check_source("pool p: DnaPool { codec: Ternary, profile: Twist, storage: bone at 150C }");
    let hot = report.diagnostics.iter().find(|d| d.code == "E-STORAGE-TEMPERATURE").expect("warning for 150C");
    assert_eq!(hot.level, nucle_lang::typeck::DiagnosticLevel::Warning);
}

#[test]
fn a_met_requirement_logs_the_shelf_life() {
    let steps = run(
        "pool vault: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: encapsulated at 4C }\n\
         store \"records.csv\" into vault { retain: 1000y }\n",
    );
    let line = steps.iter().find(|s| s.starts_with("✓ retain 1000y")).unwrap_or_else(|| panic!("{steps:?}"));
    assert!(line.contains("'vault/records.csv'") && line.contains("encapsulated at 4C"), "{line}");
}

#[test]
fn an_unmet_requirement_is_an_err_the_program_can_handle() {
    // Bone at 25 °C loses a 150 nt strand's half in a few centuries; ten
    // thousand years is out of reach, so the store's Result is Err and
    // the match falls back to the capsule pool.
    let steps = run(
        "pool shelf: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: bone at 25C }\n\
         pool vault: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: encapsulated at 4C }\n\
         fn archive() returns Result<DnaFile, Str> {\n\
         \x20   let attempt: Result<DnaFile, Str> = store \"records.csv\" into shelf { retain: 10000y }\n\
         \x20   let saved: DnaFile = match attempt {\n\
         \x20       Ok(file) => file,\n\
         \x20       Err(reason) => (store \"records.csv\" into vault { retain: 10000y })?\n\
         \x20   }\n\
         }\n\
         let result: Result<DnaFile, Str> = archive()\n",
    );
    assert!(steps.iter().any(|s| s.starts_with("✗ retain 10000y: 'shelf/records.csv' is readable for only")), "{steps:?}");
    assert!(steps.iter().any(|s| s.starts_with("✓ retain 10000y: 'vault/records.csv'")), "{steps:?}");
}

#[test]
fn an_unmet_requirement_fails_a_statement_level_store() {
    let dir = scratch();
    let src = "pool shelf: DnaPool { codec: Ternary, redundancy: 2x, profile: Twist, storage: bone at 25C }\n\
               store \"records.csv\" into shelf { retain: 50000y }\n";
    let mut plan = compile(src).unwrap();
    let mut os = NucleOS::new(16);
    let err = execute_program(&mut os, &mut plan, &dir).unwrap_err();
    assert!(err.contains("short of the required 50000y"), "{err}");
}

#[test]
fn formatter_and_docs_keep_storage_and_retention() {
    let src = "pool vault:DnaPool{codec:Ternary,redundancy:3x,profile:Twist,storage:encapsulated at -20C}\nstore \"records.csv\" into vault{retain:500y}";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("storage: encapsulated at -20C"), "{formatted}");
    assert!(formatted.contains("retain: 500y"), "{formatted}");
    assert_eq!(format_source(&formatted).unwrap(), formatted, "formatting must be idempotent");

    let tokens = Lexer::new(&formatted).tokenize().unwrap();
    let docs = generate_docs(&Parser::new(tokens).parse_program().unwrap());
    assert!(docs.contains("storage: encapsulated at -20C"), "{docs}");
}

#[test]
fn silica_pools_parse_and_check_retention_against_grass_kinetics() {
    let src = "pool beads: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: silica at 4C }";
    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse_program().unwrap();
    let Declaration::Pool(pool) = &program.declarations[0] else { panic!("expected a pool") };
    assert_eq!(pool.storage, Some(StorageSpec { medium: StorageMediumKind::Silica, celsius: 4.0 }));
    // Silica at 4 °C keeps a 150 nt strand for a few thousand years, so a
    // century passes and a million years does not.
    let steps = run(&format!("{src}\nstore \"records.csv\" into beads {{ retain: 100y }}\n"));
    assert!(steps.iter().any(|s| s.starts_with("✓ retain 100y") && s.contains("silica at 4C")), "{steps:?}");
    let dir = scratch();
    let mut plan = compile(&format!("{src}\nstore \"records.csv\" into beads {{ retain: 1000000y }}\n")).unwrap();
    assert!(execute_program(&mut NucleOS::new(16), &mut plan, &dir).is_err());
}
