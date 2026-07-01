use nucle_lang::{Lexer, Parser};
use nucle_lang::effects::effect_summary;
use nucle_lang::middle::lower_program;
use nucle_lang::diagnostics::generate_explanation;

#[test]
fn test_explain_redundancy_bump() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 1x, profile: Nanopore }
        store "data.bin" into archive
    "#;
    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse_program().unwrap();
    let summary = effect_summary(&program);
    let mir_program = lower_program(&program);
    let notes = mir_program.notes;
    let explanation = generate_explanation(&notes, &summary);

    assert!(explanation.contains("raised redundancy"), "Expected explanation to mention redundancy bump, got:\n{}", explanation);
    assert!(explanation.contains("data.bin"), "Expected explanation to mention data.bin");
    assert!(explanation.contains("Nanopore"), "Expected explanation to mention Nanopore");
}

#[test]
fn test_explain_unconfirmed_delete() {
    let src = r#"
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
        delete "critical.bin" from archive
    "#;
    let tokens = Lexer::new(src).tokenize().unwrap();
    let program = Parser::new(tokens).parse_program().unwrap();
    let summary = effect_summary(&program);
    let mir_program = lower_program(&program);
    let notes = mir_program.notes;
    let explanation = generate_explanation(&notes, &summary);

    assert!(explanation.contains("delete 'critical.bin'"), "Expected explanation to mention delete, got:\n{}", explanation);
    assert!(explanation.contains("REQUIRES CONFIRMATION"), "Expected explanation to mention REQUIRES CONFIRMATION, got:\n{}", explanation);
    assert!(explanation.contains("permanently destroys physical DNA tubes"), "Expected explanation to mention physical DNA destruction");
}
