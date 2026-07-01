use nucle_lang::{check_source, DiagnosticLevel};

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
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("lex error")
        }),
        "Expected diagnostics to contain 'lex error'"
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
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("parse error")
        }),
        "Expected diagnostics to contain 'parse error'"
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
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && (d.message.contains("is not declared") || d.message.contains("undeclared"))
        }),
        "Expected diagnostics to contain 'is not declared', got diagnostics: {:?}",
        report.diagnostics
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
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("requires explicit physical key confirmation")
        }),
        "Expected diagnostics to contain confirmation error, got diagnostics: {:?}",
        report.diagnostics
    );
}
