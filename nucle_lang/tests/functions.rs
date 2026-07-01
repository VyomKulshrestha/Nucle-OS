use nucle_lang::{check_source, DiagnosticLevel};

#[test]
fn test_valid_function_compilation() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        
        fn process_pool(source: Pool<Illumina>) returns Pool<Recovered> {
            let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
        }
    "#;
    let report = check_source(src);
    assert!(report.ok, "Expected valid function to pass check, got: {:?}", report.diagnostics);
}

#[test]
fn test_duplicate_parameter_names() {
    let src = r#"
        fn process_pool(source: Pool<Illumina>, source: Pool<Illumina>) {
            // empty
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected duplicate parameters to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("duplicate parameter name")
        }),
        "Expected diagnostics to contain duplicate parameter error, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_undeclared_variable_in_function() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        
        fn process_pool(source: Pool<Illumina>) {
            let recovered: Pool<Recovered> = consensus_vote(undeclared_var, coverage: 10x)
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected undeclared variable in function to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && (d.message.contains("undeclared") || d.message.contains("not declared") || d.message.contains("not a probabilistic pool binding"))
        }),
        "Expected diagnostics to contain undeclared variable error, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_calling_undeclared_function() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        let res: Pool<Recovered> = calling_missing_fn(noisy)
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected calling undeclared function to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("function") && d.message.contains("undeclared")
        }),
        "Expected diagnostics to contain undeclared function error, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_function_arity_mismatch() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        
        fn process(source: Pool<Illumina>) {
            // empty
        }

        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        let res: Void = process(noisy, noisy)
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected arity mismatch to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("expects 1 arguments, but 2 were provided")
        }),
        "Expected diagnostics to contain arity mismatch error, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_function_argument_type_mismatch() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        
        fn process(source: Pool<Twist>) {
            // empty
        }

        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        let res: Void = process(noisy)
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected argument type mismatch to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error && d.message.contains("expects Pool<Twist>, but got Pool<Illumina>")
        }),
        "Expected diagnostics to contain argument type mismatch error, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_archive_example_compiles() {
    let src = r#"
        fn archive(data: File, target: Pool<Illumina>, guarantee: Recovery) returns DnaFile {
            let plan: DnaFile = protect data for guarantee
            store plan into target
        }
    "#;
    let report = check_source(src);
    assert!(report.ok, "Expected archive example to compile, got: {:?}", report.diagnostics);
}
