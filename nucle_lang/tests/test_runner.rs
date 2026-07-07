//! Integration tests for `nucle test`, exercised through real source text
//! (lex -> parse -> `run_tests`) rather than hand-built ASTs -- the unit
//! tests in `nucle_lang/src/test_runner.rs` already cover the runner's
//! internal logic directly; these prove the whole pipeline works end to
//! end the way a `.nsl` file author would actually trigger it.

use nucle_lang::{run_tests, Lexer, Parser};
use std::path::Path;

fn parse(src: &str) -> nucle_lang::Program {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse_program().unwrap()
}

#[test]
fn a_passing_test_reports_pass_with_no_failures() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        test "consensus voting reduces the error rate" {
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
            assert recovered < noisy
        }
    "#;
    let report = run_tests(&parse(src), Path::new("."));
    assert!(report.compile_errors.is_empty(), "got: {:?}", report.compile_errors);
    assert_eq!(report.results.len(), 1);
    assert!(report.results[0].passed, "expected pass, got: {:?}", report.results[0].failures);
    assert!(report.all_passed());
}

#[test]
fn a_failing_assertion_is_reported_with_its_message_and_the_assert_statements_own_span() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        test "this assertion is deliberately wrong" {
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            assert noisy > 100.0, "error rate should realistically never exceed 100%"
        }
    "#;
    let report = run_tests(&parse(src), Path::new("."));
    assert!(report.compile_errors.is_empty(), "a failed assertion is not a compile error");
    assert_eq!(report.results.len(), 1);
    let result = &report.results[0];
    assert!(!result.passed);
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].message, "error rate should realistically never exceed 100%");
    // The assert statement is on line 6 of `src` (1-indexed, counting the
    // leading blank line from the raw string) -- pinning this down proves
    // the failure's span is the assertion's own, not the enclosing test
    // block's.
    assert_eq!(result.failures[0].span.line, 6);
    assert!(!report.all_passed());
}

#[test]
fn a_compile_error_inside_a_test_body_is_reported_as_a_compile_error_not_a_test_failure() {
    let src = r#"
        test "references a pool that was never declared" {
            retrieve from nonexistent_pool
        }
    "#;
    let report = run_tests(&parse(src), Path::new("."));
    assert!(report.results.is_empty(), "no test should run when the program doesn't compile");
    assert!(!report.compile_errors.is_empty());
    assert!(report.compile_errors.iter().any(|d| d.code == "E-RETRIEVE-POOL-UNDECLARED"));
    assert!(!report.all_passed());
}

#[test]
fn multiple_tests_in_one_file_are_reported_independently() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        test "first: consensus reduces error rate" {
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
            assert recovered < noisy
        }

        test "second: deliberately false" {
            assert 1.0 == 2.0, "one is not two"
        }
    "#;
    let report = run_tests(&parse(src), Path::new("."));
    assert!(report.compile_errors.is_empty());
    assert_eq!(report.results.len(), 2);
    assert!(report.results[0].passed, "first test should pass");
    assert!(!report.results[1].passed, "second test should fail");
    assert_eq!(report.results[1].failures[0].message, "one is not two");
}
