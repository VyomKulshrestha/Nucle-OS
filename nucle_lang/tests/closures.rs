//! Closures / higher-order functions. Covers the full pipeline
//! this feature touches -- parsing `Fn(...)`/anonymous `fn(...) {...}`
//! literals, type-checking (capture, return-type validation, arg
//! checking against a closure's own signature), effect analysis actually
//! seeing into a `let`-bound closure's real body, real end-to-end
//! execution of both a higher-order call and a captured binding, and
//! formatting.

use nucle_lang::ast::*;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, compile_for_simulation, execute_program, format_source, SimulationStep};
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
    assert_eq!(func.params[0].ty, TypeExpr::Fn(vec![TypeExpr::Str], Box::new(TypeExpr::Str), None));
}

#[test]
fn fn_type_with_confirm_hardware_parses_into_the_expected_shape() {
    let program = parse(
        r#"
        fn apply(f: Fn(Str) -> Str confirm hardware) returns Void {
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert_eq!(func.params[0].ty, TypeExpr::Fn(vec![TypeExpr::Str], Box::new(TypeExpr::Str), Some(FnEffectAnnotation::Hardware)));
}

#[test]
fn fn_type_with_confirm_physical_key_parses_into_the_expected_shape() {
    let program = parse(
        r#"
        fn apply(f: Fn(Str) -> Str confirm physical_key) returns Void {
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert_eq!(func.params[0].ty, TypeExpr::Fn(vec![TypeExpr::Str], Box::new(TypeExpr::Str), Some(FnEffectAnnotation::PhysicalKey)));
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
    // E-MATCH-UNRECOGNIZED-SCRUTINEE once the undeclared-variable path
    // degrades).
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
// Generic closures: `fn<T>(...)`
// ---------------------------------------------------------------------

const GENERIC_POOLS: &str = "\
pool illumina_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
";

#[test]
fn generic_closure_literal_parses_its_own_type_params() {
    let program = parse(
        r#"
        fn f<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {
            let recover: Fn(Pool<P, 0.35%>) -> Pool<Recovered> = fn<P>(inner: Pool<P, 0.35%>) -> Pool<Recovered> {
                let recovered: Pool<Recovered> = consensus_vote(inner, coverage: 10x)
            }
            let recovered: Pool<Recovered> = recover(source)
        }
        "#,
    );
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    let Expr::Closure { type_params, .. } = &binding.expr else { panic!("expected a Closure expression, got {:?}", binding.expr) };
    assert_eq!(type_params, &vec!["P".to_string()]);
}

#[test]
fn a_non_generic_closure_literal_has_an_empty_type_params_list() {
    let program = parse("fn f() returns Void {\n    let g: Fn() -> Void = fn() -> Void {\n    }\n}\n");
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    let Expr::Closure { type_params, .. } = &binding.expr else { panic!("expected a Closure expression") };
    assert!(type_params.is_empty());
}

#[test]
fn a_generic_closure_nested_in_a_generic_function_type_checks_and_calls_correctly() {
    let src = format!(
        "{GENERIC_POOLS}
fn recover_via_closure<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recover: Fn(Pool<P, 0.35%>) -> Pool<Recovered> = fn<P>(inner: Pool<P, 0.35%>) -> Pool<Recovered> {{
        let recovered: Pool<Recovered> = consensus_vote(inner, coverage: 10x)
    }}
    let recovered: Pool<Recovered> = recover(source)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered: Pool<Recovered> = recover_via_closure(noisy_illumina)
"
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_generic_closures_type_param_unresolved_from_any_argument_is_rejected() {
    // `T` is declared on the closure but never referenced by any of its
    // own parameter types (`source` is concretely `Pool<Illumina>`), so
    // no argument can ever seed it.
    let src = format!(
        "{GENERIC_POOLS}
let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let g: Fn(Pool<Illumina, 0.35%>) -> Pool<Recovered> = fn<T>(source: Pool<Illumina, 0.35%>) -> Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}
let recovered: Pool<Recovered> = g(noisy_illumina)
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-TYPE-PARAM-UNRESOLVED".to_string()));
}

// ---------------------------------------------------------------------
// Closure self-recursion
// ---------------------------------------------------------------------

// The recursive call retries into a *different* fallback filename
// (`sample_b.txt`), not the identical one that just failed -- a
// self-recursive closure that always retries the exact same failing
// operation with no changing state recurses forever (a real stack
// overflow this was caught by while developing the fix below). Uses
// `sample_a.txt`/`sample_b.txt` (real fixtures under `docs/examples/`,
// since `a_self_recursive_closure_actually_recurses_and_terminates_for_
// real` below executes for real against `examples_dir()`).
const SELF_RECURSIVE_CLOSURE_FN: &str = "\
fn f() returns Result<DnaFile, Str> {
    let attempt_with_fallback: Fn(File) -> Result<DnaFile, Str> = fn(target: File) -> Result<DnaFile, Str> {
        let attempt: Result<DnaFile, Str> = store target into archive
        let saved: DnaFile = match attempt {
            Ok(file) => file,
            Err(reason) => attempt_with_fallback(\"sample_b.txt\")?
        }
    }
    let result: Result<DnaFile, Str> = attempt_with_fallback(\"sample_a.txt\")
}
";

#[test]
fn a_self_recursive_closure_has_no_diagnostics() {
    let src = format!("{}{}", POOL, SELF_RECURSIVE_CLOSURE_FN);
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_self_recursive_closure_actually_recurses_and_terminates_for_real() {
    // A real bug found while writing this test: `call_closure` always
    // started a self-recursive call's own `env` fresh from
    // `captured_env`, which never contains the closure's own name (the
    // snapshot is taken *before* its enclosing `let` finishes binding --
    // see that arm's doc comment) -- so a self-call actually resolved to
    // "internal error: undeclared function", silently swallowed into a
    // `Value::Result(Err(...))` with no VFS step to show for it. Fixed
    // by having `call_closure` re-insert itself under the name it was
    // just called through before running its own body (see its own doc
    // comment) -- this test only passes with that fix in place.
    let dir = examples_dir();
    let src = format!("{}{}\nlet result: Result<DnaFile, Str> = f()\n", POOL, SELF_RECURSIVE_CLOSURE_FN);
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&src).expect("must compile cleanly");

    // First call: `archive` is empty, so the first attempt (`sample_a.txt`)
    // already succeeds -- the self-recursive branch isn't taken yet.
    let first = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");
    assert!(first.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_a.txt")), "steps: {:?}", first.steps);
    assert_eq!(os.dna_stat().file_count, 1);

    // Second call against the same NucleOS: `sample_a.txt` already
    // exists, so the first attempt fails and the closure calls itself
    // by its own bound name with the fallback filename -- a real,
    // distinct second attempt (not the identical one retried forever),
    // which succeeds for real.
    let second = execute_program(&mut os, &mut plan, &dir).expect("a caught, self-recursive Result::Err must not abort execute_program");
    assert!(
        second.steps.iter().any(|s| s.contains("✗ store into archive") && s.contains("sample_a.txt") && s.contains("already exists")),
        "steps: {:?}",
        second.steps
    );
    assert!(
        second.steps.iter().any(|s| s.contains("✓ store into archive") && s.contains("sample_b.txt")),
        "expected the self-recursive call's own fallback attempt to succeed, steps: {:?}",
        second.steps
    );
    assert_eq!(os.dna_stat().file_count, 2);
}

// ---------------------------------------------------------------------
// `nucle plan`/`nucle explain` narration through a `let`-bound closure
// ---------------------------------------------------------------------

#[test]
fn plan_narration_reaches_into_a_let_bound_closures_own_body() {
    let src = format!(
        "{}fn archive_with_logged_fallback() returns Result<DnaFile, Str> {{\n    let primary_attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let fallback: Fn() -> Result<DnaFile, Str> = fn() -> Result<DnaFile, Str> {{\n        let saved: DnaFile = match primary_attempt {{\n            Ok(file) => file,\n            Err(reason) => (store \"b.txt\" into archive)?\n        }}\n    }}\n    let result: Result<DnaFile, Str> = fallback()\n}}\nlet second: Result<DnaFile, Str> = archive_with_logged_fallback()\n",
        POOL
    );
    let plan = compile_for_simulation(&src).expect("must compile cleanly");
    assert!(
        plan.steps.iter().any(|s| matches!(s, SimulationStep::Store { file, .. } if file == "b.txt")),
        "expected the let-bound closure's own fallback store to be narrated, got: {:?}",
        plan.steps
    );
}

#[test]
fn plan_narration_does_not_reach_into_a_fn_typed_parameters_real_closure_body() {
    // The accepted limit this fix doesn't (and can't, without effect-
    // annotated function types) close: a closure received as a
    // `Fn(...)`-typed *parameter* -- here, `retry_once`'s own
    // `attempt_fn` -- is still unnarratable, since its real body isn't
    // known at this call site, only at runtime. Neither `retry_once`'s
    // own internal `attempt_fn()` calls nor the inline closure literal
    // passed as its argument are ever reachable from this walk.
    let src = format!(
        "{}fn retry_once(attempt_fn: Fn() -> Result<DnaFile, Str>) returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = attempt_fn()\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => attempt_fn()?\n    }}\n}}\nfn archive_with_retry() returns Result<DnaFile, Str> {{\n    let result: Result<DnaFile, Str> = retry_once(fn() -> Result<DnaFile, Str> {{\n        let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    }})\n}}\nlet first: Result<DnaFile, Str> = archive_with_retry()\n",
        POOL
    );
    let plan = compile_for_simulation(&src).expect("must compile cleanly");
    assert!(
        !plan.steps.iter().any(|s| matches!(s, SimulationStep::Store { .. })),
        "an inline closure literal passed as a Fn(...)-typed argument should not be narrated, got: {:?}",
        plan.steps
    );
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
// Effect-annotated Fn(...) types -- closing the "accurate effect
// analysis through an arbitrary closure call" gap the tests above
// document as still open. `confirm hardware`/`confirm physical_key` on
// a `Fn(...)` type declares a ceiling; every concrete closure ever bound
// into that slot is checked against it at its own binding site, which is
// what makes trusting the ceiling at the parameter's own call site sound.
// ---------------------------------------------------------------------

#[test]
fn a_properly_confirmed_matching_closure_satisfies_an_annotated_param() {
    let src = format!(
        "{}fn run_with_confirm(op: Fn() -> Void confirm physical_key) returns Void {{
    let x: Void = op()
}}
fn caller() returns Void {{
    let z: Void = run_with_confirm(fn() -> Void {{
        delete \"a.txt\" from archive confirm physical_key
    }})
}}
",
        POOL
    );
    assert!(diagnostic_codes(&src).is_empty(), "diagnostics: {:?}", diagnostic_codes(&src));
}

#[test]
fn a_closure_whose_real_effect_exceeds_the_declared_hardware_ceiling_is_rejected() {
    let src = format!(
        "{}fn run_with_hardware(op: Fn() -> Void confirm hardware) returns Void {{
    let x: Void = op()
}}
fn caller() returns Void {{
    let z: Void = run_with_hardware(fn() -> Void {{
        delete \"a.txt\" from archive confirm physical_key
    }})
}}
",
        POOL
    );
    assert!(
        diagnostic_codes(&src).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()),
        "a Destructive closure bound to a confirm-hardware-only slot must be rejected, got: {:?}",
        diagnostic_codes(&src)
    );
}

#[test]
fn a_closure_with_the_right_effect_but_no_internal_confirmation_is_rejected() {
    let src = format!(
        "{}fn run_with_physical_key(op: Fn() -> Void confirm physical_key) returns Void {{
    let x: Void = op()
}}
fn caller() returns Void {{
    let z: Void = run_with_physical_key(fn() -> Void {{
        delete \"a.txt\" from archive
    }})
}}
",
        POOL
    );
    assert!(
        diagnostic_codes(&src).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()),
        "an unconfirmed Destructive closure must be rejected even though the effect matches, got: {:?}",
        diagnostic_codes(&src)
    );
}

#[test]
fn an_unannotated_fn_typed_param_still_accepts_any_confirmed_closure_unchanged() {
    // Backward compatibility: no annotation means exactly today's
    // behavior -- the closure's own confirmation is real and independent
    // (`E-DELETE-UNCONFIRMED` would fire on its own if it weren't), but
    // this new mechanism adds no additional check for an unannotated
    // parameter.
    let src = format!(
        "{}fn run_without_annotation(op: Fn() -> Void) returns Void {{
    let x: Void = op()
}}
fn caller() returns Void {{
    let z: Void = run_without_annotation(fn() -> Void {{
        delete \"a.txt\" from archive confirm physical_key
    }})
}}
",
        POOL
    );
    assert!(diagnostic_codes(&src).is_empty(), "diagnostics: {:?}", diagnostic_codes(&src));
}

#[test]
fn capturing_an_annotated_outer_parameter_with_a_matching_annotation_is_accepted() {
    // The hole a dedicated architecture-review pass found in this
    // feature's first draft: an inner closure that CAPTURES an
    // annotated outer parameter (rather than receiving one as an
    // explicit call argument) must still resolve soundly.
    let src = format!(
        "{}fn outer(attempt_fn: Fn() -> Void confirm physical_key) returns Void {{
    let g: Fn() -> Void confirm physical_key = fn() -> Void {{
        let y: Void = attempt_fn()
    }}
    let z: Void = g()
}}
",
        POOL
    );
    assert!(diagnostic_codes(&src).is_empty(), "diagnostics: {:?}", diagnostic_codes(&src));
}

#[test]
fn capturing_an_annotated_outer_parameter_with_a_mismatched_annotation_is_rejected() {
    let src = format!(
        "{}fn outer(attempt_fn: Fn() -> Void confirm physical_key) returns Void {{
    let g: Fn() -> Void confirm hardware = fn() -> Void {{
        let y: Void = attempt_fn()
    }}
    let z: Void = g()
}}
",
        POOL
    );
    assert!(
        diagnostic_codes(&src).contains(&"E-FN-EFFECT-ARG-MISMATCH".to_string()),
        "g's own declared confirm hardware ceiling doesn't cover the Destructive effect it actually captures, got: {:?}",
        diagnostic_codes(&src)
    );
}

#[test]
fn forwarding_an_annotated_parameter_to_a_compatibly_annotated_parameter_is_accepted() {
    // A `Fn(...)`-typed parameter passed straight through as another
    // function's own compatibly-annotated parameter argument -- resolved
    // soundly via the declared ceiling alone, no concrete closure or
    // special-casing needed.
    let src = format!(
        "{}fn inner(cb: Fn() -> Void confirm physical_key) returns Void {{
    let x: Void = cb()
}}
fn outer(cb: Fn() -> Void confirm physical_key) returns Void {{
    let y: Void = inner(cb)
}}
",
        POOL
    );
    assert!(diagnostic_codes(&src).is_empty(), "diagnostics: {:?}", diagnostic_codes(&src));
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
    // `fifth` (archive_with_self_retry again): the self-recursive
    // closure's own retry into `primary` fails identically too, adding
    // one more failure on top of the three above.
    let primary_failures = result.steps.iter().filter(|s| s.contains("✗ store into primary")).count();
    assert_eq!(
        primary_failures,
        4,
        "expected 4 failed primary stores (second's own, retry_once's two, and the self-recursive retry's own first attempt), got: {:?}",
        result.steps
    );
    // `fourth` (archive_with_self_retry): the self-recursive closure's
    // own first attempt succeeds once, landing `sample_f.txt`.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary") && s.contains("sample_f.txt")), "steps: {:?}", result.steps);
    // `fifth` (archive_with_self_retry again): the first attempt fails
    // (`sample_f.txt` already exists), so the closure genuinely calls
    // itself with a different fallback filename, which succeeds.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary") && s.contains("sample_f_fallback.txt")), "steps: {:?}", result.steps);
    assert_eq!(os.dna_stat().file_count, 4);
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
