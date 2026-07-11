//! Step 14: user-defined enums and the general pattern-matching/
//! exhaustiveness engine. Covers the full pipeline this feature touches
//! -- parsing `enum`/`EnumName::Variant`/general N-arm `match`, type-
//! checking (registration, construction validity, exhaustiveness/
//! wildcard/duplicate-arm checks), conservative effect-joining across N
//! arms, real end-to-end execution, formatting, and doc generation --
//! plus the zero-regression proof that `Result<T, E>`/`Ok`/`Err`'s
//! existing behavior is unaffected by being unified into this same
//! general engine as a built-in 2-variant pseudo-enum.

use nucle_lang::ast::*;
use nucle_lang::effects::effect_summary;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, execute_program, format_source, generate_docs};
use std::path::Path;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("lex error: {}", e));
    Parser::new(tokens).parse_program().unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn diagnostic_codes(src: &str) -> Vec<String> {
    check_source(src).diagnostics.into_iter().map(|d| d.code).collect()
}

const POOL: &str = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n";

const RECOVERY_PLAN: &str = "\
enum RecoveryPlan {
    Retry,
    Fallback,
    GiveUp(Str),
}
";

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

#[test]
fn enum_decl_parses_unit_and_payload_variants() {
    let program = parse(RECOVERY_PLAN);
    let Declaration::Enum(decl) = &program.declarations[0] else { panic!("expected an enum") };
    assert_eq!(decl.name, "RecoveryPlan");
    assert_eq!(decl.variants.len(), 3);
    assert_eq!(decl.variants[0], EnumVariant { name: "Retry".to_string(), payload: None, span: decl.variants[0].span });
    assert_eq!(decl.variants[1], EnumVariant { name: "Fallback".to_string(), payload: None, span: decl.variants[1].span });
    assert_eq!(decl.variants[2].name, "GiveUp");
    assert_eq!(decl.variants[2].payload, Some(TypeExpr::Str));
}

#[test]
fn enum_construct_parses_unit_and_payload_forms() {
    let program = parse(&format!("{RECOVERY_PLAN}let a: RecoveryPlan = RecoveryPlan::Retry\nlet b: RecoveryPlan = RecoveryPlan::GiveUp(\"nope\")\n"));
    let Declaration::Let(a) = &program.declarations[1] else { panic!("expected a let") };
    assert_eq!(a.expr, Expr::EnumConstruct { enum_name: "RecoveryPlan".to_string(), variant: "Retry".to_string(), payload: None });
    let Declaration::Let(b) = &program.declarations[2] else { panic!("expected a let") };
    assert_eq!(
        b.expr,
        Expr::EnumConstruct {
            enum_name: "RecoveryPlan".to_string(),
            variant: "GiveUp".to_string(),
            payload: Some(Box::new(Expr::StringLiteral("nope".to_string())))
        }
    );
}

#[test]
fn turbofish_still_parses_unambiguously_alongside_enum_construction() {
    // `::` followed by `<` is turbofish (Step 13); `::` followed by an
    // identifier is enum variant construction (Step 14) -- one token of
    // lookahead disambiguates them with no ambiguity.
    let program = parse(
        "\
pool illumina_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
enum RecoveryPlan { Retry }
fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}
let noisy: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered: Pool<Recovered> = recover_from::<Illumina>(noisy)
let plan: RecoveryPlan = RecoveryPlan::Retry
",
    );
    let Declaration::Let(recovered) = &program.declarations[4] else { panic!("expected a let") };
    let Expr::FunctionCall { explicit_type_args, .. } = &recovered.expr else { panic!("expected a function call") };
    assert_eq!(explicit_type_args, &vec![Profile::Illumina]);
    let Declaration::Let(plan) = &program.declarations[5] else { panic!("expected a let") };
    assert!(matches!(&plan.expr, Expr::EnumConstruct { variant, .. } if variant == "Retry"));
}

#[test]
fn general_match_arm_parses_wildcard_and_payload_binding() {
    let program = parse(&format!(
        "{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        Retry => \"retry\",\n        _ => \"other\"\n    }}\n}}\n"
    ));
    let Declaration::Function(func) = &program.declarations[1] else { panic!("expected a function") };
    let Declaration::Let(binding) = &func.body[0] else { panic!("expected a let") };
    let Expr::Match { arms, .. } = &binding.expr else { panic!("expected a match") };
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].variant, Some("Retry".to_string()));
    assert_eq!(arms[0].binding, None);
    assert_eq!(arms[1].variant, None);
}

// ---------------------------------------------------------------------
// Typeck: enum declaration
// ---------------------------------------------------------------------

#[test]
fn well_formed_enum_has_no_diagnostics() {
    let report = check_source(RECOVERY_PLAN);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn duplicate_enum_name_is_rejected() {
    let src = format!("{RECOVERY_PLAN}enum RecoveryPlan {{ Other }}\n");
    assert!(diagnostic_codes(&src).contains(&"E-ENUM-DUPLICATE".to_string()));
}

#[test]
fn redeclaring_result_as_an_enum_is_rejected() {
    let src = "enum Result { Ok, Err }\n";
    assert!(diagnostic_codes(src).contains(&"E-ENUM-RESERVED-NAME".to_string()));
}

#[test]
fn an_empty_enum_is_rejected() {
    let src = "enum Empty { }\n";
    assert!(diagnostic_codes(src).contains(&"E-ENUM-EMPTY".to_string()));
}

#[test]
fn duplicate_variant_name_within_one_enum_is_rejected() {
    let src = "enum Status { Active, Active }\n";
    assert!(diagnostic_codes(src).contains(&"E-ENUM-VARIANT-DUPLICATE".to_string()));
}

// ---------------------------------------------------------------------
// Typeck: enum construction
// ---------------------------------------------------------------------

#[test]
fn constructing_an_unknown_enum_is_rejected() {
    let src = "let x: Status = Status::Active\n";
    assert!(diagnostic_codes(src).contains(&"E-ENUM-CONSTRUCT-UNKNOWN-ENUM".to_string()));
}

#[test]
fn constructing_an_unknown_variant_is_rejected() {
    let src = format!("{RECOVERY_PLAN}let x: RecoveryPlan = RecoveryPlan::Bogus\n");
    assert!(diagnostic_codes(&src).contains(&"E-ENUM-CONSTRUCT-UNKNOWN-VARIANT".to_string()));
}

#[test]
fn constructing_a_unit_variant_with_a_payload_is_rejected() {
    let src = format!("{RECOVERY_PLAN}let x: RecoveryPlan = RecoveryPlan::Retry(\"nope\")\n");
    assert!(diagnostic_codes(&src).contains(&"E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH".to_string()));
}

#[test]
fn constructing_a_payload_variant_with_no_payload_is_rejected() {
    let src = format!("{RECOVERY_PLAN}let x: RecoveryPlan = RecoveryPlan::GiveUp\n");
    assert!(diagnostic_codes(&src).contains(&"E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH".to_string()));
}

#[test]
fn constructing_a_payload_variant_with_the_wrong_payload_type_is_rejected() {
    // `noisy` is a bound `Pool<Illumina, 0.35%>` variable, not `Str` --
    // `GiveUp`'s declared payload type.
    let src = format!(
        "{}{RECOVERY_PLAN}let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina\nlet x: RecoveryPlan = RecoveryPlan::GiveUp(noisy)\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH".to_string()));
}

// ---------------------------------------------------------------------
// Typeck: general match over a user enum
// ---------------------------------------------------------------------

#[test]
fn exhaustive_match_naming_every_variant_has_no_diagnostics() {
    // Unit variants (`Retry`/`Fallback`) have no payload to bind, and a
    // bare string literal isn't a supported arm-body shape (consistent
    // with `Ok("literal")` being rejected too -- see `E-OK-CONSTRUCTOR-
    // INVALID` -- neither has ever supported bare literals), so each arm
    // produces a real `Result`-shaped value via `?` instead, the same
    // idiom `docs/examples/recovery_plan.nsl` itself uses.
    let src = format!(
        "{}{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Result<DnaFile, Str> {{\n    let saved: DnaFile = match plan {{\n        Retry => (store \"a.txt\" into archive)?,\n        Fallback => (store \"b.txt\" into archive)?,\n        GiveUp(reason) => (store \"c.txt\" into archive)?\n    }}\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn a_trailing_wildcard_covers_the_rest_with_no_diagnostics() {
    let src = format!(
        "{}{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Result<DnaFile, Str> {{\n    let saved: DnaFile = match plan {{\n        Retry => (store \"a.txt\" into archive)?,\n        _ => (store \"b.txt\" into archive)?\n    }}\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

#[test]
fn matching_a_non_enum_non_result_expression_is_rejected() {
    let src = format!("{}fn f() returns Void {{\n    let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina\n    let x: Void = match noisy {{\n        _ => noisy\n    }}\n}}\n", POOL);
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-UNRECOGNIZED-SCRUTINEE".to_string()));
}

#[test]
fn an_arm_naming_an_unknown_variant_is_rejected() {
    let src = format!("{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        Retry => \"retry\",\n        Bogus => \"other\"\n    }}\n}}\n");
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-UNKNOWN-VARIANT".to_string()));
}

#[test]
fn a_non_exhaustive_match_with_no_wildcard_is_rejected() {
    let src = format!("{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        Retry => \"retry\"\n    }}\n}}\n");
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-NON-EXHAUSTIVE".to_string()));
}

#[test]
fn two_arms_naming_the_same_variant_is_rejected() {
    let src = format!(
        "{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        Retry => \"retry\",\n        Retry => \"retry again\",\n        Fallback => \"fallback\",\n        GiveUp(reason) => reason\n    }}\n}}\n"
    );
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-DUPLICATE-ARM".to_string()));
}

#[test]
fn an_arm_after_a_wildcard_is_rejected() {
    let src = format!("{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        _ => \"other\",\n        Retry => \"retry\"\n    }}\n}}\n");
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-ARM-AFTER-WILDCARD".to_string()));
}

#[test]
fn mismatched_arm_types_across_n_arms_is_rejected() {
    let src = format!(
        "{}{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Void {{\n    let x: Void = match plan {{\n        Retry => store \"a.txt\" into archive,\n        Fallback => store \"a.txt\" into archive,\n        GiveUp(reason) => reason\n    }}\n}}\n",
        POOL
    );
    assert!(diagnostic_codes(&src).contains(&"E-MATCH-ARM-TYPE-MISMATCH".to_string()));
}

#[test]
fn a_user_enum_variant_literally_named_ok_does_not_collide_with_real_result() {
    // Proves the Result unification is real, not a name-based hack: a
    // user enum with its own unit variant named "Ok" alongside a genuine
    // Result match in a different function must not cross-contaminate --
    // each match's scrutinee kind is resolved independently.
    let src = format!(
        "{}enum Confirmation {{ Ok(Str), Denied(Str) }}\nfn describe(c: Confirmation) returns Str {{\n    let label: Str = match c {{\n        Ok(reason) => reason,\n        Denied(reason) => reason\n    }}\n}}\nfn f() returns Result<DnaFile, Str> {{\n    let attempt: Result<DnaFile, Str> = store \"a.txt\" into archive\n    let saved: DnaFile = match attempt {{\n        Ok(file) => file,\n        Err(reason) => (store \"b.txt\" into archive)?\n    }}\n}}\n",
        POOL
    );
    let report = check_source(&src);
    assert!(report.ok, "expected no diagnostics, got: {:?}", report.diagnostics);
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

#[test]
fn destructive_effect_in_a_non_first_arm_still_requires_confirmation() {
    // Generalizes the Result-only "join across every arm unconditionally"
    // rule (Step 11) to N arms/variants: a `Destructive` operation in the
    // *third* arm still counts, since this analysis never models "this
    // branch might not run".
    let program = parse(&format!(
        "{}enum Status {{ Active, Suspended, Terminated }}\nfn f(s: Status) returns Void {{\n    let x: Void = match s {{\n        Active => delete \"a.txt\" from archive confirm physical_key,\n        Suspended => delete \"b.txt\" from archive confirm physical_key,\n        Terminated => delete \"c.txt\" from archive\n    }}\n}}\n",
        POOL
    ));
    let summary = effect_summary(&program);
    let func_effect = summary.declarations.iter().find(|d| d.kind == "function").expect("expected a function entry");
    assert_eq!(func_effect.effect, Effect::Destructive);
    assert!(func_effect.confirmation_required);
    assert!(!func_effect.confirmed, "the unconfirmed delete in the third arm must still be reported as unconfirmed");
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

#[test]
fn every_variant_arm_actually_runs_against_a_real_vfs() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("recovery_plan.nsl")).unwrap();

    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    let mut plan = compile(&source).expect("the example must compile cleanly");
    let result = execute_program(&mut os, &mut plan, &dir).expect("execution must not abort");

    // `first`: `primary` is empty, so the outer `Ok` arm runs directly.
    assert!(result.steps.iter().any(|s| s.contains("✓ store into primary") && s.contains("sample_a.txt")), "steps: {:?}", result.steps);
    // `second`: the outer attempt fails (duplicate filename), so the
    // nested match over `plan` runs for real -- `Fallback` is caught by
    // the trailing wildcard, landing a real store into `secondary`.
    assert!(
        result.steps.iter().any(|s| s.contains("✗ store into primary") && s.contains("already exists")),
        "steps: {:?}",
        result.steps
    );
    assert!(result.steps.iter().any(|s| s.contains("✓ store into secondary") && s.contains("sample_b.txt")), "steps: {:?}", result.steps);
    assert_eq!(os.dna_stat().file_count, 2);
}

// ---------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------

#[test]
fn formatting_an_enum_and_general_match_fixture_is_idempotent() {
    let src = format!(
        "{RECOVERY_PLAN}fn f(plan: RecoveryPlan) returns Str {{\n    let label: Str = match plan {{\n        Retry => \"retry\",\n        Fallback => \"fallback\",\n        GiveUp(reason) => reason\n    }}\n}}\n"
    );
    let once = format_source(&src).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}

#[test]
fn the_shipped_example_formats_and_is_idempotent() {
    let dir = examples_dir();
    let source = std::fs::read_to_string(dir.join("recovery_plan.nsl")).unwrap();
    let once = format_source(&source).unwrap_or_else(|e| panic!("failed to format: {}", e));
    let twice = format_source(&once).unwrap_or_else(|e| panic!("failed to re-format its own output: {}", e));
    assert_eq!(once, twice, "formatting is not idempotent");
}

// ---------------------------------------------------------------------
// Docgen
// ---------------------------------------------------------------------

#[test]
fn an_enum_declaration_produces_the_expected_markdown_section() {
    let program = parse(&format!("/// A recovery strategy.\n{RECOVERY_PLAN}"));
    let markdown = generate_docs(&program);
    assert!(markdown.contains("## Enums"), "got:\n{}", markdown);
    assert!(markdown.contains("### `RecoveryPlan`"), "got:\n{}", markdown);
    assert!(markdown.contains("A recovery strategy."), "got:\n{}", markdown);
    assert!(markdown.contains("enum RecoveryPlan { Retry, Fallback, GiveUp(Str) }"), "got:\n{}", markdown);
}
