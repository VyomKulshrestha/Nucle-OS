//! Integration test for `nucle doc`, exercised through real source text
//! (lex -> parse -> `generate_docs`) rather than a hand-built AST -- the
//! unit tests in `nucle_lang/src/docgen.rs` already cover the renderer's
//! internal logic directly.

use nucle_lang::{generate_docs, Lexer, Parser};

fn parse(src: &str) -> nucle_lang::Program {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse_program().unwrap()
}

#[test]
fn a_function_with_a_doc_comment_produces_the_expected_markdown_section() {
    let src = r#"
        /// Archives a file with the given recovery guarantee.
        fn archive(data: File, target: Pool<Illumina>, guarantee: Recovery) returns DnaFile {
            let plan: DnaFile = protect data for guarantee
            store plan into target
        }
    "#;
    let docs = generate_docs(&parse(src));

    assert!(docs.contains("## Functions"), "got:\n{docs}");
    assert!(docs.contains("### `archive`"), "got:\n{docs}");
    assert!(docs.contains("Archives a file with the given recovery guarantee."), "got:\n{docs}");
    assert!(
        docs.contains("fn archive(data: File, target: Pool<Illumina>, guarantee: Recovery) -> DnaFile"),
        "got:\n{docs}"
    );
    // `store` inside the body gives this function a `Synthesis` effect,
    // requiring `confirm hardware` at any call site -- docgen should
    // surface that, not just the signature.
    assert!(docs.contains("**Effect:** Synthesis"), "got:\n{docs}");
}

#[test]
fn a_multi_line_doc_comment_is_joined_into_one_paragraph() {
    let src = r#"
        /// First line of the description.
        /// Second line of the description.
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
    "#;
    let docs = generate_docs(&parse(src));
    assert!(docs.contains("First line of the description.\nSecond line of the description."), "got:\n{docs}");
}

#[test]
fn a_declaration_with_no_doc_comment_still_gets_an_entry() {
    let src = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }";
    let docs = generate_docs(&parse(src));
    assert!(docs.contains("### `archive`"), "got:\n{docs}");
    assert!(docs.contains("pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }"));
}

#[test]
fn documents_pools_strands_sequences_functions_and_pipelines_in_one_program() {
    let src = r#"
        /// The main archive.
        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

        /// A hardcoded primer strand.
        strand primer: Strand = "ATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTAATCGATCGGCTAGCTA"

        /// A DNA-native sequence literal.
        seq marker: Sequence = "ATCGATCGGCTAGCTA"

        /// Backs up records into the archive.
        pipeline backup {
            encode "records/" using Ternary,
            protect with redundancy 3x,
            store into archive,
            verify roundtrip
        }
    "#;
    let docs = generate_docs(&parse(src));
    for heading in ["## Pools", "## Strands", "## Sequences", "## Pipelines"] {
        assert!(docs.contains(heading), "missing {heading} in:\n{docs}");
    }
    assert!(docs.contains("The main archive."));
    assert!(docs.contains("A hardcoded primer strand."));
    assert!(docs.contains("A DNA-native sequence literal."));
    assert!(docs.contains("Backs up records into the archive."));
}
