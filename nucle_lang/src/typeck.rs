//! Semantic analysis and biological constraint checking for NucleScript.

use crate::ast::*;
use nucle_codec::base::DnaStrand;
use nucle_codec::constraints::{ConstraintConfig, ConstraintValidator};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Default)]
pub struct TypeReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl TypeReport {
    pub fn error(&mut self, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { level: DiagnosticLevel::Error, message: message.into() });
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { level: DiagnosticLevel::Warning, message: message.into() });
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error)
    }

    pub fn has_warnings(&self) -> bool {
        self.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Warning)
    }
}

impl fmt::Display for TypeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.diagnostics.is_empty() {
            return write!(f, "no diagnostics");
        }
        for diagnostic in &self.diagnostics {
            writeln!(f, "{}: {}", diagnostic.level, diagnostic.message)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
        }
    }
}

pub fn check_program(program: &Program) -> TypeReport {
    let mut checker = TypeChecker::default();
    checker.check(program);
    checker.report
}

#[derive(Default)]
struct TypeChecker {
    pools: HashMap<String, PoolDecl>,
    strands: HashSet<String>,
    sequences: HashSet<String>,
    report: TypeReport,
}

impl TypeChecker {
    fn check(&mut self, program: &Program) {
        for declaration in &program.declarations {
            match declaration {
                Declaration::Pool(pool) => self.check_pool(pool),
                Declaration::Strand(strand) => self.check_strand(strand),
                Declaration::Sequence(sequence) => self.check_sequence(sequence),
                Declaration::Operation(Operation::Store(store)) => self.check_store(store),
                Declaration::Operation(Operation::Retrieve(retrieve)) => self.check_retrieve(retrieve),
                Declaration::Pipeline(pipeline) => self.check_pipeline(pipeline),
            }
        }
    }

    fn check_pool(&mut self, pool: &PoolDecl) {
        if self.pools.contains_key(&pool.name) {
            self.report.error(format!("pool '{}' is declared more than once", pool.name));
            return;
        }
        if pool.redundancy == 1 {
            self.report.warning(format!(
                "pool '{}' has 1x redundancy; critical files should use at least 2x",
                pool.name
            ));
        }
        if pool.codec != Codec::Ternary {
            self.report.warning(format!(
                "pool '{}' declares {} codec; current VFS backend stores through the existing Ternary syscall path",
                pool.name, pool.codec
            ));
        }
        self.pools.insert(pool.name.clone(), pool.clone());
    }

    fn check_strand(&mut self, strand: &StrandDecl) {
        if !self.strands.insert(strand.name.clone()) {
            self.report.error(format!("strand '{}' is declared more than once", strand.name));
        }
        let parsed = match DnaStrand::from_str(&strand.sequence) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.report.error(format!("strand '{}' is not valid DNA: {}", strand.name, err));
                return;
            }
        };
        let validator = ConstraintValidator::new(nuclescript_constraints());
        let result = validator.validate(&parsed);
        for violation in result.violations {
            self.report.error(format!(
                "strand '{}' violates NucleScript biological constraint: {}",
                strand.name, violation
            ));
        }
    }

    fn check_sequence(&mut self, sequence: &SequenceDecl) {
        if !self.sequences.insert(sequence.name.clone()) {
            self.report.error(format!("sequence '{}' is declared more than once", sequence.name));
        }
        let normalized = match normalize_sequence_literal(&sequence.sequence) {
            Ok(normalized) => normalized,
            Err(err) => {
                self.report.error(format!("sequence '{}' is not valid DNA: {}", sequence.name, err));
                return;
            }
        };
        let parsed = match DnaStrand::from_str(&normalized) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.report.error(format!("sequence '{}' is not valid DNA: {}", sequence.name, err));
                return;
            }
        };
        let validator = ConstraintValidator::new(sequence_literal_constraints());
        let result = validator.validate(&parsed);
        for violation in result.violations {
            self.report.error(format!(
                "sequence '{}' violates NucleScript biological constraint: {}",
                sequence.name, violation
            ));
        }
    }

    fn check_store(&mut self, store: &StoreOp) {
        let Some(pool) = self.pools.get(&store.pool).cloned() else {
            self.report.error(format!("store target pool '{}' is not declared", store.pool));
            return;
        };
        let redundancy = store.options.redundancy.unwrap_or(pool.redundancy);
        if redundancy == 1 && store.options.tags.iter().any(|tag| tag.eq_ignore_ascii_case("critical")) {
            self.report.warning(format!(
                "store '{}' is tagged critical but uses only 1x redundancy",
                store.file
            ));
        }
        let coverage = store.options.coverage.unwrap_or(redundancy);
        if pool.profile == Profile::Nanopore
            && coverage <= 1
            && store.options.expect_recovery_gt.is_some_and(|recovery| recovery > 99.5)
        {
            self.report.warning(format!(
                "expect recovery > 99.5% is statistically unsatisfiable for Nanopore at {}x coverage",
                coverage
            ));
        }
        if store.simulate && store.options.coverage.is_none() {
            self.report.warning(format!(
                "simulate store '{}' uses implicit {}x coverage from redundancy",
                store.file, coverage
            ));
        }
    }

    fn check_retrieve(&mut self, retrieve: &RetrieveOp) {
        if !self.pools.contains_key(&retrieve.pool) {
            self.report.error(format!("retrieve source pool '{}' is not declared", retrieve.pool));
        }
        for predicate in &retrieve.query {
            match predicate.field.to_ascii_lowercase().as_str() {
                "tag" | "date" | "size" | "name" | "type" => {}
                other => self.report.warning(format!(
                    "query field '{}' is not indexed by the current VFS search backend",
                    other
                )),
            }
        }
    }

    fn check_pipeline(&mut self, pipeline: &PipelineDecl) {
        let mut saw_encode = false;
        let mut saw_protect = false;
        let mut saw_store = false;
        for step in &pipeline.steps {
            match step {
                PipelineStep::Encode { codec, .. } => {
                    saw_encode = true;
                    if *codec != Codec::Ternary {
                        self.report.warning(format!(
                            "pipeline '{}' uses {} codec; current executable backend maps to Ternary VFS calls",
                            pipeline.name, codec
                        ));
                    }
                }
                PipelineStep::Protect { redundancy } => {
                    saw_protect = true;
                    if *redundancy == 1 {
                        self.report.warning(format!("pipeline '{}' protects with only 1x redundancy", pipeline.name));
                    }
                }
                PipelineStep::Store { pool } => {
                    saw_store = true;
                    if !self.pools.contains_key(pool) {
                        self.report.error(format!("pipeline '{}' stores into undeclared pool '{}'", pipeline.name, pool));
                    }
                }
                PipelineStep::VerifyRoundtrip => {}
            }
        }
        if saw_store && !saw_encode {
            self.report.error(format!("pipeline '{}' stores data before an encode step", pipeline.name));
        }
        if saw_store && !saw_protect {
            self.report.warning(format!("pipeline '{}' stores without an explicit protect step", pipeline.name));
        }
    }
}

fn nuclescript_constraints() -> ConstraintConfig {
    ConstraintConfig {
        gc_min: 0.40,
        gc_max: 0.60,
        max_homopolymer: 3,
        max_palindrome: 6,
        min_strand_length: 150,
        max_strand_length: 200,
        gc_window_size: 20,
    }
}

fn sequence_literal_constraints() -> ConstraintConfig {
    ConstraintConfig {
        gc_min: 0.40,
        gc_max: 0.60,
        max_homopolymer: 3,
        max_palindrome: 8,
        min_strand_length: 1,
        max_strand_length: 500,
        gc_window_size: 20,
    }
}

fn normalize_sequence_literal(sequence: &str) -> Result<String, String> {
    let mut normalized = String::new();
    for ch in sequence.chars() {
        match ch {
            'A' | 'a' => normalized.push('A'),
            'T' | 't' => normalized.push('T'),
            'G' | 'g' => normalized.push('G'),
            'C' | 'c' => normalized.push('C'),
            '-' | '_' | ' ' | '\t' | '\n' | '\r' => {}
            other => return Err(format!("invalid nucleotide character '{}'", other)),
        }
    }
    if normalized.is_empty() {
        return Err("empty sequence literal".into());
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_undeclared_pool() {
        let program = Program {
            declarations: vec![Declaration::Operation(Operation::Store(StoreOp {
                simulate: false,
                file: "a.txt".into(),
                pool: "archive".into(),
                options: StoreOptions::default(),
            }))],
        };
        assert!(check_program(&program).has_errors());
    }
}
