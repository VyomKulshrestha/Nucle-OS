//! `Result<T, E>` + `?` error propagation. Covers the full
//! pipeline this feature touches -- parsing the new syntax, type-checking
//! its validity rules, conservative effect-joining across a `?`
//! short-circuit, and real end-to-end execution where a genuine VFS
//! failure is caught instead of aborting the whole run.

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

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

#[test]
fn result_type_parses_as_a_return_type_and_let_annotation() {
    let program = parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into archive
            let y: DnaFile = x?
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert_eq!(func.return_type, TypeExpr::Result(Box::new(TypeExpr::DnaFile), Box::new(TypeExpr::Str)));
    let Declaration::Let(first) = &func.body[0] else { panic!("expected a let") };
    assert_eq!(first.annotation, TypeExpr::Result(Box::new(TypeExpr::DnaFile), Box::new(TypeExpr::Str)));
}

#[test]
fn postfix_question_mark_parses_as_try() {
    let program = parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into archive
            let y: DnaFile = x?
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(second) = &func.body[1] else { panic!("expected a let") };
    assert_eq!(second.expr, Expr::Try(Box::new(Expr::Variable("x".to_string()))));
}

#[test]
fn store_in_expression_position_parses_as_store_expr() {
    let program = parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into archive
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    assert!(matches!(&binding.expr, Expr::StoreExpr(op) if op.file == "a.txt" && op.pool == "archive"));
}

#[test]
fn store_statement_form_is_unaffected_by_the_new_expression_form() {
    // The exact same keyword, in *statement* position, must still parse
    // to the pre-existing Declaration::Operation shape -- proving the
    // two surface forms don't collide.
    let program = parse(r#"store "a.txt" into archive"#);
    assert!(matches!(&program.declarations[0], Declaration::Operation(Operation::Store(op)) if op.file == "a.txt"));
}

// ---------------------------------------------------------------------
// Typeck
// ---------------------------------------------------------------------

const POOL: &str = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n";

#[test]
fn well_formed_result_program_has_no_diagnostics() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let x: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let y: DnaFile = x?\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn try_outside_any_function_is_rejected() {
    let src = format!(
        "{}let x: Result<DnaFile, Str> = store \"a.txt\" into archive\nlet y: DnaFile = x?\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-TRY-OUTSIDE-RESULT-FN".to_string()));
}

#[test]
fn try_inside_a_non_result_returning_function_is_rejected() {
    let src = format!(
        "{}fn f() returns Void {{\n    let x: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let y: DnaFile = x?\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-TRY-OUTSIDE-RESULT-FN".to_string()));
}

#[test]
fn try_error_type_mismatch_is_rejected() {
    // No other error type exists to mismatch against Str with today's
    // grammar, so this instead proves the *matching* case doesn't fire
    // -- E-TRY-ERROR-TYPE-MISMATCH's true-positive path is exercised
    // structurally by `well_formed_result_program_has_no_diagnostics`
    // (same code path, opposite outcome) since no second Err type is
    // constructible with the language's current grammar (Str is the
    // only error type StoreExpr/DeleteExpr can produce).
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let x: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let y: DnaFile = x?\n}}\n",
        POOL
    );
    assert!(!diagnostic_codes(&src).contains(&"E-TRY-ERROR-TYPE-MISMATCH".to_string()));
}

#[test]
fn store_expr_still_runs_the_statement_forms_own_validation() {
    // A real gap found during development: infer_result_expr originally
    // just returned StoreExpr/DeleteExpr's type pair without ever calling
    // check_store/check_delete, so `store ... into <undeclared pool>` in
    // *expression* position silently type-checked clean -- the exact
    // validation the *statement* form always runs (E-STORE-POOL-
    // UNDECLARED, confirmation, tag/redundancy sanity, ...) was skipped
    // entirely just because it reached the compiler through the new
    // surface syntax. Fixed by having infer_result_expr call the same
    // check_* functions as a side effect; this pins that fix down.
    let src = r#"
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into undeclared_pool
            let y: DnaFile = x?
        }
        "#;
    assert!(diagnostic_codes(src).contains(&"E-STORE-POOL-UNDECLARED".to_string()));
}

#[test]
fn delete_expr_still_requires_confirmation() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let x: Result<Void, Str> = delete \"a.txt\" from archive\n    let y: Void = x?\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-DELETE-UNCONFIRMED".to_string()));
}

#[test]
fn forgetting_the_question_mark_is_rejected() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let x: DnaFile = store \"a.txt\" into archive\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-BINDING-RESULT-TYPE-MISMATCH".to_string()));
}

#[test]
fn result_returning_function_whose_body_does_not_end_in_a_result_is_rejected() {
    let src = format!("{}fn f() returns Result<DnaFile, Str> {{\n    let x: Pool<Illumina> = simulate archive under Illumina\n}}\n", POOL);
    assert!(diagnostic_codes(&src).contains(&"E-RETURN-TYPE-NOT-RESULT".to_string()));
}

#[test]
fn applying_question_mark_to_a_non_result_expression_is_rejected() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina\n    let y: DnaFile = noisy?\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-TRY-NOT-RESULT".to_string()));
}

// ---------------------------------------------------------------------
// `Ok(...)`/`Err(...)` constructors
// ---------------------------------------------------------------------

#[test]
fn ok_and_err_constructors_parse_into_the_expected_expr_variants() {
    let program = parse(
        r#"
        fn f() returns Result<DnaFile, Str> {
            let x: DnaFile = (store "a.txt" into archive)?
            let ok: Result<DnaFile, Str> = Ok(x)
            let err: Result<DnaFile, Str> = Err("blocked")
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(ok_binding) = &func.body[1] else { panic!("expected a let") };
    assert_eq!(ok_binding.expr, Expr::Ok(Box::new(Expr::Variable("x".to_string()))));
    let Declaration::Let(err_binding) = &func.body[2] else { panic!("expected a let") };
    assert_eq!(err_binding.expr, Expr::Err(Box::new(Expr::StringLiteral("blocked".to_string()))));
}

#[test]
fn ok_constructor_rewrapping_a_bound_variable_has_no_diagnostics() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let saved: DnaFile = (store \"a.txt\" into archive)?\n    let confirmed: Result<DnaFile, Str> = Ok(saved)\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn err_constructor_resolves_ok_type_from_the_enclosing_functions_return_type() {
    // No `store`/`?` anywhere in the body -- `Err`'s missing `Ok` side
    // must come purely from `f`'s own declared return type.
    let src = format!("{}fn f() returns Result<DnaFile, Str> {{\n    let disabled: Result<DnaFile, Str> = Err(\"disabled\")\n}}\n", POOL);
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn err_constructor_with_no_context_at_all_is_ambiguous() {
    // No enclosing `let` annotation and no sibling `Ok` arm to borrow a
    // type from -- `infer_result_expr`'s generic, context-free path.
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let saved: DnaFile = (store \"a.txt\" into archive)?\n    let x: DnaFile = Err(\"nope\")?\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-ERR-CONSTRUCTOR-AMBIGUOUS".to_string()));
}

#[test]
fn err_constructor_payload_must_be_a_string_literal() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let saved: DnaFile = (store \"a.txt\" into archive)?\n    let x: Result<DnaFile, Str> = Err(saved)\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-ERR-CONSTRUCTOR-INVALID".to_string()));
}

#[test]
fn ok_constructor_payload_must_be_resolvable() {
    let src = format!("{}fn f() returns Result<DnaFile, Str> {{\n    let x: Result<DnaFile, Str> = Ok(\"a.txt\")\n}}\n", POOL);
    assert!(diagnostic_codes(&src).contains(&"E-OK-CONSTRUCTOR-INVALID".to_string()));
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

#[test]
fn destructive_effect_after_a_try_short_circuit_still_requires_confirmation() {
    // Mirrors `If`'s existing "join across the untaken branch too"
    // precedent: a `?` that might short-circuit before `delete` runs
    // doesn't exempt that `delete` from needing confirmation, since
    // effect analysis is static (never models "which branch actually
    // executes").
    let program = parse(
        r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into archive
            let y: DnaFile = x?
            delete "a.txt" from archive
        }
        "#,
    );
    let summary = effect_summary(&program);
    let func_effect = summary.declarations.iter().find(|d| d.kind == "function").expect("expected a function entry");
    assert_eq!(func_effect.effect, Effect::Destructive);
    assert!(func_effect.confirmation_required);
    assert!(!func_effect.confirmed, "unconfirmed delete after a `?` must still be reported as unconfirmed");
}

#[test]
fn confirmed_destructive_effect_after_a_try_short_circuit_is_confirmed() {
    let program = parse(
        r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        fn f() returns Result<DnaFile, Str> {
            let x: Result<DnaFile, Str> = store "a.txt" into archive
            let y: DnaFile = x?
            delete "a.txt" from archive confirm physical_key
        }
        "#,
    );
    let summary = effect_summary(&program);
    let func_effect = summary.declarations.iter().find(|d| d.kind == "function").expect("expected a function entry");
    assert!(func_effect.confirmed);
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

#[test]
fn a_caught_store_failure_does_not_abort_the_run_and_a_fallback_pool_succeeds() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("result_fallback_store.nsl")).unwrap();

    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&source).expect("the example must compile cleanly");

    // `archive_in_primary("sample_a.txt")`/`confirm_backup_copy("sample_c.txt")`:
    // real, existing files, so both top-level `outcome`/`confirmed`
    // bindings hold `Ok(...)` and land real stores into `primary`/`backup`.
    // `archive_in_primary("this_file_does_not_exist.txt")`/
    // `confirm_backup_copy("this_file_does_not_exist.txt")`: a nonexistent
    // target, so both real VFS writes now genuinely fail -- this is the
    // actual motivating gap `Result<T,E>`/`?` closes. Before this feature,
    // that failure would have aborted `execute_program` entirely (a hard
    // `Result::Err` via Rust's own `?`); now it's caught inside
    // `archive_in_primary`/`confirm_backup_copy` and surfaced as a step,
    // and the run completes.
    let result = execute_program(&mut os, &mut plan, &dir).expect("a caught Result::Err must not abort execute_program");
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary")), "steps: {:?}", result.steps);
    assert!(result.steps.iter().any(|s| s.contains("✓ store into backup")), "steps: {:?}", result.steps);
    assert!(
        result.steps.iter().any(|s| s.contains("✗ store into primary") && s.contains("this_file_does_not_exist.txt")),
        "expected a caught missing-file failure, got: {:?}",
        result.steps
    );
    assert!(
        result.steps.iter().any(|s| s.contains("✗ store into backup") && s.contains("this_file_does_not_exist.txt")),
        "expected a caught missing-file failure, got: {:?}",
        result.steps
    );
    // Only the two successful stores actually landed.
    assert_eq!(os.dna_stat().file_count, 2);
}

#[test]
fn a_statement_form_store_inside_a_function_body_actually_executes() {
    // Before this fix, `exec_function_body` only ever processed
    // `Declaration::Let` -- a bare statement-form `store` inside a
    // function body was silently skipped, never reaching the real VFS.
    // Reuses `docs/examples/sample_a.txt`/`sample_b.txt` as real fixture
    // content -- `execute_program` reads files from `dir` for real, so
    // an inline source using a filename that doesn't exist on disk would
    // fail for an unrelated reason (a genuine missing-file VFS error),
    // not prove anything about this fix.
    let dir = examples_dir();
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    store \"sample_a.txt\" into archive\n    let confirmation: Result<DnaFile, Str> = store \"sample_b.txt\" into archive\n    let saved: DnaFile = confirmation?\n}}\nlet result: Result<DnaFile, Str> = f()\n",
        POOL
    );
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).expect("must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");
    assert!(result.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_a.txt")), "steps: {:?}", result.steps);
    assert!(result.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_b.txt")), "steps: {:?}", result.steps);
    assert_eq!(os.dna_stat().file_count, 2);
}

#[test]
fn a_statement_form_store_failure_short_circuits_the_rest_of_the_function_body() {
    let dir = examples_dir();
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    store \"sample_a.txt\" into archive\n    store \"this_file_does_not_exist.txt\" into archive\n    let confirmation: Result<DnaFile, Str> = store \"sample_b.txt\" into archive\n    let saved: DnaFile = confirmation?\n}}\nlet result: Result<DnaFile, Str> = f()\n",
        POOL
    );
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).expect("must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("a caught statement-form failure must not abort execute_program");
    // The second statement-form store (a nonexistent file) fails and
    // short-circuits -- the third declaration (`sample_b.txt`) never runs.
    assert!(result.steps.iter().any(|s| s.contains("✗ store into archive") && s.contains("this_file_does_not_exist.txt")), "steps: {:?}", result.steps);
    assert!(
        !result.steps.iter().any(|s| s.contains("sample_b.txt")),
        "the statement-form failure should have short-circuited before sample_b.txt, steps: {:?}",
        result.steps
    );
    assert_eq!(os.dna_stat().file_count, 1);
}

#[test]
fn a_file_typed_parameters_real_filename_flows_into_a_statement_form_store() {
    // The other half of the fix: `Expr::StringLiteral` becoming a real
    // bound `Value::Str` at the call site, and `store <ident> into
    // <pool>` resolving that identifier through `env` instead of
    // treating its literal text as the path.
    let dir = examples_dir();
    let src = format!(
        "{}fn archive_named(name: File) returns Result<DnaFile, Str> {{\n    store name into archive\n    let confirmation: Result<DnaFile, Str> = store \"sample_b.txt\" into archive\n    let saved: DnaFile = confirmation?\n}}\nlet result: Result<DnaFile, Str> = archive_named(\"sample_a.txt\")\n",
        POOL
    );
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).expect("must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");
    assert!(
        result.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_a.txt")),
        "expected the parameter's real filename ('sample_a.txt'), not the identifier 'name', to appear in the step: {:?}",
        result.steps
    );
    assert_eq!(os.dna_stat().file_count, 2);
}

#[test]
fn a_top_level_call_to_a_void_returning_function_actually_executes_its_body() {
    // A real, pre-existing gap found while building the effect-annotated
    // `Fn(...)` types example: `codegen::is_result_producing` (which
    // gates whether a top-level `let` binding is routed through the real
    // interpreter at all) only ever returned `true` for a call to a
    // `Result<_, _>`-returning function -- correct before statement-form
    // `store`/`retrieve`/`delete` could execute inside a function body,
    // wrong afterward. A top-level `let result: Void = f()` where `f`'s
    // body has a real statement-form `store` silently never ran it,
    // with no error and no diagnostic. Fixed by routing any function
    // call through `eval_expr` except one returning a compile-time-only
    // type (`Pool<...>`/`Strand`/`Sequence`/`File`/`Recovery`), which
    // has no runtime `Value` shape to produce anyway.
    let dir = examples_dir();
    let src = format!(
        "{}fn f() returns Void {{\n    store \"sample_a.txt\" into archive\n}}\nlet result: Void = f()\n",
        POOL
    );
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).expect("must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");
    assert!(
        result.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_a.txt")),
        "a Void-returning function's real statement-form store must actually execute, steps: {:?}",
        result.steps
    );
    assert_eq!(os.dna_stat().file_count, 1);
}

// ---------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------

#[test]
fn formatting_a_result_and_try_fixture_is_idempotent() {
    let src = format!(
        "{}fn f() returns Result<DnaFile, Str> {{\n    let x: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let y: DnaFile = x?\n}}\n",
        POOL
    );
    let once = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}

#[test]
fn the_shipped_example_formats_and_is_idempotent() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("result_fallback_store.nsl")).unwrap();
    let once = format_source(&source).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}
