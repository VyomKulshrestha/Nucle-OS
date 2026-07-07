use nucle_lang::{check_source, Diagnostic, DiagnosticLevel};

/// Slice out the 1-based source line a diagnostic's span points at, so
/// tests can assert the span lands on the actual offending line instead
/// of just trusting that *some* span (possibly `Span::default()`, all
/// zeros -- meaning "no real position") is attached.
fn line_at<'a>(source: &'a str, diagnostic: &Diagnostic) -> &'a str {
    source.lines().nth(diagnostic.span.line.saturating_sub(1)).unwrap_or("")
}

#[test]
fn test_check_valid_program() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        store "README.md" into archive { redundancy: 4x, tag: ["medical", "critical"] }
    "#;
    let report = check_source(src);
    assert!(report.ok, "Expected valid program to pass check, got diagnostics: {:?}", report.diagnostics);
    let has_errors = report.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error);
    assert!(!has_errors, "Expected no error diagnostics for a valid program, got: {:?}", report.diagnostics);
}

#[test]
fn test_check_lex_error() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        store "README.md into archive
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected program with lex error to fail check");
    assert!(!report.diagnostics.is_empty(), "Expected diagnostics for lex error");
    let diagnostic = report.diagnostics.iter().find(|d| {
        d.level == DiagnosticLevel::Error && d.message.contains("lex error")
    });
    let diagnostic = diagnostic.expect("Expected diagnostics to contain 'lex error'");
    assert_ne!(diagnostic.span.line, 0, "lex error diagnostic has no real source span");
    assert!(
        line_at(src, diagnostic).contains("README.md"),
        "lex error span should point at the unterminated string's line, got line {}: {:?}",
        diagnostic.span.line, line_at(src, diagnostic)
    );
}

#[test]
fn test_check_parse_error() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        store "README.md" archive
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected program with parse error to fail check");
    assert!(!report.diagnostics.is_empty(), "Expected diagnostics for parse error");
    let diagnostic = report.diagnostics.iter().find(|d| {
        d.level == DiagnosticLevel::Error && d.message.contains("parse error")
    });
    let diagnostic = diagnostic.expect("Expected diagnostics to contain 'parse error'");
    assert_ne!(diagnostic.span.line, 0, "parse error diagnostic has no real source span");
    assert!(
        line_at(src, diagnostic).contains("archive"),
        "parse error span should point at the malformed store statement's line, got line {}: {:?}",
        diagnostic.span.line, line_at(src, diagnostic)
    );
}

#[test]
fn test_check_type_error() {
    let src = r#"
        store "README.md" into archive
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected program with type error to fail check");
    assert!(!report.diagnostics.is_empty(), "Expected diagnostics for type error");
    let diagnostic = report.diagnostics.iter().find(|d| {
        d.level == DiagnosticLevel::Error && (d.message.contains("is not declared") || d.message.contains("undeclared"))
    });
    let diagnostic = diagnostic.expect("Expected diagnostics to contain 'is not declared'");
    assert_ne!(diagnostic.span.line, 0, "type error diagnostic has no real source span");
    assert!(
        line_at(src, diagnostic).contains("store"),
        "type error span should point at the store statement's line, got line {}: {:?}",
        diagnostic.span.line, line_at(src, diagnostic)
    );
}

#[test]
fn test_check_effect_confirmation_error() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }
        delete "old.bin" from archive
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected program with missing effect confirmation to fail check");
    assert!(!report.diagnostics.is_empty(), "Expected diagnostics for effect error");
    let diagnostic = report.diagnostics.iter().find(|d| {
        d.level == DiagnosticLevel::Error && d.message.contains("requires explicit physical key confirmation")
    });
    let diagnostic = diagnostic.expect("Expected diagnostics to contain confirmation error");
    assert_ne!(diagnostic.span.line, 0, "effect confirmation diagnostic has no real source span");
    assert!(
        line_at(src, diagnostic).contains("delete"),
        "confirmation error span should point at the delete statement's line, got line {}: {:?}",
        diagnostic.span.line, line_at(src, diagnostic)
    );
}
