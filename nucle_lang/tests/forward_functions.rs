use nucle_lang::{check_source, DiagnosticLevel};

#[test]
fn function_can_call_a_later_declaration() {
    let source = r#"
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

fn prepare() returns Pool<Illumina> {
    let ready: Pool<Illumina> = finish()
}

fn finish() returns Pool<Illumina> {
    let ready: Pool<Illumina> = simulate archive under Illumina
}

let result: Pool<Illumina> = prepare()
"#;
    let report = check_source(source);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error),
        "later-declared function should resolve from the whole-program signature table: {:?}",
        report.diagnostics
    );
}

#[test]
fn mutually_referenced_function_signatures_type_check() {
    let source = r#"
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

fn left() returns Pool<Illumina> {
    let result: Pool<Illumina> = right()
}

fn right() returns Pool<Illumina> {
    let result: Pool<Illumina> = left()
}
"#;
    let report = check_source(source);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error),
        "mutually referenced signatures should not depend on declaration order: {:?}",
        report.diagnostics
    );
}

#[test]
fn duplicate_function_is_reported_once_after_predeclaration() {
    let source = r#"
fn duplicate() returns Void {}
fn duplicate() returns Void {}
"#;
    let report = check_source(source);
    let duplicates = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.level == DiagnosticLevel::Error && diagnostic.code == "E-FUNCTION-DUPLICATE"
        })
        .count();
    assert_eq!(
        duplicates, 1,
        "duplicate declaration should have one stable diagnostic"
    );
}
