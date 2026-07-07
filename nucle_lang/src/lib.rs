//! # nucle_lang — NucleScript compiler
//!
//! NucleScript is a declarative operations language for NucleOS DNA storage
//! pools. It provides a practical compiler front-end (lexer, parser, type
//! checker) plus a small VFS execution backend for `.nsl` source files.

pub mod ast;
pub mod codegen;
pub mod effects;
pub mod hardware;
pub mod lexer;
pub mod lockfile;
pub mod middle;
pub mod package;
pub mod parser;
pub mod playground;
pub mod probabilistic;
pub mod sim_backend;
pub mod typeck;
pub mod diagnostics;
pub mod formatter;

use std::fmt;
use std::path::Path;

pub use ast::*;
pub use codegen::{execute_program, CompiledPlan, ExecutionReport, VfsCall};
pub use formatter::{format_source, is_formatted, FormatError};
pub use lexer::{Lexer, Token, TokenKind};
pub use parser::Parser;
pub use playground::{analyze_source, PlaygroundDiagnostic, PlaygroundReport};
pub use sim_backend::{compile_simulation, SimulationPlan, SimulationStep};
pub use typeck::{Diagnostic, DiagnosticLevel, SymbolTable, TypeReport};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CheckReport {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// Run lex -> parse -> typeck -> effect-check on source text.
pub fn check_source(source: &str) -> CheckReport {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            return CheckReport {
                ok: false,
                diagnostics: vec![Diagnostic {
                    level: DiagnosticLevel::Error,
                    code: "E-LEX-ERROR".to_string(),
                    message: format!("lex error: {}", err),
                    span: Span::point(err.line, err.column),
                }],
            };
        }
    };

    let program = match Parser::new(tokens).parse_program() {
        Ok(program) => program,
        Err(err) => {
            return CheckReport {
                ok: false,
                diagnostics: vec![Diagnostic {
                    level: DiagnosticLevel::Error,
                    code: "E-PARSE-ERROR".to_string(),
                    message: format!("parse error: {}", err),
                    span: Span::point(err.line, err.column),
                }],
            };
        }
    };

    let report = typeck::check_program(&program);
    CheckReport {
        ok: !report.has_errors(),
        diagnostics: report.diagnostics,
    }
}

/// Result of a full analysis pass: diagnostics plus the symbol table --
/// everything a language server needs from a single lex/parse/typeck pass
/// over one document, without re-deriving scope information a second time.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub report: CheckReport,
    pub symbols: SymbolTable,
}

/// Like `check_source`, but also returns the program's symbol table (see
/// `typeck::check_program_with_symbols`) -- the one entry point
/// `nucle_lsp` calls on every document open/change, since it needs both
/// diagnostics (to publish) and symbols (for hover/definition/outline)
/// from the same parse, not two independent ones that could disagree.
pub fn analyze(source: &str) -> Analysis {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            return Analysis {
                report: CheckReport {
                    ok: false,
                    diagnostics: vec![Diagnostic {
                        level: DiagnosticLevel::Error,
                        code: "E-LEX-ERROR".to_string(),
                        message: format!("lex error: {}", err),
                        span: Span::point(err.line, err.column),
                    }],
                },
                symbols: SymbolTable::default(),
            };
        }
    };

    let program = match Parser::new(tokens).parse_program() {
        Ok(program) => program,
        Err(err) => {
            return Analysis {
                report: CheckReport {
                    ok: false,
                    diagnostics: vec![Diagnostic {
                        level: DiagnosticLevel::Error,
                        code: "E-PARSE-ERROR".to_string(),
                        message: format!("parse error: {}", err),
                        span: Span::point(err.line, err.column),
                    }],
                },
                symbols: SymbolTable::default(),
            };
        }
    };

    let (report, symbols) = typeck::check_program_with_symbols(&program);
    Analysis {
        report: CheckReport { ok: !report.has_errors(), diagnostics: report.diagnostics },
        symbols,
    }
}

/// Run check on a source file.
pub fn check_source_file(path: impl AsRef<Path>) -> Result<CheckReport, CompileError> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path).map_err(|source| CompileError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(check_source(&source))
}

/// Compile source text through lexer, parser, type checker, and VFS codegen.
pub fn compile(source: &str) -> Result<CompiledPlan, CompileError> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse_program()?;
    let (report, desugared) = typeck::check_and_desugar(&program);
    if report.has_errors() {
        return Err(CompileError::Type(report));
    }
    Ok(codegen::compile_program(desugared, report))
}

/// Compile source text into a no-hardware simulation plan.
pub fn compile_for_simulation(source: &str) -> Result<SimulationPlan, CompileError> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse_program()?;
    let (report, desugared) = typeck::check_and_desugar(&program);
    if report.has_errors() {
        return Err(CompileError::Type(report));
    }
    Ok(sim_backend::compile_simulation(desugared, report))
}

/// Compile and execute a NucleScript source file against a fresh in-memory NucleOS instance.
pub fn run_source_file(path: impl AsRef<Path>) -> Result<ExecutionReport, CompileError> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path).map_err(|source| CompileError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut plan = compile(&source)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut os = nucle_vfs::syscall::NucleOS::new(100);
    execute_program(&mut os, &mut plan, base_dir).map_err(CompileError::Execution)
}

/// Backwards-compatible alias for callers that used the initial API name.
pub fn run_script(path: impl AsRef<Path>) -> Result<ExecutionReport, CompileError> {
    run_source_file(path)
}

#[derive(Debug)]
pub enum CompileError {
    Lex(lexer::LexError),
    Parse(parser::ParseError),
    Type(TypeReport),
    Execution(String),
    Io { path: String, source: std::io::Error },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "lex error: {}", err),
            Self::Parse(err) => write!(f, "parse error: {}", err),
            Self::Type(report) => write!(f, "type check failed:\n{}", report),
            Self::Execution(err) => write!(f, "execution failed: {}", err),
            Self::Io { path, source } => write!(f, "failed to read '{}': {}", path, source),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<lexer::LexError> for CompileError {
    fn from(value: lexer::LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<parser::ParseError> for CompileError {
    fn from(value: parser::ParseError) -> Self {
        Self::Parse(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_store_program() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
            store "README.md" into archive { redundancy: 4x, tag: ["medical", "critical"] }
        "#;
        let plan = compile(src).unwrap();
        assert_eq!(plan.calls.len(), 1);
        assert!(!plan.type_report.has_errors());
    }

    #[test]
    fn rejects_bad_strand_literal_at_compile_time() {
        let src = r#"strand bad: Strand = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA""#;
        let err = compile(src).unwrap_err();
        assert!(matches!(err, CompileError::Type(_)));
    }

    #[test]
    fn compiles_sequence_literals() {
        let src = r#"
            seq primer_p0: Sequence = "ATCGATCGGCTAGCTA"
            let primer_p1 = seq"ATCGATCG-GCTAGCTA"
        "#;
        let plan = compile(src).unwrap();
        assert!(plan.calls.is_empty());
        assert!(!plan.type_report.has_errors());
    }

    #[test]
    fn rejects_bad_sequence_literals() {
        let src = r#"let bad = seq"AAAAAAATTTTTTT""#;
        let err = compile(src).unwrap_err();
        assert!(matches!(err, CompileError::Type(_)));
    }

    #[test]
    fn compiles_probabilistic_pool_types() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        "#;
        let plan = compile(src).unwrap();
        assert!(plan.calls.is_empty());
        assert!(!plan.type_report.has_errors());
    }

    #[test]
    fn compiles_simulation_plan() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 1x, profile: Nanopore }
            simulate store "README.md" into archive { coverage: 1x }
        "#;
        let plan = compile_for_simulation(src).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.optimiser_notes.len(), 1);
    }

    #[test]
    fn analyze_returns_symbol_table_alongside_diagnostics() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
            seq primer: Sequence = "ATCGATCGGCTAGCTA"
            fn archive_it(source: Pool<Illumina>) -> Pool<Recovered> {
                let result: Pool<Recovered> = consensus_vote(source, coverage: 10x)
            }
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = archive_it(noisy)
        "#;
        let analysis = analyze(src);
        assert!(analysis.report.ok, "expected valid program, got: {:?}", analysis.report.diagnostics);
        assert!(analysis.symbols.pools.contains_key("archive"));
        assert!(analysis.symbols.sequences.contains_key("primer"));
        assert!(analysis.symbols.functions.contains_key("archive_it"));
        assert!(analysis.symbols.bindings.contains_key("noisy"));
        assert!(analysis.symbols.bindings.contains_key("recovered"));

        let pool_span = analysis.symbols.pools["archive"].span;
        assert_ne!(pool_span.line, 0, "pool symbol should carry a real span");
    }

    #[test]
    fn analyze_returns_lex_error_with_empty_symbols() {
        let src = r#"store "unterminated into archive"#;
        let analysis = analyze(src);
        assert!(!analysis.report.ok);
        assert!(analysis.symbols.pools.is_empty());
    }
}
