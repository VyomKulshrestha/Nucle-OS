//! Semantic analysis and biological constraint checking for NucleScript.

use crate::ast::*;
use crate::effects::{
    delete_has_required_confirmation, expr_effect, expr_has_required_confirmation, operation_effect,
};
use crate::package::{package_exists, resolve_import};
use crate::probabilistic::{consensus_error_rate_percent, profile_error_rate_percent, ProbPoolType};
use nucle_codec::base::DnaStrand;
use nucle_codec::constraints::{ConstraintConfig, ConstraintValidator};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Default, PartialEq)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    pool_bindings: HashMap<String, ProbPoolType>,
    strands: HashSet<String>,
    sequences: HashSet<String>,
    functions: HashMap<String, FunctionDecl>,
    report: TypeReport,
}

impl TypeChecker {
    fn check(&mut self, program: &Program) {
        for declaration in &program.declarations {
            self.check_declaration_single(declaration);
        }
    }

    fn check_declaration_single(&mut self, declaration: &Declaration) {
        match declaration {
            Declaration::Import(import) => self.check_import(import),
            Declaration::Pool(pool) => self.check_pool(pool),
            Declaration::Strand(strand) => self.check_strand(strand),
            Declaration::Sequence(sequence) => self.check_sequence(sequence),
            Declaration::Let(binding) => self.check_let(binding),
            Declaration::Operation(Operation::Store(store)) => self.check_store(store),
            Declaration::Operation(Operation::Retrieve(retrieve)) => self.check_retrieve(retrieve),
            Declaration::Operation(Operation::Delete(delete)) => self.check_delete(delete),
            Declaration::Pipeline(pipeline) => self.check_pipeline(pipeline),
            Declaration::Function(func) => self.check_function(func),
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
        if pool.codec == Codec::Fountain {
            self.report.warning(format!(
                "pool '{}' declares Fountain codec; the VFS backend only executes Ternary and YinYang end-to-end",
                pool.name
            ));
        }
        self.pools.insert(pool.name.clone(), pool.clone());
    }

    fn check_import(&mut self, import: &ImportDecl) {
        if !package_exists(&import.source) {
            self.report.error(format!("import source '{}' is not available", import.source));
            return;
        }
        for item in &import.items {
            if resolve_import(&import.source, &item.name).is_none() {
                self.report.error(format!(
                    "import '{}' is not exported by '{}'",
                    item.name, import.source
                ));
            }
        }
    }

    fn check_let(&mut self, binding: &LetDecl) {
        if self.pool_bindings.contains_key(&binding.name) || self.pools.contains_key(&binding.name) {
            self.report.error(format!("binding '{}' is declared more than once", binding.name));
            return;
        }

        // Effect/confirmation checking must run regardless of whether the
        // expression produces a Pool type: `infer_expr` returns `None` both
        // for genuine errors (already reported inside it) AND for a
        // perfectly valid call to a Void/DnaFile-returning function — the
        // common shape for a side-effecting function. Bailing out on `None`
        // before this check would silently skip confirmation checking for
        // exactly that case.
        let effect = expr_effect(&binding.expr, &self.functions, &mut std::collections::HashSet::new());
        if !expr_has_required_confirmation(&binding.expr, &self.functions, &mut std::collections::HashSet::new()) {
            self.report.error(format!(
                "binding '{}' has {} effect and requires explicit hardware confirmation",
                binding.name, effect
            ));
        }

        let inferred = match self.infer_expr(&binding.expr) {
            Some(inferred) => inferred,
            None => return,
        };

        match &binding.annotation {
            TypeExpr::Pool(expected) => {
                if expected.state != inferred.state {
                    self.report.error(format!(
                        "binding '{}' is annotated as Pool<{}> but expression produces Pool<{}>",
                        binding.name, expected.state, inferred.state
                    ));
                }
                if let Some(expected_error) = expected.error_rate_percent {
                    let delta = (expected_error - inferred.error_rate_percent).abs();
                    if delta > 0.01 {
                        self.report.error(format!(
                            "binding '{}' is annotated with {:.4}% error but expression infers {:.4}%",
                            binding.name, expected_error, inferred.error_rate_percent
                        ));
                    }
                }
            }
            _ => {}
        }

        self.pool_bindings.insert(binding.name.clone(), inferred);
    }

    fn infer_expr(&mut self, expr: &Expr) -> Option<ProbPoolType> {
        match expr {
            Expr::SimulatePool { pool, profile } => {
                if !self.pools.contains_key(pool) && !self.pool_bindings.contains_key(pool) {
                    self.report.error(format!("simulate source pool '{}' is not declared", pool));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::SynthesizePool { source, profile, .. } => {
                if !self.pools.contains_key(source) && !self.pool_bindings.contains_key(source) {
                    self.report.error(format!("synthesise source pool '{}' is not declared", source));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::SequencePool { source, profile, .. } => {
                if !self.pools.contains_key(source) && !self.pool_bindings.contains_key(source) {
                    self.report.error(format!("sequence source pool '{}' is not declared", source));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::ConsensusVote { source, coverage } => {
                let Some(source_type) = self.pool_bindings.get(source).cloned() else {
                    self.report.error(format!("consensus_vote source '{}' is not a probabilistic pool binding", source));
                    return None;
                };
                if *coverage == 1 {
                    self.report.warning(format!(
                        "consensus_vote on '{}' uses 1x coverage; error budget is unchanged",
                        source
                    ));
                }
                Some(ProbPoolType::new(
                    PoolState::Recovered,
                    consensus_error_rate_percent(source_type.error_rate_percent, *coverage),
                ))
            }
            Expr::FunctionCall { name, args } => {
                let Some(func) = self.functions.get(name).cloned() else {
                    self.report.error(format!("function '{}' is undeclared", name));
                    return None;
                };
                if func.params.len() != args.len() {
                    self.report.error(format!(
                        "function '{}' expects {} arguments, but {} were provided",
                        name, func.params.len(), args.len()
                    ));
                    return None;
                }
                for (param, arg) in func.params.iter().zip(args.iter()) {
                    if let TypeExpr::Pool(expected_pool) = &param.ty {
                        let Some(inferred_arg) = self.infer_expr(arg) else {
                            self.report.error(format!(
                                "argument for parameter '{}' must be a Pool type",
                                param.name
                            ));
                            continue;
                        };
                        if expected_pool.state != inferred_arg.state {
                            self.report.error(format!(
                                "argument for parameter '{}' expects Pool<{}>, but got Pool<{}>",
                                param.name, expected_pool.state, inferred_arg.state
                            ));
                        }
                    }
                }
                match &func.return_type {
                    TypeExpr::Pool(pool_type) => Some(ProbPoolType {
                        state: pool_type.state.clone(),
                        error_rate_percent: pool_type.error_rate_percent.unwrap_or(0.0),
                    }),
                    _ => None,
                }
            }
            Expr::Protect { .. } => None,
            Expr::Variable(name) => {
                if let Some(pool) = self.pool_bindings.get(name).cloned() {
                    Some(pool)
                } else if self.strands.contains(name) || self.sequences.contains(name) || self.pools.contains_key(name) {
                    None
                } else {
                    self.report.error(format!("variable '{}' is undeclared", name));
                    None
                }
            }
            Expr::StringLiteral(_) => None,
        }
    }

    fn check_function(&mut self, func: &FunctionDecl) {
        if self.functions.contains_key(&func.name) {
            self.report.error(format!("function '{}' is declared more than once", func.name));
            return;
        }
        self.functions.insert(func.name.clone(), func.clone());

        let mut param_names = HashSet::new();
        for param in &func.params {
            if !param_names.insert(&param.name) {
                self.report.error(format!("duplicate parameter name '{}' in function '{}'", param.name, func.name));
            }
        }

        let mut body_checker = TypeChecker::default();
        body_checker.pools = self.pools.clone();
        body_checker.functions = self.functions.clone();
        for param in &func.params {
            match &param.ty {
                TypeExpr::Pool(pool_type) => {
                    body_checker.pool_bindings.insert(
                        param.name.clone(),
                        ProbPoolType {
                            state: pool_type.state.clone(),
                            error_rate_percent: pool_type.error_rate_percent.unwrap_or(0.0),
                        },
                    );
                }
                TypeExpr::Sequence => {
                    body_checker.sequences.insert(param.name.clone());
                }
                TypeExpr::Strand => {
                    body_checker.strands.insert(param.name.clone());
                }
                _ => {}
            }
        }

        for decl in &func.body {
            body_checker.check_declaration_single(decl);
        }

        // Return-type validation: the language has no explicit `return`
        // expression (a function body is a statement sequence), so the only
        // well-defined case to check is a `Pool<...>`-returning function
        // whose last statement is a `let` binding producing a pool type —
        // that's the shape every current example uses to "return" a value.
        // A `Void`/`File`/`DnaFile`/etc. return type, or a body ending in a
        // non-`let` statement (e.g. `store ... into ...`), isn't validated:
        // there's no reliable inferred value to compare against without
        // inventing semantics the AST doesn't otherwise support.
        if let TypeExpr::Pool(expected) = &func.return_type {
            match func.body.last() {
                Some(Declaration::Let(last_binding)) => {
                    match body_checker.pool_bindings.get(&last_binding.name) {
                        Some(actual) => {
                            if actual.state != expected.state {
                                self.report.error(format!(
                                    "function '{}' is declared to return Pool<{}> but its body produces Pool<{}>",
                                    func.name, expected.state, actual.state
                                ));
                            }
                        }
                        None => {
                            self.report.error(format!(
                                "function '{}' is declared to return Pool<{}> but its last binding does not produce a pool type",
                                func.name, expected.state
                            ));
                        }
                    }
                }
                _ => {
                    self.report.error(format!(
                        "function '{}' is declared to return Pool<{}> but its body does not end in a binding that produces one",
                        func.name, expected.state
                    ));
                }
            }
        }

        self.report.diagnostics.extend(body_checker.report.diagnostics);
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
        let effect = operation_effect(&Operation::Store(store.clone()));
        if effect == Effect::Synthesis && !store.simulate {
            self.report.warning(format!(
                "store '{}' has Synthesis effect; use simulate store for no-hardware execution",
                store.file
            ));
        }
        let pool = if let Some(pool) = self.pools.get(&store.pool).cloned() {
            pool
        } else if let Some(binding) = self.pool_bindings.get(&store.pool).cloned() {
            let profile = match binding.state {
                PoolState::Profile(p) => p,
                _ => Profile::Illumina,
            };
            PoolDecl {
                name: store.pool.clone(),
                codec: Codec::Ternary,
                redundancy: store.options.redundancy.unwrap_or(2),
                profile,
            }
        } else {
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

    fn check_delete(&mut self, delete: &DeleteOp) {
        if !self.pools.contains_key(&delete.pool) && !self.pool_bindings.contains_key(&delete.pool) {
            self.report.error(format!("delete target pool '{}' is not declared", delete.pool));
        }
        if !delete_has_required_confirmation(delete) {
            self.report.error(format!(
                "delete '{}' from '{}' has Destructive effect and requires explicit physical key confirmation",
                delete.file, delete.pool
            ));
        }
    }

    fn check_retrieve(&mut self, retrieve: &RetrieveOp) {
        if !self.pools.contains_key(&retrieve.pool) && !self.pool_bindings.contains_key(&retrieve.pool) {
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
                    if *codec == Codec::Fountain {
                        self.report.warning(format!(
                            "pipeline '{}' uses Fountain codec; the VFS backend only executes Ternary and YinYang end-to-end",
                            pipeline.name
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

    #[test]
    fn accepts_probabilistic_pool_flow() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Illumina,
                }),
                Declaration::Let(LetDecl {
                    name: "noisy".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Profile(Profile::Illumina),
                        error_rate_percent: Some(0.35),
                    }),
                    expr: Expr::SimulatePool {
                        pool: "archive".into(),
                        profile: Profile::Illumina,
                    },
                }),
                Declaration::Let(LetDecl {
                    name: "recovered".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Recovered,
                        error_rate_percent: None,
                    }),
                    expr: Expr::ConsensusVote {
                        source: "noisy".into(),
                        coverage: 10,
                    },
                }),
            ],
        };
        assert!(!check_program(&program).has_errors());
    }

    #[test]
    fn rejects_wrong_probabilistic_error_annotation() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Illumina,
                }),
                Declaration::Let(LetDecl {
                    name: "noisy".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Profile(Profile::Illumina),
                        error_rate_percent: Some(9.0),
                    }),
                    expr: Expr::SimulatePool {
                        pool: "archive".into(),
                        profile: Profile::Illumina,
                    },
                }),
            ],
        };
        assert!(check_program(&program).has_errors());
    }

    #[test]
    fn rejects_unconfirmed_hardware_effect() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Twist,
                }),
                Declaration::Let(LetDecl {
                    name: "strands".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Profile(Profile::Twist),
                        error_rate_percent: Some(0.03),
                    }),
                    expr: Expr::SynthesizePool {
                        source: "archive".into(),
                        profile: Profile::Twist,
                        confirmed: false,
                    },
                }),
            ],
        };
        assert!(check_program(&program).has_errors());
    }

    #[test]
    fn accepts_confirmed_effects() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Twist,
                }),
                Declaration::Let(LetDecl {
                    name: "strands".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Profile(Profile::Twist),
                        error_rate_percent: Some(0.03),
                    }),
                    expr: Expr::SynthesizePool {
                        source: "archive".into(),
                        profile: Profile::Twist,
                        confirmed: true,
                    },
                }),
                Declaration::Operation(Operation::Delete(DeleteOp {
                    file: "old.bin".into(),
                    pool: "archive".into(),
                    confirmed: true,
                })),
            ],
        };
        assert!(!check_program(&program).has_errors());
    }

    #[test]
    fn rejects_unconfirmed_destructive_effect() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Illumina,
                }),
                Declaration::Operation(Operation::Delete(DeleteOp {
                    file: "old.bin".into(),
                    pool: "archive".into(),
                    confirmed: false,
                })),
            ],
        };
        assert!(check_program(&program).has_errors());
    }

    #[test]
    fn validates_package_imports() {
        let program = Program {
            declarations: vec![Declaration::Import(ImportDecl {
                source: "nuclescript/presets".into(),
                items: vec![ImportItem {
                    name: "medical_archive".into(),
                    alias: None,
                }],
            })],
        };
        assert!(!check_program(&program).has_errors());
    }

    #[test]
    fn rejects_unknown_package_imports() {
        let program = Program {
            declarations: vec![Declaration::Import(ImportDecl {
                source: "nuclescript/presets".into(),
                items: vec![ImportItem {
                    name: "missing".into(),
                    alias: None,
                }],
            })],
        };
        assert!(check_program(&program).has_errors());
    }
}
