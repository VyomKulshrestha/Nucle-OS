//! Step 12: closures / higher-order functions. Covers the full pipeline
//! this feature touches -- parsing `Fn(...)`/anonymous `fn(...) {...}`
//! literals, type-checking (capture, return-type validation, arg
//! checking against a closure's own signature), effect analysis actually
//! seeing into a `let`-bound closure's real body, real end-to-end
//! execution of both a higher-order call and a captured binding, and
//! formatting.

use nucle_lang::ast::*;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, execute_program, format_source};
use std::path::Path;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("lex error: {}", e));
    Parser::new(tokens).parse_program().unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn diagnostic_codes(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| d.code).collect()
}

const POOL: &str = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n";

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

#[test]
fn fn_type_parses_into_the_expected_shape() {
    let program = parse(
        r#"
        fn apply(f: Fn(Str) -> Str) returns Void {
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert_eq!(func.params[0].ty, TypeExpr::Fn(vec![TypeExpr::Str], Box::new(TypeExpr::Str)));
}

#[test]
fn closure_literal_parses_into_the_expected_shape() {
    let program = parse(
        r#"
        fn f() returns Void {
            let g: Fn() -> Void = fn() -> Void {
            }
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    let Expr::Closure { params, return_type, body, .. } = &binding.expr else {
        panic!("expected a Closure expression, got {:?}", binding.expr);
    };
    assert!(params.is_empty());
    assert_eq!(*return_type, TypeExpr::Void);
    assert!(body.is_empty());
}

// ---------------------------------------------------------------------
// Typeck
// ---------------------------------------------------------------------

#[test]
fn a_let_bound_closure_has_no_diagnostics() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let g: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {{\n        let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    }}\n    let saved: Result<DnaFile, Str> = g()\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_closure_passed_inline_as_an_argument_has_no_diagnostics() {
    let src = format!(
        "{}fn apply(g: Fn() -> Result<DnaFile, Str>) returns Result<DnaFile, Str> {{\n    let result: Result<DnaFile, Str> = g()\n}}\nfn f() returns Result<DnaFile, Str> {{\n    let result: Result<DnaFile, Str> = apply(fn() -> Result<DnaFile, Str> {{\n        let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    }})\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_closure_captures_a_result_binding_from_its_enclosing_function() {
    // If capture didn't work, `attempt` inside the closure body would be
    // E-VARIABLE-UNDECLARED (or, since it's used as a match scrutinee,
    // E-MATCH-NOT-RESULT once the undeclared-variable path degrades).
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let g: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {{\n        let saved: DnaFile = match attempt {{\n            Ok(file) => file,\n            Err(reason) => (store \"b.txt\" into archive)?\n        }}\n    }}\n    let saved: Result<DnaFile, Str> = g()\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_closure_body_producing_the_wrong_type_is_rejected() {
    // The closure is declared to return `Result<DnaFile, Str>` but its
    // body's last binding is `Result<Void, Str>` (from a `delete`, not a
    // `store`) -- a genuine mismatch.
    let src = format!(
        "{}fn f() returns Void {{\n    let g: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {{\n        let attempt: Result<Void, Str> = delete \"a.txt\" from archive confirm physical_key\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-CLOSURE-RETURN-TYPE-MISMATCH".to_string()));
}

#[test]
fn calling_a_closure_with_the_wrong_arity_reports_the_existing_arity_code() {
    let src = format!(
        "{}fn f() returns Void {{\n    let g: Fn() -> Void = fn() -> Void {{\n    }}\n    let x: Void = g(1)\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-FUNCTION-ARITY".to_string()));
}

#[test]
fn a_genuinely_undeclared_call_still_reports_the_existing_code() {
    // Proves the closures-first lookup didn't disturb the pre-existing
    // path for a real typo.
    let src = format!("{}fn f() returns Void {{\n    let x: Void = totally_bogus_fn()\n}}\n", POOL);
    assert!(diagnostic_codes(&src).contains(&"E-FUNCTION-UNDECLARED".to_string()));
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

#[test]
fn calling_a_let_bound_closure_reflects_its_real_body_effect() {
    // The actual point of effects.rs's plumbing change: a `Destructive`
    // operation inside a closure's own body must be visible at the call
    // site, not incorrectly treated as inert just because the callee
    // isn't a top-level named function. This runs through `check_source`
    // (`typeck::TypeChecker::check_let`'s own confirmation gate), which
    // has real per-scope closure information -- unlike `effect_summary`
    // (used by `nucle explain`), which has none and treats any closure
    // call as inert (a separate, documented gap).
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        fn f() returns Void {
            let g: Fn() -> Void = fn() -> Void {
                delete "a.txt" from archive
            }
            let x: Void = g()
        }
        "#;
    assert!(
        diagnostic_codes(src).contains(&"E-SYNTHESIS-UNCONFIRMED".to_string()),
        "an unconfirmed delete inside a called closure must still require confirmation"
    );
}

#[test]
fn confirmed_destructive_effect_inside_a_closure_is_confirmed() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        fn f() returns Void {
            let g: Fn() -> Void = fn() -> Void {
                delete "a.txt" from archive confirm physical_key
            }
            let x: Void = g()
        }
        "#;
    assert!(!diagnostic_codes(src).contains(&"E-SYNTHESIS-UNCONFIRMED".to_string()));
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

#[test]
fn the_higher_order_call_retries_and_the_captured_binding_fallback_lands() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("closure_retry.nsl")).unwrap();

    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&source).expect("the example must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");

    // `first` (archive_with_retry): the passed closure's first attempt
    // already succeeds.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary")), "steps: {:?}", result.steps);
    // `second` (archive_with_logged_fallback): the captured binding's
    // Err arm fires, and its fallback into `secondary` actually lands.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into secondary")), "steps: {:?}", result.steps);
    // `third` (archive_with_retry again): `retry_once` genuinely calls
    // the closure a *second* time -- both attempts fail identically.
    let primary_failures = result.steps.iter().filter(|s| s.contains("✗ store into primary")).count();
    assert_eq!(primary_failures, 3, "expected 3 failed primary stores (second's own + retry_once's two), got: {:?}", result.steps);
    assert_eq!(os.dna_stat().file_count, 2);
}

// ---------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------

#[test]
fn formatting_a_closure_fixture_is_idempotent() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let g: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {{\n        let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    }}\n    let saved: Result<DnaFile, Str> = g()\n}}\n",
        POOL
    );
    let once = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}

#[test]
fn the_shipped_example_formats_and_is_idempotent() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("closure_retry.nsl")).unwrap();
    let once = format_source(&source).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}
