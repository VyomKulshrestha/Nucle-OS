//! The broken promises `tests/soundness.rs`'s runtime monitor found, each
//! pinned as the smallest program that showed it.

use nucle_lang::ast::*;
use nucle_lang::effects::effect_summary;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, execute_program};
use nucle_vfs::syscall::NucleOS;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("lex error: {}", e));
    Parser::new(tokens).parse_program().unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn effect_of(src: &str, name: &str) -> Effect {
    effect_summary(&parse(src)).declarations.into_iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no declaration '{name}'")).effect
}

fn diagnostic_codes(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| d.code).collect()
}

const POOL: &str = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n";

/// A pipeline lowers to a real store, so `nucle explain` must not call it
/// Pure.
#[test]
fn a_pipeline_that_stores_is_synthesis() {
    let src = format!("{POOL}pipeline backup {{\n    encode \"a.txt\" using Ternary,\n    protect with redundancy 4x,\n    store into archive\n}}\n");
    assert_eq!(effect_of(&src, "backup"), Effect::Synthesis);
}

/// A call reaches the closures the callee binds in its own body.
#[test]
fn a_call_reaches_a_closure_bound_inside_the_callee() {
    let src = format!(
        "{POOL}fn purge() returns Void {{\n    let wipe: Fn() -> Void = fn() -> Void {{\n        delete \"a.txt\" from archive confirm physical_key\n    }}\n    let done: Void = wipe()\n}}\nlet ran: Void = purge()\n"
    );
    assert_eq!(effect_of(&src, "ran"), Effect::Destructive);
}

/// The same at top level: a later call to a closure bound earlier.
#[test]
fn a_top_level_call_reaches_a_closure_bound_earlier() {
    let src = format!(
        "{POOL}let wipe: Fn() -> Void = fn() -> Void {{\n    delete \"a.txt\" from archive confirm physical_key\n}}\nlet ran: Void = wipe()\n"
    );
    assert_eq!(effect_of(&src, "ran"), Effect::Destructive);
}

/// A delete hidden behind a function's own closure must not get through a
/// `confirm hardware` slot.
#[test]
fn a_delete_behind_a_callees_closure_does_not_fit_confirm_hardware() {
    let src = format!(
        "{POOL}fn purge() returns Void {{\n    let wipe: Fn() -> Void = fn() -> Void {{\n        delete \"a.txt\" from archive confirm physical_key\n    }}\n    let done: Void = wipe()\n}}\n\
         fn with_hardware(action: Fn() -> Void confirm hardware) returns Void {{\n    let done: Void = action()\n}}\n\
         let ran: Void = with_hardware(fn() -> Void {{\n    let inner: Void = purge()\n}})\n"
    );
    assert!(diagnostic_codes(&src).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()), "{:?}", diagnostic_codes(&src));
}

/// A closure that stores and deletes needs both confirmations, so it fits
/// neither ceiling, though its worst effect alone fits `confirm
/// physical_key`.
#[test]
fn a_closure_that_stores_and_deletes_fits_no_single_ceiling() {
    let src = format!(
        "{POOL}fn mixed() returns Void {{\n    delete \"a.txt\" from archive confirm physical_key\n    store \"b.txt\" into archive\n}}\n\
         fn with_key(action: Fn() -> Void confirm physical_key) returns Void {{\n    let done: Void = action()\n}}\n\
         let ran: Void = with_key(fn() -> Void {{\n    let inner: Void = mixed()\n}})\n"
    );
    assert!(diagnostic_codes(&src).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()), "{:?}", diagnostic_codes(&src));
}

const MAKER: &str = "fn make() returns Fn() -> Void {\n    let wipe: Fn() -> Void = fn() -> Void {\n        delete \"a.txt\" from archive confirm physical_key\n    }\n}\n";

/// A closure returned from a function is out of the checker's sight: it
/// must fit no ceiling, rather than every one.
#[test]
fn a_closure_returned_from_a_function_fits_no_ceiling() {
    let src = format!(
        "{POOL}{MAKER}fn with_hardware(action: Fn() -> Void confirm hardware) returns Void {{\n    let done: Void = action()\n}}\n\
         let made: Fn() -> Void = make()\nlet ran: Void = with_hardware(made)\n"
    );
    let report = check_source(&src);
    let mismatch = report.diagnostics.iter().find(|d| d.code == "E-FN-EFFECT-ARG-MISMATCH").unwrap_or_else(|| panic!("{:?}", report.diagnostics));
    assert!(mismatch.message.contains("cannot see"), "{}", mismatch.message);
}

/// ...and a call through it is reported with every effect, not as Pure.
#[test]
fn a_call_through_a_returned_closure_has_every_effect() {
    let src = format!("{POOL}{MAKER}let made: Fn() -> Void = make()\nlet ran: Void = made()\n");
    assert_eq!(effect_of(&src, "ran"), Effect::Destructive);
}

const ALIAS: &str = "let wipe: Fn() -> Void = fn() -> Void {\n    delete \"a.txt\" from archive confirm physical_key\n}\nlet again: Fn() -> Void = wipe\nlet ran: Void = again()\n";

/// Another name for a closure resolves to that closure, exactly.
#[test]
fn an_alias_of_a_closure_has_that_closures_effect() {
    let summary = effect_summary(&parse(&format!("{POOL}{ALIAS}")));
    let ran = summary.declarations.iter().find(|d| d.name == "ran").unwrap();
    assert_eq!(ran.effects, nucle_lang::effects::EffectSet::of(Effect::Destructive));
}

/// The alias also exists when the program runs: calling it used to find
/// nothing to call.
#[test]
fn an_alias_of_a_closure_is_bound_at_run_time() {
    let dir = std::env::temp_dir().join(format!("nsl_alias_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    let mut plan = compile(&format!("{POOL}store \"a.txt\" into archive\n{ALIAS}")).unwrap();
    let report = execute_program(&mut NucleOS::new(10), &mut plan, &dir).unwrap();
    assert!(report.steps.iter().any(|s| s.starts_with("delete from archive: removed 'archive/a.txt'")), "{:?}", report.steps);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A parameter without a ceiling may hold any closure, so forwarding it
/// into a `confirm hardware` slot must not pass: the closure the caller
/// gave runs under that ceiling.
#[test]
fn a_parameter_without_a_ceiling_fits_no_ceiling() {
    let src = format!(
        "{POOL}fn with_hardware(action: Fn() -> Void confirm hardware) returns Void {{\n    let done: Void = action()\n}}\n\
         fn relay(action: Fn() -> Void) returns Void {{\n    let done: Void = with_hardware(action)\n}}\n\
         fn relay_inline(action: Fn() -> Void) returns Void {{\n    let done: Void = with_hardware(fn() -> Void {{\n        let inner: Void = action()\n    }})\n}}\n"
    );
    let codes = diagnostic_codes(&src);
    assert_eq!(codes.iter().filter(|c| *c == "E-FN-EFFECT-ARG-MISMATCH").count(), 2, "{codes:?}");
}
