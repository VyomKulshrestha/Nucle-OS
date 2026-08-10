use nucle_lang::{check_source, DiagnosticLevel};

fn error_codes(source: &str) -> Vec<String> {
    check_source(source)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn result_producing_calls_enforce_arity() {
    let codes = error_codes(
        r#"
fn wrap(value: Str) returns Result<Str, Str> {
    let wrapped: Result<Str, Str> = Ok(value)
}
let result: Result<Str, Str> = wrap()
"#,
    );

    assert!(codes.contains(&"E-FUNCTION-ARITY".to_string()));
}

#[test]
fn result_producing_generic_calls_unify_pool_arguments() {
    let report = check_source(
        r#"
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
fn wrap<P>(value: Pool<P>) returns Result<Str, Str> {
    let message: Str = "ready"
    let wrapped: Result<Str, Str> = Ok(message)
}
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
let result: Result<Str, Str> = wrap(noisy)
"#,
    );

    assert!(
        report.ok,
        "expected generic Result call to type-check: {:?}",
        report.diagnostics
    );
}

#[test]
fn enum_producing_match_scrutinees_enforce_arity() {
    let codes = error_codes(
        r#"
enum Status { Ready }
fn status(value: Str) returns Status {
    let ready: Status = Status::Ready
}
let label: Str = match status() {
    Ready => "ready"
}
"#,
    );

    assert!(codes.contains(&"E-FUNCTION-ARITY".to_string()));
}
