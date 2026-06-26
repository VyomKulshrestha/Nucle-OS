//! # nucle_lang — NucleScript compiler
//!
//! NucleScript is a declarative operations language for NucleOS DNA storage
//! pools. It provides a practical compiler front-end (lexer, parser, type
//! checker) plus a small VFS execution backend for `.nsl` source files.

pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod parser;
pub mod typeck;

use std::fmt;
use std::path::Path;

pub use ast::*;
pub use codegen::{execute_program, CompiledPlan, ExecutionReport, VfsCall};
pub use lexer::{Lexer, Token, TokenKind};
pub use parser::Parser;
pub use typeck::{Diagnostic, DiagnosticLevel, TypeReport};

/// Compile source text through lexer, parser, type checker, and VFS codegen.
pub fn compile(source: &str) -> Result<CompiledPlan, CompileError> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens).parse_program()?;
    let report = typeck::check_program(&program);
    if report.has_errors() {
        return Err(CompileError::Type(report));
    }
    Ok(codegen::compile_program(program, report))
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
}
