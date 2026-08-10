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
fn string_literal_cannot_silently_initialize_a_dna_file() {
    let codes = error_codes(r#"let invalid: DnaFile = "plain text""#);
    assert!(codes.contains(&"E-BINDING-TYPE-MISMATCH".to_string()));
}

#[test]
fn void_function_call_cannot_silently_initialize_a_dna_file() {
    let codes = error_codes(
        r#"
fn noop() returns Void {}
let invalid: DnaFile = noop()
"#,
    );
    assert!(codes.contains(&"E-BINDING-TYPE-MISMATCH".to_string()));
}

#[test]
fn ordinary_value_parameter_and_return_types_are_checked() {
    let report = check_source(
        r#"
fn identity(value: Str) returns Str {
    let result: Str = value
}
let answer: Str = identity("ok")
"#,
    );
    assert!(
        report.ok,
        "expected a sound scalar call, got: {:?}",
        report.diagnostics
    );

    let codes = error_codes(
        r#"
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
fn identity(value: Str) returns Str {
    let result: Str = value
}
let answer: Str = identity(noisy)
"#,
    );
    assert!(codes.contains(&"E-ARG-TYPE-MISMATCH".to_string()));

    let codes = error_codes(
        r#"
fn source(value: Str) returns Str {
    let result: Str = value
}
fn identity(value: Str) returns Str {
    let result: Str = value
}
let answer: Str = identity(source())
"#,
    );
    assert!(codes.contains(&"E-FUNCTION-ARITY".to_string()));
}

#[test]
fn non_void_function_and_closure_require_a_matching_tail_value() {
    let function_codes = error_codes("fn missing() returns Str {}");
    assert!(function_codes.contains(&"E-RETURN-TYPE-MISMATCH".to_string()));

    let closure_codes = error_codes(r#"let missing: Fn() -> Str = fn() -> Str {}"#);
    assert!(closure_codes.contains(&"E-CLOSURE-RETURN-TYPE-MISMATCH".to_string()));
}

#[test]
fn protect_produces_the_dna_file_value_its_examples_bind() {
    let report = check_source(
        r#"
fn archive(data: File, target: Pool<Illumina>, guarantee: Recovery) returns DnaFile {
    let plan: DnaFile = protect data for guarantee
    store plan into target
}
"#,
    );
    assert!(
        report.ok,
        "protect should preserve a DnaFile value: {:?}",
        report.diagnostics
    );

    let codes = error_codes(
        r#"
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
let invalid: DnaFile = protect noisy for noisy
"#,
    );
    assert!(codes.contains(&"E-ARG-TYPE-MISMATCH".to_string()));
}
