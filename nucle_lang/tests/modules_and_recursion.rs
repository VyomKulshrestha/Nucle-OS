//! Package imports that bring real declarations into a program, calls that
//! pass declared pools and recovery profiles, `for` loops over files that
//! bind per-iteration values, closures returned from functions, and
//! recursion of named functions.

use nucle_lang::{check_source, compile, execute_program};
use nucle_vfs::syscall::NucleOS;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nucle_lang_modules_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.join(name), format!("contents of {name}")).unwrap();
    }
    dir
}

fn errors(src: &str) -> Vec<String> {
    check_source(src)
        .diagnostics
        .into_iter()
        .filter(|d| d.level == nucle_lang::typeck::DiagnosticLevel::Error)
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

fn run(src: &str) -> Vec<String> {
    let mut plan = compile(src).unwrap_or_else(|e| panic!("compile: {e}"));
    let mut os = NucleOS::new(16);
    execute_program(&mut os, &mut plan, &scratch()).unwrap_or_else(|e| panic!("execute: {e}")).steps
}

#[test]
fn an_imported_function_runs_against_an_imported_pool() {
    let steps = run(
        "import { archive_with_guarantee, medical_archive, illumina_recovery } from \"nuclescript/presets\"\n\
         let saved: DnaFile = archive_with_guarantee(\"a.txt\", medical_archive, illumina_recovery)\n",
    );
    assert!(steps.iter().any(|s| s.contains("Stored 'medical_archive/a.txt'")), "{steps:?}");
}

#[test]
fn an_imported_pool_can_be_stored_into_directly_and_aliased() {
    let steps = run(
        "import { twist_archive as cheap } from \"nuclescript/presets\"\n\
         store \"b.txt\" into cheap\n",
    );
    assert!(steps.iter().any(|s| s.contains("Stored 'cheap/b.txt'")), "{steps:?}");
}

#[test]
fn imported_recovery_functions_accept_declared_pools_and_recovery_profiles() {
    let src = "import { recover_with_consensus, recovery_consensus_10x } from \"nuclescript/recovery\"\n\
               import { illumina_standard } from \"nuclescript/profiles\"\n\
               let r: Pool<Recovered> = recover_with_consensus(illumina_standard, recovery_consensus_10x)\n";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn a_declared_pool_of_the_wrong_profile_is_still_rejected() {
    let src = "import { recover_with_consensus, recovery_consensus_10x } from \"nuclescript/recovery\"\n\
               import { nanopore_standard } from \"nuclescript/profiles\"\n\
               let r: Pool<Recovered> = recover_with_consensus(nanopore_standard, recovery_consensus_10x)\n";
    assert!(errors(src).iter().any(|e| e.starts_with("E-ARG-TYPE-MISMATCH")), "{:?}", errors(src));
}

#[test]
fn importing_a_name_the_program_also_declares_is_a_duplicate() {
    let src = "import { medical_archive } from \"nuclescript/presets\"\n\
               pool medical_archive: DnaPool { codec: Ternary, redundancy: 2x, profile: Twist }\n";
    assert!(errors(src).iter().any(|e| e.starts_with("E-POOL-DUPLICATE")), "{:?}", errors(src));
}

#[test]
fn a_for_loop_passes_each_file_to_a_function_and_rebinds_its_let() {
    let steps = run(
        "pool p: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }\n\
         fn put(f: File) returns Result<DnaFile, Str> {\n\
         \x20   let r: Result<DnaFile, Str> = store f into p\n\
         }\n\
         for f in [\"a.txt\", \"b.txt\", \"c.txt\"] {\n\
         \x20   let saved: Result<DnaFile, Str> = put(f)\n\
         }\n",
    );
    for name in ["a.txt", "b.txt", "c.txt"] {
        assert!(steps.iter().any(|s| s.contains(&format!("Stored 'p/{name}'"))), "{steps:?}");
    }
}

#[test]
fn a_closure_returned_from_a_function_can_be_called() {
    let steps = run(
        "pool p: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }\n\
         fn make_storer(target: File) returns Fn() -> Result<DnaFile, Str> {\n\
         \x20   let storer: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {\n\
         \x20       let r: Result<DnaFile, Str> = store target into p\n\
         \x20   }\n\
         }\n\
         let storer: Fn() -> Result<DnaFile, Str> = make_storer(\"c.txt\")\n\
         let stored: Result<DnaFile, Str> = storer()\n",
    );
    assert!(steps.iter().any(|s| s.contains("Stored 'p/c.txt'")), "{steps:?}");
}

#[test]
fn a_named_function_can_recurse_to_a_base_case() {
    let steps = run(
        "enum Step {\n    Copy(Str),\n    Skip,\n}\n\
         pool p: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }\n\
         fn walk(step: Step, f: File) returns Result<DnaFile, Str> {\n\
         \x20   let out: DnaFile = match step {\n\
         \x20       Copy(name) => (store f into p)?,\n\
         \x20       Skip => (walk(Step::Copy(\"again\"), f))?,\n\
         \x20   }\n\
         }\n\
         let saved: Result<DnaFile, Str> = walk(Step::Skip, \"a.txt\")\n",
    );
    assert!(steps.iter().any(|s| s.contains("Stored 'p/a.txt'")), "{steps:?}");
}

#[test]
fn unbounded_recursion_stops_with_an_error_instead_of_overflowing() {
    let dir = scratch();
    let src = "pool p: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }\n\
               fn forever(f: File) returns Void {\n\
               \x20   let again: Void = forever(f)\n\
               }\n\
               let x: Void = forever(\"a.txt\")\n";
    let mut plan = compile(src).unwrap();
    let mut os = NucleOS::new(16);
    let err = execute_program(&mut os, &mut plan, &dir).unwrap_err();
    assert!(err.contains("exceeded the maximum call depth"), "{err}");

    let closure = "fn outer() returns Void {\n\
                   \x20   let spin: Fn() -> Void = fn() -> Void {\n\
                   \x20       let again: Void = spin()\n\
                   \x20   }\n\
                   \x20   let x: Void = spin()\n\
                   }\n\
                   let y: Void = outer()\n";
    let mut plan = compile(closure).unwrap();
    let err = execute_program(&mut NucleOS::new(16), &mut plan, &dir).unwrap_err();
    assert!(err.contains("exceeded the maximum call depth"), "{err}");
}
