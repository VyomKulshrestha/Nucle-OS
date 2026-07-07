//! `nucle test`: executes every `test "description" { ... }` block in a
//! program (see `ast::TestDecl`) and reports pass/fail per test.
//!
//! Two independent things can fail a test, both surfaced the same way
//! `nucle check`/`nucle run` already would:
//! - An `assert` inside it evaluates to `false` at type-check time (see
//!   `typeck::TypeChecker::check_assert`) -- reported as an
//!   `E-ASSERTION-FAILED` diagnostic at the assertion's own span.
//! - A real `store`/`retrieve`/`delete` operation inside it fails at
//!   runtime against a fresh, per-test `NucleOS` instance -- the exact
//!   same execution path `nucle run` uses, so a test can catch a genuine
//!   VFS failure, not just a failed assertion.
//!
//! A test whose body has a *real* compile error (anything other than a
//! failed assertion -- an undeclared pool, a type mismatch, ...) is
//! reported as a compile error and aborts the whole run before any test
//! executes, matching how a syntax/type error in a `#[test]` fn aborts
//! `cargo test` before any test runs, rather than being folded into that
//! one test's pass/fail result.

use crate::ast::{Declaration, Program, Span, TestDecl};
use crate::codegen::compile_program;
use crate::typeck::{check_and_desugar, Diagnostic, DiagnosticLevel, TypeReport};
use nucle_vfs::syscall::NucleOS;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct TestFailure {
    pub message: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub span: Span,
    pub passed: bool,
    pub failures: Vec<TestFailure>,
}

#[derive(Debug, Clone, Default)]
pub struct TestRunReport {
    /// Non-empty here means the file didn't compile at all -- no test was
    /// run, and `results` is empty.
    pub compile_errors: Vec<Diagnostic>,
    pub results: Vec<TestResult>,
}

impl TestRunReport {
    pub fn all_passed(&self) -> bool {
        self.compile_errors.is_empty() && self.results.iter().all(|r| r.passed)
    }
}

/// Runs every `test { ... }` block in `program`. `base_dir` resolves
/// relative file paths a test's `store` operations reference, exactly
/// like `codegen::execute_program`'s own `base_dir` parameter for
/// `nucle run`.
pub fn run_tests(program: &Program, base_dir: &Path) -> TestRunReport {
    let (report, desugared) = check_and_desugar(program);

    // Anything other than a failed assertion is a real compile error --
    // abort before running anything, the same way a type error anywhere
    // in a Rust test file stops `cargo test` from running any test at all.
    let compile_errors: Vec<Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Error && d.code != "E-ASSERTION-FAILED")
        .cloned()
        .collect();
    if !compile_errors.is_empty() {
        return TestRunReport { compile_errors, results: Vec::new() };
    }

    // Every non-test top-level declaration (pools, lets, functions, ...)
    // is in scope for every test, the same way it would be for any other
    // declaration in the file -- each test still gets built into its own
    // "virtual program" below so its `store`/`retrieve`/`delete`
    // operations run against a fresh `NucleOS`, isolated from every other
    // test.
    let outer_context: Vec<Declaration> =
        desugared.declarations.iter().filter(|d| !matches!(d, Declaration::Test(_))).cloned().collect();

    let results = desugared
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Test(test) => Some(run_one_test(test, &outer_context, &report, base_dir)),
            _ => None,
        })
        .collect();

    TestRunReport { compile_errors: Vec::new(), results }
}

fn run_one_test(test: &TestDecl, outer_context: &[Declaration], report: &TypeReport, base_dir: &Path) -> TestResult {
    let mut failures: Vec<TestFailure> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "E-ASSERTION-FAILED" && d.span.line >= test.span.line && d.span.line <= test.span.end_line)
        .map(|d| TestFailure { message: d.message.clone(), span: d.span })
        .collect();

    let mut virtual_program = Program { declarations: outer_context.to_vec() };
    virtual_program.declarations.extend(test.body.clone());
    let mut plan = compile_program(virtual_program, TypeReport::default());
    let mut os = NucleOS::new(100);
    if let Err(err) = crate::codegen::execute_program(&mut os, &mut plan, base_dir) {
        failures.push(TestFailure { message: err, span: test.span });
    }

    TestResult { name: test.name.clone(), span: test.span, passed: failures.is_empty(), failures }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    /// `line` must fall inside the enclosing `TestDecl`'s span range for
    /// `run_one_test`'s line-range attribution to find it -- a real parsed
    /// program always satisfies this since an assert's span is a real
    /// source position between its enclosing `test { ... }`'s braces;
    /// these hand-built fixtures have to fake that positioning explicitly.
    fn assert_decl(condition: Expr, message: Option<&str>, line: usize) -> Declaration {
        Declaration::Operation(Operation::Assert(AssertOp {
            condition,
            message: message.map(String::from),
            span: Span::point(line, 1),
        }))
    }

    #[test]
    fn a_passing_assertion_reports_the_test_as_passed() {
        let program = Program {
            declarations: vec![Declaration::Test(TestDecl {
                name: "trivially true".into(),
                body: vec![assert_decl(Expr::BinaryOp { op: BinOp::Eq, left: Box::new(Expr::Number(1.0)), right: Box::new(Expr::Number(1.0)) }, None, 2)],
                span: Span { line: 1, column: 1, end_line: 3, end_column: 1 },
            })],
        };
        let report = run_tests(&program, Path::new("."));
        assert!(report.compile_errors.is_empty());
        assert_eq!(report.results.len(), 1);
        assert!(report.results[0].passed, "expected pass, got: {:?}", report.results[0].failures);
    }

    #[test]
    fn a_failing_assertion_reports_the_test_as_failed_with_its_message() {
        let program = Program {
            declarations: vec![Declaration::Test(TestDecl {
                name: "trivially false".into(),
                body: vec![assert_decl(
                    Expr::BinaryOp { op: BinOp::Eq, left: Box::new(Expr::Number(1.0)), right: Box::new(Expr::Number(2.0)) },
                    Some("one should equal two, apparently"),
                    2,
                )],
                span: Span { line: 1, column: 1, end_line: 3, end_column: 1 },
            })],
        };
        let report = run_tests(&program, Path::new("."));
        assert!(!report.results[0].passed);
        assert_eq!(report.results[0].failures.len(), 1);
        assert_eq!(report.results[0].failures[0].message, "one should equal two, apparently");
    }

    #[test]
    fn a_compile_error_aborts_the_whole_run_instead_of_failing_one_test() {
        let program = Program {
            declarations: vec![Declaration::Test(TestDecl {
                name: "references an undeclared pool".into(),
                body: vec![Declaration::Operation(Operation::Retrieve(RetrieveOp {
                    pool: "does_not_exist".into(),
                    query: Vec::new(),
                    span: Span::default(),
                }))],
                span: Span { line: 1, column: 1, end_line: 3, end_column: 1 },
            })],
        };
        let report = run_tests(&program, Path::new("."));
        assert!(!report.compile_errors.is_empty());
        assert!(report.results.is_empty(), "no test should run when the program doesn't compile");
        assert!(report.compile_errors.iter().any(|d| d.code == "E-RETRIEVE-POOL-UNDECLARED"));
    }

    #[test]
    fn multiple_tests_are_isolated_from_each_other() {
        let program = Program {
            declarations: vec![
                Declaration::Test(TestDecl {
                    name: "first".into(),
                    body: vec![assert_decl(Expr::BinaryOp { op: BinOp::Eq, left: Box::new(Expr::Number(1.0)), right: Box::new(Expr::Number(1.0)) }, None, 2)],
                    span: Span { line: 1, column: 1, end_line: 3, end_column: 1 },
                }),
                Declaration::Test(TestDecl {
                    name: "second".into(),
                    body: vec![assert_decl(Expr::BinaryOp { op: BinOp::Eq, left: Box::new(Expr::Number(1.0)), right: Box::new(Expr::Number(2.0)) }, None, 6)],
                    span: Span { line: 5, column: 1, end_line: 7, end_column: 1 },
                }),
            ],
        };
        let report = run_tests(&program, Path::new("."));
        assert_eq!(report.results.len(), 2);
        assert!(report.results[0].passed, "'first' should pass");
        assert!(!report.results[1].passed, "'second' should fail");
    }
}
