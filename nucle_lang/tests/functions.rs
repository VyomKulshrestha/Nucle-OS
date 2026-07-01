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
        fn process_pool(source: Pool<Illumina>, source: Pool<Illumina>) returns Void {
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
        
        fn process_pool(source: Pool<Illumina>) returns Pool<Recovered> {
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
        
        fn process(source: Pool<Illumina>) returns Void {
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
        
        fn process(source: Pool<Twist>) returns Void {
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
fn test_calling_destructive_function_requires_confirmation() {
    // Calling a function is not automatically Pure/pre-confirmed just
    // because it's wrapped in a function — the join effect of its body
    // (here, an unconfirmed destructive delete) must propagate to the call
    // site, exactly like a literal Destructive operation would.
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }

        fn purge() returns Void {
            delete "old_archive.bin" from archive
        }

        let result: Void = purge()
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected unconfirmed destructive function call to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error
                && d.message.contains("result")
                && d.message.contains("Destructive")
                && d.message.contains("confirmation")
        }),
        "Expected diagnostics to flag the call-site confirmation, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_calling_confirmed_destructive_function_passes() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }

        fn purge() returns Void {
            delete "old_archive.bin" from archive confirm physical_key
        }

        let result: Void = purge()
    "#;
    let report = check_source(src);
    assert!(report.ok, "Expected confirmed destructive function call to pass, got: {:?}", report.diagnostics);
}

#[test]
fn test_return_type_mismatch_is_rejected() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        fn process_pool(source: Pool<Illumina>) returns Pool<Recovered> {
            let noisy: Pool<Illumina, 0.35%> = simulate source under Illumina
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected return-type mismatch to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error
                && d.message.contains("process_pool")
                && d.message.contains("Recovered")
                && d.message.contains("Illumina")
        }),
        "Expected diagnostics to flag the return-type mismatch, got: {:?}",
        report.diagnostics
    );
}

#[test]
fn test_missing_return_type_is_a_parse_error() {
    let src = r#"
        fn process(source: Pool<Illumina>) {
            // no '->' or 'returns' before the body — must be rejected,
            // not silently defaulted to Void.
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "Expected missing return type to fail");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.level == DiagnosticLevel::Error
                && (d.message.contains("return type") || d.message.contains("'returns'") || d.message.contains("'->'"))
        }),
        "Expected diagnostics to flag the missing return type, got: {:?}",
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
