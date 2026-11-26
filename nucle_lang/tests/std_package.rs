//! `nuclescript/std`, the standard library written in NucleScript: its
//! functions check clean, run on pools of any profile, and carry the
//! effects and ceilings their source declares.

use nucle_lang::effects::effect_summary;
use nucle_lang::package::checksum_sources;
use nucle_lang::typeck::DiagnosticLevel;
use nucle_lang::{check_source, compile, execute_program, Effect};
use std::path::Path;

const PRELUDE: &str = "\
import { clear, each_file, prune } from \"nuclescript/std\"
pool scratch: DnaPool { codec: Ternary, redundancy: 2x, profile: Nanopore }
pool keep: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
";

fn errors(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().filter(|d| d.level == DiagnosticLevel::Error).map(|d| d.code).collect()
}

/// Runs `src` against a fresh VFS; the files each pool holds afterwards.
fn run(src: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples");
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(src).unwrap();
    let report = execute_program(&mut os, &mut plan, &dir).expect("the run finishes");
    let names = |prefix: &str| os.dna_list(prefix).into_iter().map(|f| f.filename).collect::<Vec<_>>();
    (report.steps, names("scratch/"), names("keep/"))
}

#[test]
fn the_package_source_checks_clean_on_its_own() {
    let sources = checksum_sources("nuclescript/std");
    assert_eq!(sources.len(), 1, "std has one embedded source");
    let report = check_source(sources[0]);
    let problems: Vec<_> = report.diagnostics.iter().map(|d| d.code.clone()).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn clear_empties_the_pool_it_is_given_and_no_other() {
    let src = format!(
        "{PRELUDE}store \"sample_a.txt\" into scratch\nstore \"sample_b.txt\" into scratch\nstore \"sample_a.txt\" into keep\n\
         let wiped: Void = clear(scratch)\n"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
    let (steps, scratch, keep) = run(&src);
    assert!(steps.iter().any(|s| s.contains("files of scratch: 2 file(s)")), "{steps:?}");
    assert!(scratch.is_empty(), "{scratch:?}");
    assert_eq!(keep.len(), 1);
    // One generic body serves the Nanopore and the Illumina pool alike.
    let both = format!("{PRELUDE}let a: Void = clear(scratch)\nlet b: Void = clear(keep)\n");
    assert!(errors(&both).is_empty(), "{:?}", errors(&both));
}

#[test]
fn calling_clear_is_destructive() {
    let src = format!("{PRELUDE}let wiped: Void = clear(scratch)\n");
    let summary = effect_summary(&compile(&src).unwrap().program);
    let call = summary.declarations.iter().find(|d| d.name == "wiped").unwrap();
    assert_eq!(call.effect, Effect::Destructive);
}

#[test]
fn each_file_runs_the_closure_once_per_file_with_its_effect() {
    let src = format!(
        "{PRELUDE}store \"sample_a.txt\" into scratch\nstore \"sample_b.txt\" into scratch\n\
         let moved: Void = each_file(scratch, fn(f: File) -> Void {{\n    delete f from scratch confirm physical_key\n}})\n"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
    let summary = effect_summary(&compile(&src).unwrap().program);
    assert_eq!(summary.declarations.iter().find(|d| d.name == "moved").unwrap().effect, Effect::Destructive);
    let (_, scratch, _) = run(&src);
    assert!(scratch.is_empty(), "{scratch:?}");

    // The delete inside the closure still needs its confirmation.
    let unconfirmed = format!("{PRELUDE}let moved: Void = each_file(scratch, fn(f: File) -> Void {{\n    delete f from scratch\n}})\n");
    assert!(errors(&unconfirmed).contains(&"E-DELETE-UNCONFIRMED".to_string()), "{:?}", errors(&unconfirmed));
}

#[test]
fn prune_takes_a_deleting_closure_and_refuses_a_synthesising_one() {
    let deletes = format!(
        "{PRELUDE}store \"sample_a.txt\" into keep\n\
         let tidy: Void = prune(keep, fn(f: File) -> Void {{\n    delete f from keep confirm physical_key\n}})\n"
    );
    assert!(errors(&deletes).is_empty(), "{:?}", errors(&deletes));
    let (_, _, keep) = run(&deletes);
    assert!(keep.is_empty(), "{keep:?}");

    let synthesises = format!(
        "{PRELUDE}let tidy: Void = prune(keep, fn(f: File) -> Void {{\n    let strands: Pool<Twist> = synthesise keep via Twist confirm hardware\n}})\n"
    );
    assert!(errors(&synthesises).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()), "{:?}", errors(&synthesises));
}

#[test]
fn the_readme_example_checks_and_runs() {
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../packages/nuclescript-std/README.md")).unwrap();
    let start = readme.find("```nuclescript\n").expect("a nuclescript block") + "```nuclescript\n".len();
    let src = &readme[start..start + readme[start..].find("```").unwrap()];
    assert!(errors(src).is_empty(), "{:?}", errors(src));
    let (_, scratch, _) = run(src);
    assert!(scratch.is_empty(), "{scratch:?}");
}
