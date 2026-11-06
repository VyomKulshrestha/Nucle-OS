//! NucleScript 1.0 held to its promise (docs/stability.md).
//!
//! `tests/conformance/1.0` holds the shipped example programs as they were
//! when 1.0 was declared. Every 1.x release must check each one to the
//! same result: accepted or rejected, the same diagnostic codes, the same
//! effect for each top-level declaration. `expected.txt` records those
//! results. Rewriting it (`NSL_BLESS=1`) is for a change docs/stability.md
//! allows in 1.x, such as a new warning, and the change goes in its log.
//!
//! The grammar's EBNF in docs/grammar.md is pinned by digest too, so a
//! change to the syntax cannot land without someone updating the digest
//! and the log.

use nucle_lang::effects::effect_summary;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, DiagnosticLevel, LANGUAGE_VERSION};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// SHA-256 of the EBNF block in docs/grammar.md, line endings normalized.
const GRAMMAR_1_0_DIGEST: &str = "bf6bf6034e3398c78ab8b80ad38d2014e4cc28af41b77912889206ac901abc19";

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("conformance").join("1.0")
}

/// What the checker makes of one program, as lines of text.
fn outcome(name: &str, source: &str) -> String {
    let report = check_source(source);
    let accepted = !report.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error);
    let mut lines = vec![format!("=== {name}"), if accepted { "accepted" } else { "rejected" }.to_string()];
    let mut codes: Vec<String> = report.diagnostics.iter().map(|d| format!("{:?} {}", d.level, d.code)).collect();
    codes.sort();
    lines.extend(codes);
    if let Ok(Ok(program)) = Lexer::new(source).tokenize().map(|tokens| Parser::new(tokens).parse_program()) {
        lines.extend(effect_summary(&program).declarations.iter().map(|d| format!("{} {}: {}", d.kind, d.name, d.effect)));
    }
    lines.join("\n") + "\n"
}

#[test]
fn every_1_0_program_checks_as_it_did() {
    let dir = corpus();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "nsl")).collect();
    files.sort();
    assert!(files.len() >= 30, "the 1.0 corpus has only {} programs", files.len());
    let actual: String = files
        .iter()
        .map(|path| outcome(&path.file_name().unwrap().to_string_lossy(), &std::fs::read_to_string(path).unwrap()))
        .collect::<Vec<_>>()
        .join("\n");

    let expected_path = dir.join("expected.txt");
    if std::env::var("NSL_BLESS").is_ok() {
        std::fs::write(&expected_path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap().replace("\r\n", "\n");
    if actual != expected {
        let (a, e): (Vec<&str>, Vec<&str>) = (actual.lines().collect(), expected.lines().collect());
        let first = a.iter().zip(&e).position(|(x, y)| x != y).unwrap_or(a.len().min(e.len()));
        panic!(
            "a NucleScript {LANGUAGE_VERSION} program no longer checks as it did; first difference at line {}:\n  expected: {:?}\n  actual:   {:?}\n\
             1.x may add warnings but must not reject, or change the effects of, what 1.0 accepted (docs/stability.md). \
             If the change is one 1.x allows, log it there and rerun with NSL_BLESS=1.",
            first + 1,
            e.get(first),
            a.get(first)
        );
    }
}

#[test]
fn the_grammar_is_the_one_1_0_was_declared_with() {
    let grammar = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("grammar.md")).unwrap().replace("\r\n", "\n");
    let start = grammar.find("```ebnf\n").expect("docs/grammar.md has an ebnf block") + "```ebnf\n".len();
    let end = start + grammar[start..].find("\n```").expect("the ebnf block is closed");
    let digest: String = Sha256::digest(&grammar.as_bytes()[start..end]).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        digest, GRAMMAR_1_0_DIGEST,
        "docs/grammar.md's EBNF changed. 1.x may only add to the grammar: record the addition in docs/stability.md and update GRAMMAR_1_0_DIGEST"
    );
}

#[test]
fn the_language_version_is_1_0() {
    assert_eq!(LANGUAGE_VERSION, "1.0");
}
