use nucle_lang::{format_source, is_formatted};
use std::path::Path;

/// Every `.nsl` file directly under a given directory.
fn nsl_files_in(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("nsl"))
        .collect();
    files.sort();
    files
}

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

/// `nucle fmt --write` once, then `nucle fmt --check` forever after: every
/// shipped example must both format without error and be a fixed point of
/// formatting (formatting its own output changes nothing). This is the
/// acceptance bar from actions.md's Step 5 plan, run for real against every
/// file NucleScript actually ships, not just hand-picked snippets.
#[test]
fn every_example_formats_and_is_idempotent() {
    let dir = examples_dir();
    let mut files = nsl_files_in(&dir);
    files.extend(nsl_files_in(&dir.join("failures")));
    assert!(!files.is_empty(), "expected to find .nsl example files under {}", dir.display());

    for path in files {
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
        let once = format_source(&source).unwrap_or_else(|e| panic!("{} failed to format: {}", path.display(), e));
        let twice = format_source(&once).unwrap_or_else(|e| panic!("{} failed to re-format its own output: {}", path.display(), e));
        assert_eq!(once, twice, "formatting {} is not idempotent", path.display());
        assert!(is_formatted(&once).unwrap(), "{}'s own formatted output does not report itself as already formatted", path.display());
    }
}

#[test]
fn formatting_never_changes_the_parsed_program() {
    let dir = examples_dir();
    let mut files = nsl_files_in(&dir);
    files.extend(nsl_files_in(&dir.join("failures")));

    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        let formatted = format_source(&source).unwrap_or_else(|e| panic!("{} failed to format: {}", path.display(), e));

        let before = nucle_lang::Parser::new(nucle_lang::Lexer::new(&source).tokenize().unwrap()).parse_program().unwrap();
        let after = nucle_lang::Parser::new(nucle_lang::Lexer::new(&formatted).tokenize().unwrap()).parse_program().unwrap();

        // Spans legitimately shift (formatting changes line/column), so
        // compare declarations with spans zeroed out rather than raw
        // equality -- this is checking "same program", not "same source
        // positions".
        let strip_spans = |mut program: nucle_lang::Program| -> nucle_lang::Program {
            for decl in &mut program.declarations {
                zero_span(decl);
            }
            program
        };
        assert_eq!(
            strip_spans(before),
            strip_spans(after),
            "formatting {} changed the parsed program (should only change whitespace/comments)",
            path.display()
        );
    }
}

fn zero_span(decl: &mut nucle_lang::Declaration) {
    use nucle_lang::Declaration::*;
    let zero = nucle_lang::Span::default();
    match decl {
        Import(d) => d.span = zero,
        Pool(d) => d.span = zero,
        Strand(d) => d.span = zero,
        Sequence(d) => d.span = zero,
        Let(d) => d.span = zero,
        Pipeline(d) => d.span = zero,
        Function(d) => {
            d.span = zero;
            for inner in &mut d.body {
                zero_span(inner);
            }
        }
        If(d) => {
            d.span = zero;
            for inner in &mut d.then_branch {
                zero_span(inner);
            }
            if let Some(branch) = &mut d.else_branch {
                for inner in branch {
                    zero_span(inner);
                }
            }
        }
        For(d) => {
            d.span = zero;
            for inner in &mut d.body {
                zero_span(inner);
            }
        }
        Test(d) => {
            d.span = zero;
            for inner in &mut d.body {
                zero_span(inner);
            }
        }
        Operation(op) => {
            use nucle_lang::Operation::*;
            match op {
                Store(o) => o.span = zero,
                Retrieve(o) => o.span = zero,
                Delete(o) => o.span = zero,
                Assert(o) => o.span = zero,
            }
        }
        Enum(d) => d.span = zero,
    }
}

/// A deliberately misformatted input should come out matching a
/// hand-written expected canonical form -- the idempotence test above
/// only proves formatting is *stable*, not that it's the style actually
/// intended, so this pins down the concrete expected output for a case
/// covering most of the grammar at once.
#[test]
fn reformats_a_misformatted_program_to_the_expected_canonical_form() {
    let src = r#"
        pool   archive :DnaPool{
codec:Ternary,redundancy:3x,profile:Illumina}


        let noisy:Pool<Illumina,0.35%>   =   simulate archive under Illumina
        if noisy>0.1&&noisy<5.0{
        let recovered:Pool<Recovered> =consensus_vote(noisy,coverage:10x)
        }else{
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
        }
        store "a.txt" into archive
    "#;

    let expected = "pool archive: DnaPool {\n    codec: Ternary, redundancy: 3x, profile: Illumina }\n\nlet noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina\n\nif noisy > 0.1 && noisy < 5.0 {\n    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)\n} else {\n    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)\n}\n\nstore \"a.txt\" into archive\n";

    let formatted = format_source(src).unwrap();
    assert_eq!(formatted, expected);
}
