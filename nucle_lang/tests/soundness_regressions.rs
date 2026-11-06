//! The broken promises `tests/soundness.rs`'s runtime monitor found, each
//! pinned as the smallest program that showed it.

use nucle_lang::ast::*;
use nucle_lang::effects::effect_summary;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::check_source;

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
