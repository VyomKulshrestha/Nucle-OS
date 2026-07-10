//! Step 11: pattern matching over `Result<T, E>`. Covers the full
//! pipeline this feature touches -- parsing `match`/`Ok`/`Err`/`=>`,
//! type-checking its validity rules (scrutinee must be Result-shaped,
//! both arms must unify), conservative effect-joining across both arms,
//! real end-to-end execution of both arms, and formatting.

use nucle_lang::ast::*;
use nucle_lang::effects::effect_summary;
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
fn match_expr_parses_into_the_expected_shape() {
    let program = parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let attempt: Result<DnaFile, Str> = store "a.txt" into archive
            let saved: DnaFile = match attempt {
                Ok(file) => file,
                Err(reason) => file
            }
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(second) = &func.body[1] else { panic!("expected a let") };
    let Expr::Match { scrutinee, ok_pattern, ok_body, err_pattern, err_body } = &second.expr else {
        panic!("expected a Match expression, got {:?}", second.expr);
    };
    assert_eq!(**scrutinee, Expr::Variable("attempt".to_string()));
    assert_eq!(ok_pattern, "file");
    assert_eq!(**ok_body, Expr::Variable("file".to_string()));
    assert_eq!(err_pattern, "reason");
    assert_eq!(**err_body, Expr::Variable("file".to_string()));
}

#[test]
fn match_accepts_an_optional_trailing_comma() {
    parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let attempt: Result<DnaFile, Str> = store "a.txt" into archive
            let saved: DnaFile = match attempt {
                Ok(file) => file,
                Err(reason) => file,
            }
        }
        "#,
    );
}

#[test]
fn match_requires_ok_before_err() {
    // Arm order is fixed (`Ok` then `Err`) -- `Result` is a closed
    // two-variant type with no general/reorderable arm machinery (see
    // `Expr::Match`'s doc comment in ast.rs). Swapping the order is a
    // parse error, not a semantic one.
    let tokens = Lexer::new(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let attempt: Result<DnaFile, Str> = store "a.txt" into archive
            let saved: DnaFile = match attempt {
                Err(reason) => reason,
                Ok(file) => file
            }
        }
        "#,
    )
    .tokenize()
    .unwrap();
    assert!(Parser::new(tokens).parse_program().is_err());
}

// ---------------------------------------------------------------------
// Typeck
// ---------------------------------------------------------------------

#[test]
fn well_formed_match_over_a_result_binding_has_no_diagnostics() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => (store \"b.txt\" into archive)?\n    }}\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn matching_a_non_result_expression_is_rejected() {
    let src = format!(
        "{}fn f() returns Void {{\n    let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina\n    let x: Void = match noisy {{\n        Ok(a) => a,\n        Err(b) => b\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-NOT-RESULT".to_string()));
}

#[test]
fn mismatched_arm_types_are_rejected() {
    // `Ok(file) => file` produces `DnaFile`; the `Err` arm produces a
    // still-wrapped `Result<DnaFile, Str>` instead (no `?`) -- the two
    // arms disagree.
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => store \"b.txt\" into archive\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-ARM-TYPE-MISMATCH".to_string()));
}

#[test]
fn an_arm_body_that_is_not_a_supported_shape_is_rejected() {
    // Neither arm's bound name, `?`, a Result-shaped expression, nor a
    // Pool-shaped expression -- a bare string literal is none of those.
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => \"unused\"\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-ARM-UNTYPABLE".to_string()));
}

#[test]
fn question_mark_inside_an_arm_validates_against_the_enclosing_functions_return_type() {
    // `?` inside a match arm is checked the same way it is anywhere else
    // -- against `enclosing_result_return`, not some arm-local notion --
    // so it's rejected outside any Result-returning function even when
    // nested inside a match arm's body.
    let src = format!(
        "{}fn f() returns Void {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: Void = match attempt {{\n        Ok(file) => file,\n        Err(reason) => (store \"b.txt\" into archive)?\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-TRY-OUTSIDE-RESULT-FN".to_string()));
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

#[test]
fn destructive_effect_in_only_the_err_arm_still_requires_confirmation() {
    // Mirrors `If`'s existing "join across the untaken branch too"
    // precedent (and Step 9's identical reasoning for `?`): a
    // `Destructive` operation that only runs in the `Err` arm still
    // counts, since effect analysis never models "this branch might not
    // run".
    let program = parse(
        r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        fn f() returns Void {
            let attempt: Result<DnaFile, Str> = store "a.txt" into archive
            let x: Void = match attempt {
                Ok(file) => delete "b.txt" from archive confirm physical_key,
                Err(reason) => delete "a.txt" from archive
            }
        }
        "#,
    );
    let summary = effect_summary(&program);
    let func_effect = summary.declarations.iter().find(|d| d.kind == "function").expect("expected a function entry");
    assert_eq!(func_effect.effect, Effect::Destructive);
    assert!(func_effect.confirmation_required);
    assert!(!func_effect.confirmed, "the unconfirmed delete in the Err arm must still be reported as unconfirmed");
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

#[test]
fn the_ok_arm_runs_and_the_err_arm_runs_and_its_fallback_write_lands() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("match_result_fallback.nsl")).unwrap();

    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&source).expect("the example must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");

    // First call's primary store succeeds -- the `Ok` arm ran.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary")), "steps: {:?}", result.steps);
    // Second call's primary store fails (same filename, already exists),
    // and its `Err` arm's fallback into `secondary` actually lands.
    assert!(
        result.steps.iter().any(|s| s.contains("✗ store into primary") && s.contains("already exists")),
        "expected a caught duplicate-file failure, got: {:?}",
        result.steps
    );
    assert!(result.steps.iter().any(|s| s.contains("✓ store into secondary")), "steps: {:?}", result.steps);
    assert_eq!(os.dna_stat().file_count, 2);
}

// ---------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------

#[test]
fn formatting_a_match_fixture_is_idempotent() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => (store \"b.txt\" into archive)?,\n    }}\n}}\n",
        POOL
    );
    let once = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}

#[test]
fn the_shipped_example_formats_and_is_idempotent() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("match_result_fallback.nsl")).unwrap();
    let once = format_source(&source).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}
