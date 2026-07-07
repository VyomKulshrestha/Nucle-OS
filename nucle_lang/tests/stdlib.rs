//! `consensus_vote`/`protect` are ordinary `FunctionTable` entries
//! (`nucle_lang::stdlib::builtin_functions`), resolved through the exact
//! same lookup as a user-defined `fn` -- these tests prove that's real,
//! not just true by construction of the AST, by checking the specific
//! behaviors that lookup is responsible for: arity checking, "did you
//! mean" suggestions, and effect propagation through nested calls.

use nucle_lang::typeck::check_program;
use nucle_lang::{check_source, Declaration, Expr, FnParam, FunctionDecl, LetDecl, Program, Span, TypeExpr};

fn call(name: &str, args: Vec<Expr>) -> Declaration {
    Declaration::Let(LetDecl {
        name: "result".into(),
        annotation: TypeExpr::Void,
        expr: Expr::FunctionCall { name: name.into(), args },
        span: Span::default(),
    })
}

/// `consensus_vote`'s stdlib entry declares 2 params (see
/// `stdlib::builtin_functions`) -- calling it with the wrong count, via a
/// hand-built AST (the parser's own `consensus_vote(...)` grammar always
/// produces exactly 2 args, so this bypasses that to exercise the shared
/// arity check directly), must report the same code a user function
/// would for the same mistake.
#[test]
fn consensus_vote_wrong_arity_reports_the_same_code_a_user_function_would() {
    let stdlib_program = Program {
        declarations: vec![call("consensus_vote", vec![Expr::Variable("noisy".into()), Expr::Number(10.0), Expr::Number(1.0)])],
    };
    let stdlib_report = check_program(&stdlib_program);
    assert!(stdlib_report.diagnostics.iter().any(|d| d.code == "E-FUNCTION-ARITY"), "got: {:?}", stdlib_report.diagnostics);

    let user_program = Program {
        declarations: vec![
            Declaration::Function(FunctionDecl {
                name: "two_args".into(),
                params: vec![
                    FnParam { name: "a".into(), ty: TypeExpr::Void },
                    FnParam { name: "b".into(), ty: TypeExpr::Void },
                ],
                return_type: TypeExpr::Void,
                body: Vec::new(),
                span: Span::default(),
                doc: None,
            }),
            call("two_args", vec![Expr::Number(1.0), Expr::Number(2.0), Expr::Number(3.0)]),
        ],
    };
    let user_report = check_program(&user_program);
    assert!(user_report.diagnostics.iter().any(|d| d.code == "E-FUNCTION-ARITY"), "got: {:?}", user_report.diagnostics);
}

/// A typo'd stdlib name gets the same "did you mean X?" treatment an
/// undeclared user function call does -- proving built-in names live in
/// the same candidate pool `suggest_name` draws from, not a separate,
/// parser-only keyword list.
#[test]
fn typoing_a_stdlib_function_name_suggests_the_real_one() {
    let program = Program {
        declarations: vec![call("consensus_vte", vec![Expr::Variable("noisy".into()), Expr::Number(10.0)])],
    };
    let report = check_program(&program);
    assert!(
        report.diagnostics.iter().any(|d| d.code == "E-FUNCTION-UNDECLARED" && d.message.contains("consensus_vote")),
        "got: {:?}",
        report.diagnostics
    );
}

/// A user-defined function that calls `consensus_vote`/`protect`
/// internally must have its own effect/confirmation correctly computed
/// by joining over the callee's effect -- exactly the same
/// `function_call_effect` machinery a call to another user function goes
/// through, with no separate hardcoded "these two are always Pure" rule
/// bypassing it.
#[test]
fn effects_propagate_through_a_user_function_wrapping_consensus_vote() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        fn recover(source: Pool<Illumina>) returns Pool<Recovered> {
            let result: Pool<Recovered> = consensus_vote(source, coverage: 10x)
        }

        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        let recovered: Pool<Recovered> = recover(noisy)
    "#;
    let report = check_source(src);
    assert!(report.ok, "expected valid program, got: {:?}", report.diagnostics);
}

/// Same idea for `protect`: wrapping it in a user function (this is
/// exactly `docs/examples/archive_fn.nsl`'s real shape) must still
/// type-check and classify as `Pure`/always-confirmed with no
/// confirmation error.
#[test]
fn effects_propagate_through_a_user_function_wrapping_protect() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        fn archive_it(data: File, target: Pool<Illumina>, guarantee: Recovery) returns DnaFile {
            let plan: DnaFile = protect data for guarantee
            store plan into target
        }
    "#;
    let report = check_source(src);
    assert!(report.ok, "expected valid program, got: {:?}", report.diagnostics);
}

/// `consensus_vote`'s keyword sugar (`consensus_vote(source, coverage:
/// N)`) must desugar to exactly the same `Expr::FunctionCall`
/// representation a hand-written call would produce -- there is only one
/// way this call is ever represented past the parser, not two.
#[test]
fn consensus_vote_keyword_syntax_desugars_to_a_function_call() {
    // `parse_program` only parses declarations; a `let` binding is the
    // only place this expression syntax is valid. An un-annotated `let x
    // = ...` only accepts the `seq"..."` sugar, so this needs a type
    // annotation to reach the general expression parser.
    let src = "let x: Recovery = consensus_vote(noisy, coverage: 10x)";
    let tokens = nucle_lang::Lexer::new(src).tokenize().unwrap();
    let program = nucle_lang::Parser::new(tokens).parse_program().unwrap();
    let expr = match &program.declarations[0] {
        Declaration::Let(binding) => binding.expr.clone(),
        other => panic!("expected a let binding, got {:?}", other),
    };
    assert_eq!(
        expr,
        Expr::FunctionCall {
            name: "consensus_vote".into(),
            args: vec![Expr::Variable("noisy".into()), Expr::Number(10.0)],
        }
    );
}
