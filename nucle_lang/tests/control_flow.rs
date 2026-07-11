use nucle_lang::typeck::check_and_desugar;
use nucle_lang::{check_source, Declaration, Operation, Program};

fn parse(src: &str) -> Program {
    let tokens = nucle_lang::Lexer::new(src).tokenize().unwrap();
    nucle_lang::Parser::new(tokens).parse_program().unwrap()
}

#[test]
fn if_true_branch_is_taken_and_desugared_away() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        if noisy > 0.1 {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        } else {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
        }
    "#;
    let program = parse(src);
    let (report, desugared) = check_and_desugar(&program);
    assert!(!report.has_errors(), "expected no errors, got: {:?}", report.diagnostics);
    assert!(!desugared.declarations.iter().any(|d| matches!(d, Declaration::If(_))));

    let recovered = desugared
        .declarations
        .iter()
        .find_map(|d| match d {
            Declaration::Let(binding) if binding.name == "recovered" => Some(binding),
            _ => None,
        })
        .expect("expected a 'recovered' binding to survive desugaring");
    match &recovered.expr {
        nucle_lang::Expr::FunctionCall { name, args, .. } if name == "consensus_vote" => {
            assert_eq!(args[1], nucle_lang::Expr::Number(10.0));
        }
        other => panic!("expected consensus_vote call, got {:?}", other),
    }
}

#[test]
fn if_false_branch_falls_through_to_else() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        if noisy > 10.0 {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        } else {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
        }
    "#;
    let program = parse(src);
    let (report, desugared) = check_and_desugar(&program);
    assert!(!report.has_errors(), "expected no errors, got: {:?}", report.diagnostics);

    let recovered = desugared
        .declarations
        .iter()
        .find_map(|d| match d {
            Declaration::Let(binding) if binding.name == "recovered" => Some(binding),
            _ => None,
        })
        .expect("expected a 'recovered' binding to survive desugaring");
    match &recovered.expr {
        nucle_lang::Expr::FunctionCall { name, args, .. } if name == "consensus_vote" => {
            assert_eq!(args[1], nucle_lang::Expr::Number(2.0));
        }
        other => panic!("expected consensus_vote call, got {:?}", other),
    }
}

#[test]
fn if_without_else_produces_nothing_when_condition_is_false() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        if noisy > 10.0 {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        }
    "#;
    let program = parse(src);
    let (report, desugared) = check_and_desugar(&program);
    assert!(!report.has_errors(), "expected no errors, got: {:?}", report.diagnostics);
    assert!(!desugared.declarations.iter().any(|d| matches!(d, Declaration::Let(binding) if binding.name == "recovered")));
}

#[test]
fn boolean_operators_combine_comparisons() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        if noisy > 0.1 && noisy < 1.0 {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        }
        if noisy > 10.0 || !(noisy > 1.0) {
            let recovered2: Pool<Recovered> = consensus_vote(noisy, coverage: 5x)
        }
    "#;
    let program = parse(src);
    let (report, desugared) = check_and_desugar(&program);
    assert!(!report.has_errors(), "expected no errors, got: {:?}", report.diagnostics);
    assert!(desugared.declarations.iter().any(|d| matches!(d, Declaration::Let(b) if b.name == "recovered")));
    assert!(desugared.declarations.iter().any(|d| matches!(d, Declaration::Let(b) if b.name == "recovered2")));
}

#[test]
fn for_loop_unrolls_over_pool_names() {
    let src = r#"
        pool archive1: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        pool archive2: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        for target in [archive1, archive2] {
            store "genome.fasta" into target { redundancy: 4x }
        }
    "#;
    let program = parse(src);
    let (report, desugared) = check_and_desugar(&program);
    assert!(!report.has_errors(), "expected no errors, got: {:?}", report.diagnostics);
    assert!(!desugared.declarations.iter().any(|d| matches!(d, Declaration::For(_))));

    let store_pools: Vec<&str> = desugared
        .declarations
        .iter()
        .filter_map(|d| match d {
            Declaration::Operation(Operation::Store(store)) => Some(store.pool.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(store_pools, vec!["archive1", "archive2"]);
}

#[test]
fn condition_referencing_undeclared_binding_is_rejected() {
    let src = r#"
        if missing_pool > 0.1 {
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "expected undeclared condition binding to fail");
    assert!(report.diagnostics.iter().any(|d| d.code == "E-CONDITION-UNDECLARED"));
}

#[test]
fn non_boolean_condition_is_rejected() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        if noisy {
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        }
    "#;
    let report = check_source(src);
    assert!(!report.ok, "expected a bare (non-comparison) condition to fail");
    assert!(report.diagnostics.iter().any(|d| d.code == "E-CONDITION-NOT-BOOLEAN"));
}
