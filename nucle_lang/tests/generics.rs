//! Generics over `Pool<T>`'s profile: `fn name<T>(params) returns T { body }`.
//! Type-check-time-only -- resolved via call-site unification against the
//! argument's real concrete state, with no runtime representation and no
//! per-instantiation re-checking of the function body. See the "Generics"
//! section of docs/grammar.md for the full semantics.

use nucle_lang::ast::*;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, format_source};

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("lex error: {}", e));
    Parser::new(tokens).parse_program().unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn diagnostic_codes(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| d.code).collect()
}

const POOLS: &str = "\
pool illumina_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
pool nanopore_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Nanopore }
";

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

#[test]
fn type_parameter_list_parses_into_function_decl() {
    let program = parse("fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> { let x: Pool<Recovered> = consensus_vote(source, coverage: 10x) }");
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert_eq!(func.type_params, vec!["P".to_string()]);
}

#[test]
fn pool_of_a_declared_type_param_parses_as_pool_state_var() {
    let program = parse("fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> { let x: Pool<Recovered> = consensus_vote(source, coverage: 10x) }");
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert!(matches!(&func.params[0].ty, TypeExpr::Pool(PoolType { state: PoolState::Var(name), .. }) if name == "P"));
}

#[test]
fn non_generic_function_has_an_empty_type_params_list() {
    let program = parse("fn archive_illumina_only(target: Pool<Illumina>) returns Void { let x: Void = protect target for target }");
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    assert!(func.type_params.is_empty());
    assert!(matches!(&func.params[0].ty, TypeExpr::Pool(PoolType { state: PoolState::Profile(Profile::Illumina), .. })));
}

// ---------------------------------------------------------------------
// Typeck
// ---------------------------------------------------------------------

#[test]
fn generic_function_called_with_two_different_profiles_has_no_diagnostics() {
    // The actual motivating pain point: the SAME function, called with
    // Pool<Illumina> and Pool<Nanopore> -- fails to type-check today
    // without generics, since a non-generic Pool<Illumina> parameter
    // couldn't accept a Pool<Nanopore> argument.
    let src = format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered_a: Pool<Recovered> = recover_from(noisy_illumina)

let noisy_nanopore: Pool<Nanopore, 5%> = simulate nanopore_archive under Nanopore
let recovered_b: Pool<Recovered> = recover_from(noisy_nanopore)
"
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn same_type_param_bound_to_two_different_profiles_in_one_call_is_rejected() {
    let src = format!(
        "{POOLS}
fn combine<P>(a: Pool<P, 0.35%>, b: Pool<P, 0.35%>) returns Void {{
    let x: Void = protect a for a
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let noisy_nanopore: Pool<Nanopore, 5%> = simulate nanopore_archive under Nanopore
let y: Void = combine(noisy_illumina, noisy_nanopore)
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-TYPE-PARAM-CONFLICT".to_string()));
}

#[test]
fn same_type_param_bound_consistently_across_two_params_is_accepted() {
    let src = format!(
        "{POOLS}
fn combine<P>(a: Pool<P, 0.35%>, b: Pool<P, 0.35%>) returns Void {{
    let x: Void = protect a for a
}}

let a: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let b: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let y: Void = combine(a, b)
"
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn existing_non_generic_profile_mismatch_still_reports_arg_type_mismatch() {
    // Proves the concrete-vs-concrete path (pre-existing, unrelated to
    // generics) is completely untouched by the new Var-unification branch.
    let src = format!(
        "{POOLS}
fn archive_illumina_only(target: Pool<Illumina>) returns Void {{
    let x: Void = protect target for target
}}

let noisy_nanopore: Pool<Nanopore, 5%> = simulate nanopore_archive under Nanopore
let y: Void = archive_illumina_only(noisy_nanopore)
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-ARG-TYPE-MISMATCH".to_string()));
}

// ---------------------------------------------------------------------
// Explicit type-argument syntax: `name::<Illumina>(...)`
// ---------------------------------------------------------------------

const GENERIC_FN_WITH_UNRESOLVABLE_PARAM: &str = "
fn recover_generically<P>(source: Pool<Illumina, 0.35%>, recover_fn: Fn(Pool<P, 0.35%>) -> Pool<Recovered>) returns Pool<Recovered> {
    let recovered: Pool<Recovered> = recover_fn(source)
}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
";

#[test]
fn explicit_type_argument_parses_into_the_function_calls_own_field() {
    let program = parse(&format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered_a: Pool<Recovered> = recover_from::<Illumina>(noisy_illumina)
"
    ));
    let Declaration::Let(last) = program.declarations.last().unwrap() else { panic!("expected a let") };
    let Expr::FunctionCall { explicit_type_args, .. } = &last.expr else { panic!("expected a function call, got {:?}", last.expr) };
    assert_eq!(explicit_type_args, &vec![Profile::Illumina]);
}

#[test]
fn every_pre_existing_function_call_has_an_empty_explicit_type_args_list() {
    // The backward-compatibility guarantee this field's addition relies
    // on: an ordinary call with no `::<...>` at all parses to an empty
    // list, not `None`/some other sentinel.
    let program = parse("fn f() returns Void {\n    let x: Void = totally_bogus_fn()\n}\n");
    let Declaration::Function(func) = &program.declarations[0] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    let Expr::FunctionCall { explicit_type_args, .. } = &binding.expr else { panic!("expected a function call") };
    assert!(explicit_type_args.is_empty());
}

#[test]
fn explicit_type_argument_resolves_a_type_param_inference_alone_could_not() {
    // `P` appears only inside `recover_fn`'s own `Fn(...)`-typed
    // parameter, never as a directly `Pool<P>`-shaped argument -- the
    // named-function argument-checking loop only unifies a type
    // parameter from a `Pool<...>`-shaped argument, so inference alone
    // has nothing to resolve `P` from at this call site.
    let src = format!(
        "{POOLS}{GENERIC_FN_WITH_UNRESOLVABLE_PARAM}
let recovered: Pool<Recovered> = recover_generically::<Illumina>(noisy_illumina, fn<P>(source: Pool<P, 0.35%>) -> Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}})
"
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn omitting_the_explicit_type_argument_when_inference_cannot_resolve_it_is_rejected() {
    let src = format!(
        "{POOLS}{GENERIC_FN_WITH_UNRESOLVABLE_PARAM}
let recovered: Pool<Recovered> = recover_generically(noisy_illumina, fn<P>(source: Pool<P, 0.35%>) -> Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}})
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-TYPE-PARAM-UNRESOLVED".to_string()));
}

#[test]
fn explicit_type_argument_conflicting_with_an_inferred_one_is_rejected() {
    let src = format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered: Pool<Recovered> = recover_from::<Nanopore>(noisy_illumina)
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-TYPE-PARAM-CONFLICT".to_string()));
}

#[test]
fn wrong_number_of_explicit_type_arguments_is_rejected() {
    let src = format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered: Pool<Recovered> = recover_from::<Illumina, Nanopore>(noisy_illumina)
"
    );
    assert!(diagnostic_codes(&src).contains(&"E-TYPE-PARAM-ARITY".to_string()));
}

// ---------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------

#[test]
fn generic_function_signature_formats_with_no_space_before_angle_brackets() {
    let src = format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}
"
    );
    let formatted = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    assert!(
        formatted.contains("fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {"),
        "got:\n{}",
        formatted
    );
    let twice = format_source(&formatted).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(formatted, twice, "formatting is not idempotent");
}

#[test]
fn explicit_type_argument_formats_with_no_space_before_or_inside_angle_brackets() {
    let src = format!(
        "{POOLS}
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {{
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered: Pool<Recovered> = recover_from::<Illumina>(noisy_illumina)
"
    );
    let formatted = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    assert!(formatted.contains("recover_from::<Illumina>(noisy_illumina)"), "got:\n{}", formatted);
    let twice = format_source(&formatted).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(formatted, twice, "formatting is not idempotent");
}

#[test]
fn comparison_expressions_are_not_mistaken_for_a_generic_open() {
    // The exact regression a naive "any identifier before `<`" fix would
    // introduce: `noisy < 0.1` has an identifier immediately followed by
    // `<`, the same shape as `fn foo<T>` -- only the `fn`-keyword lookback
    // tells them apart.
    let src = "\
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina

if noisy > 0.1 {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
} else {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
}
";
    let formatted = format_source(src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    assert!(formatted.contains("if noisy > 0.1 {"), "comparison lost its spacing, got:\n{}", formatted);
}
