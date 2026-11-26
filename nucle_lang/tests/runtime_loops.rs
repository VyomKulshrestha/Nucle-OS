//! `for f in files of pool { ... }`: a loop over the files a pool holds
//! while the program runs, unlike the literal-list `for`, which the checker
//! unrolls before anything runs.

use nucle_lang::effects::effect_summary;
use nucle_lang::typeck::DiagnosticLevel;
use nucle_lang::{check_source, compile, execute_program, format_source, take_run_trace, Declaration, Effect, Lexer, Parser};
use std::path::Path;

const POOLS: &str = "\
pool scratch: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
pool keep: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
";

fn errors(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().filter(|d| d.level == DiagnosticLevel::Error).map(|d| d.code).collect()
}

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

#[test]
fn files_of_parses_and_is_kept_for_the_interpreter() {
    let src = format!("{POOLS}for f in files of scratch {{\n    delete f from scratch confirm physical_key\n}}\n");
    let tokens = Lexer::new(&src).tokenize().unwrap();
    let program = Parser::new(tokens).parse_program().unwrap();
    let Some(Declaration::For(loop_decl)) = program.declarations.last() else { panic!("expected a for loop") };
    assert_eq!(loop_decl.files_of.as_deref(), Some("scratch"));
    assert!(loop_decl.items.is_empty());
    // The checker keeps it, unlike a literal-list loop, which it unrolls.
    let plan = compile(&src).unwrap();
    assert!(plan.program.declarations.iter().any(|d| matches!(d, Declaration::For(f) if f.files_of.is_some())));
}

#[test]
fn the_loop_variable_is_a_file_and_the_pool_must_exist() {
    let ok = format!("{POOLS}for f in files of scratch {{\n    delete f from scratch confirm physical_key\n}}\n");
    assert!(errors(&ok).is_empty(), "{:?}", errors(&ok));
    let missing = format!("{POOLS}for f in files of nowhere {{\n    retrieve from keep\n}}\n");
    assert!(errors(&missing).contains(&"E-FOR-POOL-UNDECLARED".to_string()), "{:?}", errors(&missing));
    // The loop variable and the body's own names end with the loop.
    let leaked = format!("{POOLS}for f in files of scratch {{\n    let name: File = f\n}}\nlet again: File = name\n");
    assert!(!errors(&leaked).is_empty(), "a name bound in the loop body leaked out");
}

#[test]
fn a_delete_in_the_loop_still_needs_its_confirmation_and_counts_as_destructive() {
    let unconfirmed = format!("{POOLS}for f in files of scratch {{\n    delete f from scratch\n}}\n");
    assert!(errors(&unconfirmed).contains(&"E-DELETE-UNCONFIRMED".to_string()), "{:?}", errors(&unconfirmed));

    let src = format!("{POOLS}for f in files of scratch {{\n    delete f from scratch confirm physical_key\n}}\n");
    let summary = effect_summary(&compile(&src).unwrap().program);
    let loop_effect = summary.declarations.iter().find(|d| d.kind == "for").unwrap();
    assert_eq!(loop_effect.effect, Effect::Destructive);
}

/// It runs once per file the pool holds when it starts, and a delete in
/// the body does not change which files it visits.
#[test]
fn the_loop_runs_over_the_files_the_pool_holds() {
    let src = format!(
        "{POOLS}store \"sample_a.txt\" into scratch\nstore \"sample_b.txt\" into scratch\nstore \"sample_a.txt\" into keep\n\
         for f in files of scratch {{\n    delete f from scratch confirm physical_key\n}}\n"
    );
    let dir = examples_dir();
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).unwrap();
    let report = execute_program(&mut os, &mut plan, &dir).expect("the run finishes");
    assert!(report.steps.iter().any(|s| s.contains("for f in files of scratch: 2 file(s)")), "{:?}", report.steps);
    assert!(os.dna_list("scratch/").is_empty(), "the loop left files in scratch");
    assert_eq!(os.dna_list("keep/").len(), 1, "the loop touched another pool");
    // The monitor attributes both deletes to the loop, confirmed.
    let trace = take_run_trace();
    let deletes: Vec<_> = trace.effects.iter().filter(|e| e.effect == Effect::Destructive).collect();
    assert_eq!(deletes.len(), 2);
    assert!(deletes.iter().all(|e| e.confirmed));
}

/// A loop over an empty pool runs no passes; one inside a function walks
/// the pool its caller passes.
#[test]
fn the_loop_works_inside_a_function_and_over_an_empty_pool() {
    let src = format!(
        "{POOLS}fn clear(target: Pool<Illumina>) returns Void {{\n    for f in files of target {{\n        delete f from target confirm physical_key\n    }}\n}}\n\
         store \"sample_a.txt\" into keep\nlet none: Void = clear(scratch)\nlet done: Void = clear(keep)\n"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
    let dir = examples_dir();
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).unwrap();
    let report = execute_program(&mut os, &mut plan, &dir).expect("the run finishes");
    assert!(report.steps.iter().any(|s| s.contains("files of scratch: 0 file(s)")), "{:?}", report.steps);
    assert!(os.dna_list("keep/").is_empty(), "{:?}", report.steps);
}

#[test]
fn files_of_formats_stably() {
    let src = format!("{POOLS}for f in files of scratch {{\n    delete f from scratch confirm physical_key\n}}\n");
    let formatted = format_source(&src).unwrap();
    assert!(formatted.contains("for f in files of scratch {"), "{formatted}");
    assert_eq!(format_source(&formatted).unwrap(), formatted);
}
