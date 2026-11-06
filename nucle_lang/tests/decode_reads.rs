//! `decode reads "<reads>" against "<references>"`: real sequencing reads
//! decoded and scored by a NucleScript program, with the numbers `nucle
//! reads` publishes.

use nucle_codec::base::DnaStrand;
use nucle_ecc::cluster::ClusterConfig;
use nucle_ecc::realdata::{evaluate_blind_fitted, evaluate_published, evaluate_unlabelled, parse_cnr, parse_sequences};
use nucle_lang::effects::effect_summary;
use nucle_lang::lexer::Lexer;
use nucle_lang::parser::Parser;
use nucle_lang::{check_source, compile, execute_program, format_source, Declaration, DecodeReadsOp, Effect, Operation, Program};
use nucle_vfs::syscall::NucleOS;
use std::path::{Path, PathBuf};

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| panic!("lex error: {}", e));
    Parser::new(tokens).parse_program().unwrap_or_else(|e| panic!("parse error: {}", e))
}

fn parse_error(src: &str) -> String {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse_program().expect_err("should not parse").to_string()
}

/// The 50-cluster MinION sample of the Clustered Nanopore Reads dataset.
fn cnr() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("nucle_ecc").join("tests").join("fixtures").join("cnr")
}

/// Runs `source` from `dir`; the steps it logged, or the error it stopped on.
fn run(source: &str, dir: &Path) -> Result<Vec<String>, String> {
    let mut plan = compile(source).map_err(|e| e.to_string())?;
    let mut os = NucleOS::new(10);
    execute_program(&mut os, &mut plan, dir).map(|report| report.steps)
}

#[test]
fn every_option_parses() {
    let program = parse("decode reads \"Clusters.txt\" against \"Centers.txt\" { blind, limit: 20, expect recovery > 90% }");
    let Declaration::Operation(Operation::Decode(op)) = &program.declarations[0] else { panic!("expected decode") };
    let DecodeReadsOp { reads, references, blind, limit, expect_recovery_gt, .. } = op;
    assert_eq!((reads.as_str(), references.as_str(), *blind, *limit, *expect_recovery_gt), ("Clusters.txt", "Centers.txt", true, Some(20), Some(90.0)));

    let bare = parse("decode reads \"r.fastq\" against \"refs.fasta\"");
    let Declaration::Operation(Operation::Decode(op)) = &bare.declarations[0] else { panic!("expected decode") };
    assert!(!op.blind && op.limit.is_none() && op.expect_recovery_gt.is_none());
}

#[test]
fn bad_options_are_refused() {
    assert!(parse_error("decode reads \"r\" against \"c\" { limit: 0 }").contains("limit must be a whole number"));
    assert!(parse_error("decode reads \"r\" against \"c\" { coverage: 3x }").contains("unknown decode option 'coverage'"));
    let codes: Vec<String> = check_source("decode reads \"r\" against \"c\" { expect recovery > 100% }").diagnostics.into_iter().map(|d| d.code).collect();
    assert_eq!(codes, vec!["E-DECODE-EXPECT-INVALID"]);
}

#[test]
fn decoding_reads_touches_no_hardware() {
    let summary = effect_summary(&parse("decode reads \"r\" against \"c\""));
    assert_eq!((summary.declarations[0].kind.as_str(), summary.declarations[0].effect), ("decode", Effect::Pure));
}

/// The statement logs the numbers `nucle reads` computes, from the same
/// functions.
#[test]
fn the_minion_sample_decodes_to_the_numbers_nucle_reads_reports() {
    let dir = cnr();
    let steps = run("decode reads \"Clusters.txt\" against \"Centers.txt\" { blind, expect recovery > 90% }", &dir).unwrap();

    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    let data = parse_cnr(&read("Centers.txt"), &read("Clusters.txt")).unwrap();
    let labelled = evaluate_published(&data);
    let (config, blind) = evaluate_blind_fitted(&data);
    assert!(steps[0].contains(&format!("{} reads of 50 references", labelled.reads)), "{}", steps[0]);
    assert!(steps[0].contains(&format!("consensus exact for {} ({:.2}%)", labelled.exact, labelled.exact_rate() * 100.0)), "{}", steps[0]);
    assert!(steps[1].contains(&format!("{} clusters at a {:.3} merge threshold", blind.clusters, config.max_distance_fraction)), "{}", steps[1]);
    assert!(steps[1].contains(&format!("{} of 50 references recovered", blind.recovered_exact)), "{}", steps[1]);
    assert!(blind.recovery_rate() > 0.9);
}

#[test]
fn an_unmet_expectation_stops_the_run() {
    let err = run("decode reads \"Clusters.txt\" against \"Centers.txt\" { expect recovery > 95% }", &cnr()).unwrap_err();
    assert!(err.contains("not more than the 95% expected"), "{err}");
    // The first 20 references all come back.
    run("decode reads \"Clusters.txt\" against \"Centers.txt\" { limit: 20, expect recovery > 99.5% }", &cnr()).unwrap();
}

#[test]
fn a_decode_inside_a_function_runs_and_its_failure_reaches_the_caller() {
    let src = "fn check() returns Void {\n    decode reads \"Clusters.txt\" against \"Centers.txt\" { expect recovery > 95% }\n}\nlet checked: Void = check()\n";
    let err = run(src, &cnr()).unwrap_err();
    assert!(err.starts_with("decode reads Clusters.txt recovered 92.00% of its references"), "{err}");
}

/// Reads straight off a sequencer carry no cluster labels: FASTQ, with a
/// read holding an N call, against FASTA references.
#[test]
fn unlabelled_fastq_reads_decode_against_fasta_references() {
    let source = cnr();
    let read = |name: &str| std::fs::read_to_string(source.join(name)).unwrap();
    let data = parse_cnr(&read("Centers.txt"), &read("Clusters.txt")).unwrap();
    let dir = std::env::temp_dir().join(format!("nsl_decode_fastq_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut fastq = String::new();
    let mut reads: Vec<DnaStrand> = Vec::new();
    for (i, read) in data.iter().flat_map(|c| c.reads.iter()).enumerate() {
        let seq = read.to_string();
        fastq.push_str(&format!("@read{i}\n{seq}\n+\n{}\n", "@".repeat(seq.len())));
        reads.push(read.clone());
    }
    fastq.push_str("@withN\nACGTNACGT\n+\nIIIIIIIII\n");
    let fasta: String = data.iter().enumerate().map(|(i, c)| format!(">ref{i}\n{}\n", c.reference)).collect();
    std::fs::write(dir.join("reads.fastq"), fastq).unwrap();
    std::fs::write(dir.join("refs.fasta"), fasta).unwrap();

    let steps = run("decode reads \"reads.fastq\" against \"refs.fasta\" { expect recovery > 85% }", &dir).unwrap();
    let references: Vec<DnaStrand> = data.iter().map(|c| c.reference.clone()).collect();
    let report = evaluate_unlabelled(&reads, &references, &ClusterConfig::fitted(&reads));
    assert!(steps[0].contains(&format!("{} unlabelled reads (1 with other bases left out)", reads.len())), "{}", steps[0]);
    assert!(steps[0].contains(&format!("{} of 50 references recovered", report.recovered_exact)), "{}", steps[0]);
    let (parsed, skipped) = parse_sequences(&std::fs::read_to_string(dir.join("reads.fastq")).unwrap(), true).unwrap();
    assert_eq!((parsed.len(), skipped), (reads.len(), 1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_decode_statement_formats_stably() {
    let messy = "decode   reads \"r.fastq\"   against \"refs.fasta\" {blind,limit: 5,expect recovery > 90%}\n";
    let once = format_source(messy).unwrap();
    assert_eq!(format_source(&once).unwrap(), once);
    let Declaration::Operation(Operation::Decode(op)) = &parse(&once).declarations[0] else { panic!("expected decode in {once}") };
    assert!(op.blind && op.limit == Some(5) && op.expect_recovery_gt == Some(90.0), "{once}");
}
