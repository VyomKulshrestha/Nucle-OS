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
    /// Record an error at `span` with a stable `code` -- every check site
    /// has some declaration or operation in scope to take a span from, and
    /// a fixed mnemonic identifying which check fired, so this is the only
    /// entry point: there is no fallback that omits either, on purpose, so
    /// a new check can't silently ship without something `docs/errors.md`
    /// and an editor can link to.
    pub fn error(&mut self, span: Span, code: &'static str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { level: DiagnosticLevel::Error, code: code.to_string(), message: message.into(), span });
    }

    pub fn warning(&mut self, span: Span, code: &'static str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { level: DiagnosticLevel::Warning, code: code.to_string(), message: message.into(), span });
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
            writeln!(f, "{}:{}: {} [{}]: {}", diagnostic.span.line, diagnostic.span.column, diagnostic.level, diagnostic.code, diagnostic.message)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    #[serde(default = "default_code")]
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub span: Span,
}

fn default_code() -> String {
    "E-UNKNOWN".to_string()
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
    check_and_desugar(program).0
}

/// Type-checks `program` and resolves every `if`/`for` away, producing a
/// plain, control-flow-free `Program` -- `codegen`/`middle`/`sim_backend`
/// consume this output, never the original `program`, so they never need
/// to know `Declaration::If`/`Declaration::For` exist. See `IfDecl`'s and
/// `ForDecl`'s doc comments in `ast.rs` for why this is a compile-time
/// desugaring pass rather than runtime branching.
pub fn check_and_desugar(program: &Program) -> (TypeReport, Program) {
    let mut checker = TypeChecker::default();
    let desugared = checker.check(program);
    (checker.report, desugared)
}

/// Same check as `check_program`, but also returns every top-level
/// declaration's name/span, keyed by kind -- for tooling (the language
/// server's hover/go-to-definition/document-symbol) that needs "where is
/// X defined" without re-deriving the same scope-tracking `TypeChecker`
/// already does during the single real check pass. Function-body-local
/// bindings/parameters aren't included -- only top-level symbols, which
/// covers the common "jump to this pool/function/strand/sequence" case
/// without a second, per-scope symbol table design.
pub fn check_program_with_symbols(program: &Program) -> (TypeReport, SymbolTable) {
    let mut checker = TypeChecker::default();
    let _ = checker.check(program);
    let symbols = SymbolTable {
        pools: checker.pools,
        functions: checker.functions,
        strands: checker.strands,
        sequences: checker.sequences,
        bindings: checker.bindings,
    };
    (checker.report, symbols)
}

/// Every top-level symbol a program declares, with enough of each
/// declaration to answer "what is this" (hover) and "where is it defined"
/// (go-to-definition/document outline) -- see `check_program_with_symbols`.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    pub pools: HashMap<String, PoolDecl>,
    pub functions: HashMap<String, FunctionDecl>,
    pub strands: HashMap<String, Span>,
    pub sequences: HashMap<String, Span>,
    pub bindings: HashMap<String, LetDecl>,
}

#[derive(Default)]
struct TypeChecker {
    pools: HashMap<String, PoolDecl>,
    pool_bindings: HashMap<String, ProbPoolType>,
    bindings: HashMap<String, LetDecl>,
    strands: HashMap<String, Span>,
    sequences: HashMap<String, Span>,
    functions: HashMap<String, FunctionDecl>,
    /// Bindings whose inferred type is `Result<Ok, Err>` -- parallel to
    /// `pool_bindings`, but for the structurally different "this is a
    /// Result, not a pool" shape `infer_expr` has no room for (see
    /// `infer_result_expr`).
    result_bindings: HashMap<String, (TypeExpr, TypeExpr)>,
    /// The `(Ok, Err)` pair of the function currently being type-checked,
    /// if it declares a `Result<...>` return type -- `None` at top level
    /// (a `?` outside any function) and for a non-`Result`-returning
    /// function, both of which make `?` invalid. Set once in
    /// `check_function` before its body is checked; never mutated by
    /// nested constructs (there's no lambda/closure to rebind it).
    enclosing_result_return: Option<(TypeExpr, TypeExpr)>,
    report: TypeReport,
}

impl TypeChecker {
    /// Type-checks every declaration and returns the desugared program:
    /// each `Declaration::If`/`Declaration::For` is replaced by the
    /// declarations of its resolved branch/unrolled iterations, so the
    /// result never contains either variant.
    fn check(&mut self, program: &Program) -> Program {
        let mut declarations = Vec::new();
        for declaration in &program.declarations {
            declarations.extend(self.check_declaration_single(declaration));
        }
        Program { declarations }
    }

    /// Checks one declaration and returns its desugared form: every
    /// variant except `If`/`For` type-checks itself and passes through
    /// unchanged (as a one-element `Vec`); `If`/`For` expand into zero or
    /// more declarations from their resolved branch/unrolled body. A
    /// nested `Function`'s body is recursively desugared the same way, so
    /// control flow inside a function body is resolved too.
    fn check_declaration_single(&mut self, declaration: &Declaration) -> Vec<Declaration> {
        match declaration {
            Declaration::Import(import) => {
                self.check_import(import);
                vec![declaration.clone()]
            }
            Declaration::Pool(pool) => {
                self.check_pool(pool);
                vec![declaration.clone()]
            }
            Declaration::Strand(strand) => {
                self.check_strand(strand);
                vec![declaration.clone()]
            }
            Declaration::Sequence(sequence) => {
                self.check_sequence(sequence);
                vec![declaration.clone()]
            }
            Declaration::Let(binding) => {
                self.check_let(binding);
                vec![declaration.clone()]
            }
            Declaration::Operation(Operation::Store(store)) => {
                self.check_store(store);
                vec![declaration.clone()]
            }
            Declaration::Operation(Operation::Retrieve(retrieve)) => {
                self.check_retrieve(retrieve);
                vec![declaration.clone()]
            }
            Declaration::Operation(Operation::Delete(delete)) => {
                self.check_delete(delete);
                vec![declaration.clone()]
            }
            Declaration::Operation(Operation::Assert(assert)) => {
                self.check_assert(assert);
                vec![declaration.clone()]
            }
            Declaration::Pipeline(pipeline) => {
                self.check_pipeline(pipeline);
                vec![declaration.clone()]
            }
            Declaration::Function(func) => vec![Declaration::Function(self.check_function(func))],
            Declaration::If(if_decl) => self.check_if(if_decl),
            Declaration::For(for_decl) => self.check_for(for_decl),
            Declaration::Test(test) => vec![Declaration::Test(self.check_test(test))],
        }
    }

    /// `assert <condition>` -- evaluated with the exact same
    /// `eval_condition` machinery an `if` condition uses (see
    /// `TestDecl`'s doc comment in `ast.rs` for why this is the right
    /// place, not a deferred runtime check). A false condition reports
    /// `E-ASSERTION-FAILED` with the custom message if one was given,
    /// else the condition can't speak for itself the way a hand-written
    /// message can -- there's no expression-to-source-text printer in
    /// this compiler, so an unmessaged failing assertion just says which
    /// one, by span, not what it compared.
    fn check_assert(&mut self, assert: &AssertOp) {
        match self.eval_condition(&assert.condition, assert.span, "assert") {
            Some(true) | None => {}
            Some(false) => {
                let message = assert.message.clone().unwrap_or_else(|| "assertion failed".to_string());
                self.report.error(assert.span, "E-ASSERTION-FAILED", message);
            }
        }
    }

    /// Type-checks a test body in a scope that inherits the enclosing
    /// program's pools/functions (so a test can exercise real pool
    /// schemas and helper functions) but starts with fresh bindings (so
    /// tests don't share simulated state with each other) -- the same
    /// isolation `check_function` gives a function body, for the same
    /// reason: independent tests shouldn't be able to see each other's
    /// local `let`s.
    fn check_test(&mut self, test: &TestDecl) -> TestDecl {
        let mut body_checker = TypeChecker::default();
        body_checker.pools = self.pools.clone();
        body_checker.functions = self.functions.clone();

        let mut desugared_body = Vec::new();
        for decl in &test.body {
            desugared_body.extend(body_checker.check_declaration_single(decl));
        }
        self.report.diagnostics.extend(body_checker.report.diagnostics);

        TestDecl { name: test.name.clone(), body: desugared_body, span: test.span }
    }

    /// Evaluates `condition` at compile time and type-checks only the
    /// taken branch (the untaken branch is never checked at all, matching
    /// `#[cfg(...)]` more than a runtime `if` -- see `IfDecl`'s doc
    /// comment). Returns that branch's desugared declarations, or an empty
    /// `Vec` if `condition` couldn't be evaluated (the error is already
    /// recorded by `eval_condition`).
    fn check_if(&mut self, if_decl: &IfDecl) -> Vec<Declaration> {
        let Some(taken) = self.eval_condition(&if_decl.condition, if_decl.span, "if") else {
            return Vec::new();
        };
        let branch: &[Declaration] = if taken {
            &if_decl.then_branch
        } else {
            if_decl.else_branch.as_deref().unwrap_or(&[])
        };
        let mut out = Vec::new();
        for decl in branch {
            out.extend(self.check_declaration_single(decl));
        }
        out
    }

    /// Unrolls the loop by substituting `binding` with each item's literal
    /// value in a fresh copy of `body`, type-checking each copy
    /// independently -- see `ForDecl`'s doc comment.
    fn check_for(&mut self, for_decl: &ForDecl) -> Vec<Declaration> {
        let mut out = Vec::new();
        for item in &for_decl.items {
            for body_decl in &for_decl.body {
                let substituted = substitute_declaration(body_decl, &for_decl.binding, item);
                out.extend(self.check_declaration_single(&substituted));
            }
        }
        out
    }

    /// Resolves a numeric operand in an `if`/`assert` condition: either a
    /// literal number, or (the deliberate coercion this design relies on
    /// so a condition can inspect "the pool's observed error rate"
    /// without inventing general field-access syntax) a probabilistic
    /// pool binding's name, which resolves to its inferred
    /// `error_rate_percent`. `context` (`"if"`/`"assert"`) only affects
    /// error message wording -- both call sites share this one
    /// evaluator, per Step 7's reuse of Step 4's comparison operators
    /// rather than a separate assertion DSL.
    fn eval_numeric(&mut self, expr: &Expr, span: Span, context: &str) -> Option<f64> {
        match expr {
            Expr::Number(value) => Some(*value),
            Expr::Variable(name) => {
                if let Some(pool) = self.pool_bindings.get(name) {
                    Some(pool.error_rate_percent)
                } else {
                    let suggestion = self.suggest_pool_name(name);
                    self.report.error(span, "E-CONDITION-UNDECLARED", format!(
                        "{} condition references undeclared probabilistic pool binding '{}'{}",
                        context, name, did_you_mean(suggestion)
                    ));
                    None
                }
            }
            _ => {
                self.report.error(span, "E-CONDITION-NOT-NUMERIC", format!("expected a number or a probabilistic pool binding's error rate in this {} condition", context));
                None
            }
        }
    }

    /// Evaluates a loop-free boolean expression to a concrete `bool` at
    /// type-check time -- shared by `if`'s condition and `assert`'s
    /// condition (see `AssertOp`'s doc comment for why an assertion is
    /// evaluated here rather than deferred to a later "runtime" phase).
    /// `condition` must reduce entirely to comparisons/`&&`/`||`/`!` over
    /// numbers and pool bindings, since there is no runtime to defer
    /// evaluation to.
    fn eval_condition(&mut self, expr: &Expr, span: Span, context: &str) -> Option<bool> {
        match expr {
            Expr::BinaryOp { op: BinOp::And, left, right } => {
                let left = self.eval_condition(left, span, context);
                let right = self.eval_condition(right, span, context);
                Some(left? && right?)
            }
            Expr::BinaryOp { op: BinOp::Or, left, right } => {
                let left = self.eval_condition(left, span, context);
                let right = self.eval_condition(right, span, context);
                Some(left? || right?)
            }
            Expr::BinaryOp { op, left, right } => {
                let left = self.eval_numeric(left, span, context);
                let right = self.eval_numeric(right, span, context);
                let (left, right) = (left?, right?);
                Some(match op {
                    BinOp::Eq => (left - right).abs() < f64::EPSILON,
                    BinOp::Ne => (left - right).abs() >= f64::EPSILON,
                    BinOp::Lt => left < right,
                    BinOp::Gt => left > right,
                    BinOp::Le => left <= right,
                    BinOp::Ge => left >= right,
                    BinOp::And | BinOp::Or => unreachable!("handled above"),
                })
            }
            Expr::Not(inner) => self.eval_condition(inner, span, context).map(|value| !value),
            _ => {
                self.report.error(span, "E-CONDITION-NOT-BOOLEAN", format!("{} condition must be a comparison, or a boolean combination of comparisons using && / || / !", context));
                None
            }
        }
    }

    fn check_pool(&mut self, pool: &PoolDecl) {
        if self.pools.contains_key(&pool.name) {
            self.report.error(pool.span, "E-POOL-DUPLICATE", format!("pool '{}' is declared more than once", pool.name));
            return;
        }
        if pool.redundancy == 1 {
            self.report.warning(pool.span, "E-POOL-LOW-REDUNDANCY", format!(
                "pool '{}' has 1x redundancy; critical files should use at least 2x",
                pool.name
            ));
        }
        if pool.codec == Codec::Fountain {
            self.report.warning(pool.span, "E-POOL-UNSUPPORTED-CODEC", format!(
                "pool '{}' declares Fountain codec; the VFS backend only executes Ternary and YinYang end-to-end",
                pool.name
            ));
        }
        self.pools.insert(pool.name.clone(), pool.clone());
    }

    fn check_import(&mut self, import: &ImportDecl) {
        if !package_exists(&import.source) {
            self.report.error(import.span, "E-IMPORT-UNKNOWN-SOURCE", format!("import source '{}' is not available", import.source));
            return;
        }
        for item in &import.items {
            if resolve_import(&import.source, &item.name).is_none() {
                let candidates = crate::package::exported_names(&import.source);
                let suggestion = suggest_name(&item.name, &candidates);
                self.report.error(import.span, "E-IMPORT-UNKNOWN-ITEM", format!(
                    "import '{}' is not exported by '{}'{}",
                    item.name, import.source, did_you_mean(suggestion)
                ));
            }
        }
    }

    fn check_let(&mut self, binding: &LetDecl) {
        if self.pool_bindings.contains_key(&binding.name)
            || self.pools.contains_key(&binding.name)
            || self.result_bindings.contains_key(&binding.name)
        {
            self.report.error(binding.span, "E-BINDING-DUPLICATE", format!("binding '{}' is declared more than once", binding.name));
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
            self.report.error(binding.span, "E-SYNTHESIS-UNCONFIRMED", format!(
                "binding '{}' has {} effect and requires explicit hardware confirmation",
                binding.name, effect
            ));
        }

        // `let x: T = <fallible>?` -- the unwrap form. Checked before
        // anything else since `?`'s own validity (is the inner expression
        // really Result-shaped? is there an enclosing Result-returning
        // function? does the Err type match it?) is entirely orthogonal to
        // whatever T is.
        if let Expr::Try(inner) = &binding.expr {
            if let Some(ok_ty) = self.check_try(inner, binding.span) {
                if ok_ty != binding.annotation {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but '?' unwraps to {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&ok_ty)
                    ));
                } else {
                    self.bindings.insert(binding.name.clone(), binding.clone());
                }
            }
            return;
        }

        // `let x: Result<T, E> = store ... into ...` (or a variable/call
        // already known to be Result-shaped) -- the un-unwrapped form.
        // Checked before the Pool-shaped path below so a Result-producing
        // expression bound to a non-Result annotation (a `?` was likely
        // forgotten) is a clear error instead of `infer_expr` silently
        // returning `None` and this function just giving up.
        if let Some((ok, err)) = self.infer_result_expr(&binding.expr) {
            match &binding.annotation {
                TypeExpr::Result(expected_ok, expected_err) => {
                    if &ok != expected_ok.as_ref() || &err != expected_err.as_ref() {
                        self.report.error(binding.span, "E-BINDING-RESULT-TYPE-MISMATCH", format!(
                            "binding '{}' is annotated as Result<{}, {}> but expression produces Result<{}, {}>",
                            binding.name,
                            crate::docgen::render_type(expected_ok), crate::docgen::render_type(expected_err),
                            crate::docgen::render_type(&ok), crate::docgen::render_type(&err),
                        ));
                    } else {
                        self.result_bindings.insert(binding.name.clone(), (ok, err));
                        self.bindings.insert(binding.name.clone(), binding.clone());
                    }
                }
                _ => {
                    self.report.error(binding.span, "E-BINDING-RESULT-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but the expression produces Result<{}, {}> -- use '?' to unwrap it, or annotate the binding as Result<{}, {}>",
                        binding.name, crate::docgen::render_type(&binding.annotation),
                        crate::docgen::render_type(&ok), crate::docgen::render_type(&err),
                        crate::docgen::render_type(&ok), crate::docgen::render_type(&err),
                    ));
                }
            }
            return;
        }

        let inferred = match self.infer_expr(&binding.expr, binding.span) {
            Some(inferred) => inferred,
            None => return,
        };

        match &binding.annotation {
            TypeExpr::Pool(expected) => {
                if expected.state != inferred.state {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as Pool<{}> but expression produces Pool<{}>",
                        binding.name, expected.state, inferred.state
                    ));
                }
                if let Some(expected_error) = expected.error_rate_percent {
                    let delta = (expected_error - inferred.error_rate_percent).abs();
                    if delta > 0.01 {
                        self.report.error(binding.span, "E-BINDING-ERROR-RATE-MISMATCH", format!(
                            "binding '{}' is annotated with {:.4}% error but expression infers {:.4}%",
                            binding.name, expected_error, inferred.error_rate_percent
                        ));
                    }
                }
            }
            _ => {}
        }

        self.pool_bindings.insert(binding.name.clone(), inferred);
        self.bindings.insert(binding.name.clone(), binding.clone());
    }

    /// Infers the `(Ok, Err)` type pair of a `Result`-shaped expression --
    /// `StoreExpr`/`DeleteExpr` (always `(DnaFile, Str)`/`(Void, Str)`), a
    /// call to a `Result`-returning function, or a variable already bound
    /// to one. Returns `None` for anything that isn't `Result`-shaped,
    /// including `RetrieveExpr` (retrieve has no real failure mode today
    /// -- see its doc comment in `ast.rs`) and `Expr::Try` (unwrapping
    /// removes the `Result` wrapper entirely; `?` is handled at its use
    /// site by `check_try` instead of through this function).
    fn infer_result_expr(&mut self, expr: &Expr) -> Option<(TypeExpr, TypeExpr)> {
        match expr {
            // `check_store`/`check_delete`/`check_retrieve` are the SAME
            // validation the statement form already runs (pool declared?
            // confirmed? sane redundancy/coverage?) -- called here as a
            // side effect so the expression-position surface form can't
            // silently skip checks the statement form always runs, just
            // because this function's job is normally type inference, not
            // validation. `RetrieveExpr` still returns `None` (it's never
            // Result-shaped -- see its doc comment in ast.rs), but its
            // pool/query validation must still happen somewhere, and this
            // is the one place every reachable occurrence passes through.
            Expr::StoreExpr(op) => {
                self.check_store(op);
                Some((TypeExpr::DnaFile, TypeExpr::Str))
            }
            Expr::DeleteExpr(op) => {
                self.check_delete(op);
                Some((TypeExpr::Void, TypeExpr::Str))
            }
            Expr::RetrieveExpr(op) => {
                self.check_retrieve(op);
                None
            }
            Expr::Variable(name) => self.result_bindings.get(name).cloned(),
            Expr::FunctionCall { name, .. } => match self.lookup_function(name)?.return_type {
                TypeExpr::Result(ok, err) => Some((*ok, *err)),
                _ => None,
            },
            _ => None,
        }
    }

    /// `expr?`'s validity: `expr` must itself be `Result`-shaped
    /// (`E-TRY-NOT-RESULT` if not), there must be an enclosing function
    /// declared to return `Result<_, E>` (`E-TRY-OUTSIDE-RESULT-FN` if
    /// not), and its `Err` type must exactly match that function's
    /// declared `Err` type -- no coercion, matching this project's
    /// existing avoidance of implicit conversions (`E-TRY-ERROR-TYPE-
    /// MISMATCH` if not). Returns the unwrapped `Ok` type on success.
    fn check_try(&mut self, inner: &Expr, span: Span) -> Option<TypeExpr> {
        let Some((ok_ty, err_ty)) = self.infer_result_expr(inner) else {
            self.report.error(span, "E-TRY-NOT-RESULT", "'?' can only be applied to a Result<T, E>-typed expression");
            return None;
        };
        let Some((_, enclosing_err)) = self.enclosing_result_return.clone() else {
            self.report.error(span, "E-TRY-OUTSIDE-RESULT-FN", "'?' can only be used inside a function whose return type is Result<T, E>");
            return None;
        };
        if err_ty != enclosing_err {
            self.report.error(span, "E-TRY-ERROR-TYPE-MISMATCH", format!(
                "'?' propagates an Err({}) but the enclosing function returns Result<_, {}>",
                crate::docgen::render_type(&err_ty), crate::docgen::render_type(&enclosing_err),
            ));
            return None;
        }
        Some(ok_ty)
    }

    /// `span` is the enclosing declaration's span (a `let` binding, or a
    /// function-call argument's own enclosing binding when called
    /// recursively) -- `Expr` itself carries no span of its own yet, so
    /// this is the finest location available for errors found while
    /// inferring an expression's type.
    fn infer_expr(&mut self, expr: &Expr, span: Span) -> Option<ProbPoolType> {
        match expr {
            Expr::SimulatePool { pool, profile } => {
                if !self.pools.contains_key(pool) && !self.pool_bindings.contains_key(pool) {
                    let suggestion = self.suggest_pool_name(pool);
                    self.report.error(span, "E-POOL-UNDECLARED", format!("simulate source pool '{}' is not declared{}", pool, did_you_mean(suggestion)));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::SynthesizePool { source, profile, .. } => {
                if !self.pools.contains_key(source) && !self.pool_bindings.contains_key(source) {
                    let suggestion = self.suggest_pool_name(source);
                    self.report.error(span, "E-POOL-UNDECLARED", format!("synthesise source pool '{}' is not declared{}", source, did_you_mean(suggestion)));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::SequencePool { source, profile, .. } => {
                if !self.pools.contains_key(source) && !self.pool_bindings.contains_key(source) {
                    let suggestion = self.suggest_pool_name(source);
                    self.report.error(span, "E-POOL-UNDECLARED", format!("sequence source pool '{}' is not declared{}", source, did_you_mean(suggestion)));
                    return None;
                }
                Some(ProbPoolType::new(
                    PoolState::Profile(*profile),
                    profile_error_rate_percent(*profile),
                ))
            }
            Expr::FunctionCall { name, args } => {
                let Some(func) = self.lookup_function(name) else {
                    let candidates = self.function_name_candidates();
                    let suggestion = suggest_name(name, &candidates);
                    self.report.error(span, "E-FUNCTION-UNDECLARED", format!("function '{}' is undeclared{}", name, did_you_mean(suggestion)));
                    return None;
                };
                if func.params.len() != args.len() {
                    self.report.error(span, "E-FUNCTION-ARITY", format!(
                        "function '{}' expects {} arguments, but {} were provided",
                        name, func.params.len(), args.len()
                    ));
                    return None;
                }
                for (param, arg) in func.params.iter().zip(args.iter()) {
                    if let TypeExpr::Pool(expected_pool) = &param.ty {
                        let Some(inferred_arg) = self.infer_expr(arg, span) else {
                            self.report.error(span, "E-ARG-TYPE-INVALID", format!(
                                "argument for parameter '{}' must be a Pool type",
                                param.name
                            ));
                            continue;
                        };
                        if expected_pool.state != inferred_arg.state {
                            self.report.error(span, "E-ARG-TYPE-MISMATCH", format!(
                                "argument for parameter '{}' expects Pool<{}>, but got Pool<{}>",
                                param.name, expected_pool.state, inferred_arg.state
                            ));
                        }
                    }
                }

                // `consensus_vote`'s result genuinely depends on its
                // *argument values* (the source binding's inferred error
                // rate and the requested coverage), not just a fixed
                // declared signature -- arity and effect classification
                // for it already went through the exact same
                // `FunctionTable`-based machinery above as any user
                // function (see `stdlib::builtin_functions`), but this
                // one intrinsic-recognition-by-name branch is still
                // needed for its actual return type, the same way a real
                // compiler special-cases a handful of true intrinsics
                // rather than pretending every one of them fits an
                // ordinary statically-typed function signature.
                if name == "consensus_vote" {
                    return self.infer_consensus_vote(args, span);
                }

                match &func.return_type {
                    TypeExpr::Pool(pool_type) => Some(ProbPoolType {
                        state: pool_type.state.clone(),
                        error_rate_percent: pool_type.error_rate_percent.unwrap_or(0.0),
                    }),
                    _ => None,
                }
            }
            Expr::Variable(name) => {
                if let Some(pool) = self.pool_bindings.get(name).cloned() {
                    Some(pool)
                } else if self.strands.contains_key(name) || self.sequences.contains_key(name) || self.pools.contains_key(name) {
                    None
                } else {
                    let suggestion = self.suggest_pool_name(name);
                    self.report.error(span, "E-VARIABLE-UNDECLARED", format!("variable '{}' is undeclared{}", name, did_you_mean(suggestion)));
                    None
                }
            }
            // These never produce a `Pool<...>` value -- they only ever
            // appear as the plain string/boolean/numeric operands of an
            // `if` condition (see `eval_condition`/`eval_numeric`), which
            // is evaluated separately and doesn't route through here.
            Expr::StringLiteral(_) | Expr::Number(_) | Expr::BinaryOp { .. } | Expr::Not(_) => None,
            // Result<T,E>-shaped, not Pool<...>-shaped -- see
            // `infer_result_expr` for their actual (Ok, Err) inference.
            Expr::Try(_) | Expr::StoreExpr(_) | Expr::RetrieveExpr(_) | Expr::DeleteExpr(_) => None,
        }
    }

    /// "did you mean X?" candidate for an unresolved pool/variable name --
    /// pools, pool bindings, strands, and sequences are all valid targets
    /// for `Expr::Variable`/pool-source references, so the suggestion
    /// pool spans all of them, not just one namespace.
    fn suggest_pool_name(&self, target: &str) -> Option<String> {
        let candidates: Vec<String> = self.pools.keys()
            .chain(self.pool_bindings.keys())
            .chain(self.strands.keys())
            .chain(self.sequences.keys())
            .cloned()
            .collect();
        suggest_name(target, &candidates)
    }

    /// Resolves a call target against user-declared functions first, then
    /// falls back to the built-ins (`stdlib::builtin_functions`) -- a
    /// program redeclaring a built-in's name shadows it, the same
    /// precedence `effects::function_table` gives it.
    fn lookup_function(&self, name: &str) -> Option<FunctionDecl> {
        self.functions.get(name).cloned().or_else(|| crate::stdlib::builtin_functions().get(name).cloned())
    }

    /// Every name `Expr::FunctionCall` could resolve to -- user-declared
    /// functions plus built-ins -- for "did you mean X?" suggestions on
    /// an undeclared/typo'd call.
    fn function_name_candidates(&self) -> Vec<String> {
        self.functions.keys().cloned().chain(crate::stdlib::builtin_functions().into_keys()).collect()
    }

    /// `consensus_vote(source, coverage)`'s intrinsic return-type
    /// computation -- see the call site in `infer_expr` for why this
    /// can't just be `func.return_type`. Both arguments are exactly what
    /// the parser's `consensus_vote(...)` desugaring always produces
    /// (`Expr::Variable`, `Expr::Number`); the fallback error branches
    /// below only matter if that invariant is ever violated (e.g. by a
    /// future change letting `consensus_vote` be called via the general
    /// `name(args...)` syntax with arbitrary expressions), so they report
    /// a clear diagnostic instead of panicking rather than because
    /// they're expected to fire today.
    fn infer_consensus_vote(&mut self, args: &[Expr], span: Span) -> Option<ProbPoolType> {
        let source_name = match args.first() {
            Some(Expr::Variable(name)) => name.clone(),
            _ => {
                self.report.error(span, "E-CONSENSUS-INVALID-SOURCE", "consensus_vote's first argument must be a probabilistic pool binding");
                return None;
            }
        };
        let coverage = match args.get(1) {
            Some(Expr::Number(value)) => *value as usize,
            _ => {
                self.report.error(span, "E-CONSENSUS-INVALID-COVERAGE", "consensus_vote's second argument must be a coverage number");
                return None;
            }
        };
        let Some(source_type) = self.pool_bindings.get(&source_name).cloned() else {
            self.report.error(span, "E-CONSENSUS-INVALID-SOURCE", format!("consensus_vote source '{}' is not a probabilistic pool binding", source_name));
            return None;
        };
        if coverage == 1 {
            self.report.warning(span, "E-CONSENSUS-NOOP-COVERAGE", format!(
                "consensus_vote on '{}' uses 1x coverage; error budget is unchanged",
                source_name
            ));
        }
        Some(ProbPoolType::new(PoolState::Recovered, consensus_error_rate_percent(source_type.error_rate_percent, coverage)))
    }

    fn check_function(&mut self, func: &FunctionDecl) -> FunctionDecl {
        if self.functions.contains_key(&func.name) {
            self.report.error(func.span, "E-FUNCTION-DUPLICATE", format!("function '{}' is declared more than once", func.name));
            return func.clone();
        }
        self.functions.insert(func.name.clone(), func.clone());

        let mut param_names = HashSet::new();
        for param in &func.params {
            if !param_names.insert(&param.name) {
                self.report.error(func.span, "E-PARAM-DUPLICATE", format!("duplicate parameter name '{}' in function '{}'", param.name, func.name));
            }
        }

        let mut body_checker = TypeChecker::default();
        body_checker.pools = self.pools.clone();
        body_checker.functions = self.functions.clone();
        // Must be set before the body is checked below (not after): it's
        // what `check_try` (reached through `check_let`, reached through
        // `check_declaration_single`) reads to validate every `?` inside
        // this function's body.
        if let TypeExpr::Result(ok, err) = &func.return_type {
            body_checker.enclosing_result_return = Some(((**ok).clone(), (**err).clone()));
        }
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
                    // A parameter has no declaration span of its own (`FnParam`
                    // doesn't carry one) -- point at the enclosing function's
                    // span as the closest available "where is this defined".
                    body_checker.sequences.insert(param.name.clone(), func.span);
                }
                TypeExpr::Strand => {
                    body_checker.strands.insert(param.name.clone(), func.span);
                }
                _ => {}
            }
        }

        let mut desugared_body = Vec::new();
        for decl in &func.body {
            desugared_body.extend(body_checker.check_declaration_single(decl));
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
            match desugared_body.last() {
                Some(Declaration::Let(last_binding)) => {
                    match body_checker.pool_bindings.get(&last_binding.name) {
                        Some(actual) => {
                            if actual.state != expected.state {
                                self.report.error(last_binding.span, "E-RETURN-TYPE-MISMATCH", format!(
                                    "function '{}' is declared to return Pool<{}> but its body produces Pool<{}>",
                                    func.name, expected.state, actual.state
                                ));
                            }
                        }
                        None => {
                            self.report.error(last_binding.span, "E-RETURN-TYPE-MISMATCH", format!(
                                "function '{}' is declared to return Pool<{}> but its last binding does not produce a pool type",
                                func.name, expected.state
                            ));
                        }
                    }
                }
                _ => {
                    self.report.error(func.span, "E-RETURN-TYPE-MISMATCH", format!(
                        "function '{}' is declared to return Pool<{}> but its body does not end in a binding that produces one",
                        func.name, expected.state
                    ));
                }
            }
        }

        // Same idea as the `Pool<...>` case above, extended to `Result<T,
        // E>`: the body's last `let` is the implicit return. Two valid
        // shapes -- still-wrapped (`let x: Result<T,E> = store f into p`,
        // no `?`) or already-unwrapped via `?` (`let x: T = <fallible>?`,
        // the actual acceptance-example shape) -- since a Result-returning
        // function auto-wraps a successful unwrapped tail into `Ok(...)`
        // at the call boundary (see `codegen::call_user_function`).
        if let TypeExpr::Result(expected_ok, expected_err) = &func.return_type {
            match desugared_body.last() {
                Some(Declaration::Let(last_binding)) if matches!(last_binding.expr, Expr::Try(_)) => {
                    // The inner fallible expression's Err type was already
                    // checked against `expected_err` by `check_try` (via
                    // `enclosing_result_return`, set above) while checking
                    // this binding -- only the Ok side is left to verify
                    // here, against the function's declared Ok type
                    // specifically (not just "whatever `?` unwrapped to",
                    // which `check_let` already confirmed equals this
                    // binding's own annotation).
                    if &last_binding.annotation != expected_ok.as_ref() {
                        self.report.error(last_binding.span, "E-RETURN-TYPE-RESULT-MISMATCH", format!(
                            "function '{}' is declared to return Result<{}, {}> but its unwrapped tail produces {}",
                            func.name, crate::docgen::render_type(expected_ok), crate::docgen::render_type(expected_err),
                            crate::docgen::render_type(&last_binding.annotation)
                        ));
                    }
                }
                Some(Declaration::Let(last_binding)) => match body_checker.result_bindings.get(&last_binding.name) {
                    Some((actual_ok, actual_err)) => {
                        if actual_ok != expected_ok.as_ref() || actual_err != expected_err.as_ref() {
                            self.report.error(last_binding.span, "E-RETURN-TYPE-RESULT-MISMATCH", format!(
                                "function '{}' is declared to return Result<{}, {}> but its body produces Result<{}, {}>",
                                func.name,
                                crate::docgen::render_type(expected_ok), crate::docgen::render_type(expected_err),
                                crate::docgen::render_type(actual_ok), crate::docgen::render_type(actual_err),
                            ));
                        }
                    }
                    None => {
                        self.report.error(last_binding.span, "E-RETURN-TYPE-NOT-RESULT", format!(
                            "function '{}' is declared to return Result<{}, {}> but its last binding does not produce a Result",
                            func.name, crate::docgen::render_type(expected_ok), crate::docgen::render_type(expected_err)
                        ));
                    }
                },
                _ => {
                    self.report.error(func.span, "E-RETURN-TYPE-NOT-RESULT", format!(
                        "function '{}' is declared to return Result<{}, {}> but its body does not end in a binding that produces one",
                        func.name, crate::docgen::render_type(expected_ok), crate::docgen::render_type(expected_err)
                    ));
                }
            }
        }

        self.report.diagnostics.extend(body_checker.report.diagnostics);

        FunctionDecl {
            name: func.name.clone(),
            params: func.params.clone(),
            return_type: func.return_type.clone(),
            body: desugared_body,
            span: func.span,
            doc: func.doc.clone(),
        }
    }

    fn check_strand(&mut self, strand: &StrandDecl) {
        if self.strands.insert(strand.name.clone(), strand.span).is_some() {
            self.report.error(strand.span, "E-STRAND-DUPLICATE", format!("strand '{}' is declared more than once", strand.name));
        }
        let parsed = match DnaStrand::from_str(&strand.sequence) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.report.error(strand.span, "E-STRAND-INVALID-DNA", format!("strand '{}' is not valid DNA: {}", strand.name, err));
                return;
            }
        };
        let validator = ConstraintValidator::new(nuclescript_constraints());
        let result = validator.validate(&parsed);
        for violation in result.violations {
            self.report.error(strand.span, "E-STRAND-CONSTRAINT-VIOLATION", format!(
                "strand '{}' violates NucleScript biological constraint: {}",
                strand.name, violation
            ));
        }
    }

    fn check_sequence(&mut self, sequence: &SequenceDecl) {
        if self.sequences.insert(sequence.name.clone(), sequence.span).is_some() {
            self.report.error(sequence.span, "E-SEQUENCE-DUPLICATE", format!("sequence '{}' is declared more than once", sequence.name));
        }
        let normalized = match normalize_sequence_literal(&sequence.sequence) {
            Ok(normalized) => normalized,
            Err(err) => {
                self.report.error(sequence.span, "E-SEQUENCE-INVALID-DNA", format!("sequence '{}' is not valid DNA: {}", sequence.name, err));
                return;
            }
        };
        let parsed = match DnaStrand::from_str(&normalized) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.report.error(sequence.span, "E-SEQUENCE-INVALID-DNA", format!("sequence '{}' is not valid DNA: {}", sequence.name, err));
                return;
            }
        };
        let validator = ConstraintValidator::new(sequence_literal_constraints());
        let result = validator.validate(&parsed);
        for violation in result.violations {
            self.report.error(sequence.span, "E-SEQUENCE-CONSTRAINT-VIOLATION", format!(
                "sequence '{}' violates NucleScript biological constraint: {}",
                sequence.name, violation
            ));
        }
    }

    fn check_store(&mut self, store: &StoreOp) {
        let effect = operation_effect(&Operation::Store(store.clone()));
        if effect == Effect::Synthesis && !store.simulate {
            self.report.warning(store.span, "E-STORE-SYNTHESIS-WITHOUT-SIMULATE", format!(
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
                // Synthesized from a probabilistic `let` binding, not a
                // real `pool` declaration -- there is no separate span to
                // point to, so borrow the enclosing `store`'s.
                span: store.span,
                doc: None,
            }
        } else {
            let suggestion = self.suggest_pool_name(&store.pool);
            self.report.error(store.span, "E-STORE-POOL-UNDECLARED", format!("store target pool '{}' is not declared{}", store.pool, did_you_mean(suggestion)));
            return;
        };
        let redundancy = store.options.redundancy.unwrap_or(pool.redundancy);
        if redundancy == 1 && store.options.tags.iter().any(|tag| tag.eq_ignore_ascii_case("critical")) {
            self.report.warning(store.span, "E-STORE-CRITICAL-LOW-REDUNDANCY", format!(
                "store '{}' is tagged critical but uses only 1x redundancy",
                store.file
            ));
        }
        let coverage = store.options.coverage.unwrap_or(redundancy);
        if pool.profile == Profile::Nanopore
            && coverage <= 1
            && store.options.expect_recovery_gt.is_some_and(|recovery| recovery > 99.5)
        {
            self.report.warning(store.span, "E-STORE-UNSATISFIABLE-RECOVERY", format!(
                "expect recovery > 99.5% is statistically unsatisfiable for Nanopore at {}x coverage",
                coverage
            ));
        }
        if store.simulate && store.options.coverage.is_none() {
            self.report.warning(store.span, "E-STORE-IMPLICIT-COVERAGE", format!(
                "simulate store '{}' uses implicit {}x coverage from redundancy",
                store.file, coverage
            ));
        }
    }

    fn check_delete(&mut self, delete: &DeleteOp) {
        if !self.pools.contains_key(&delete.pool) && !self.pool_bindings.contains_key(&delete.pool) {
            let suggestion = self.suggest_pool_name(&delete.pool);
            self.report.error(delete.span, "E-DELETE-POOL-UNDECLARED", format!("delete target pool '{}' is not declared{}", delete.pool, did_you_mean(suggestion)));
        }
        if !delete_has_required_confirmation(delete) {
            self.report.error(delete.span, "E-DELETE-UNCONFIRMED", format!(
                "delete '{}' from '{}' has Destructive effect and requires explicit physical key confirmation",
                delete.file, delete.pool
            ));
        }
    }

    fn check_retrieve(&mut self, retrieve: &RetrieveOp) {
        if !self.pools.contains_key(&retrieve.pool) && !self.pool_bindings.contains_key(&retrieve.pool) {
            let suggestion = self.suggest_pool_name(&retrieve.pool);
            self.report.error(retrieve.span, "E-RETRIEVE-POOL-UNDECLARED", format!("retrieve source pool '{}' is not declared{}", retrieve.pool, did_you_mean(suggestion)));
        }
        for predicate in &retrieve.query {
            match predicate.field.to_ascii_lowercase().as_str() {
                "tag" | "date" | "size" | "name" | "type" => {}
                other => self.report.warning(retrieve.span, "E-RETRIEVE-UNINDEXED-FIELD", format!(
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
                        self.report.warning(pipeline.span, "E-PIPELINE-UNSUPPORTED-CODEC", format!(
                            "pipeline '{}' uses Fountain codec; the VFS backend only executes Ternary and YinYang end-to-end",
                            pipeline.name
                        ));
                    }
                }
                PipelineStep::Protect { redundancy } => {
                    saw_protect = true;
                    if *redundancy == 1 {
                        self.report.warning(pipeline.span, "E-PIPELINE-LOW-REDUNDANCY", format!("pipeline '{}' protects with only 1x redundancy", pipeline.name));
                    }
                }
                PipelineStep::Store { pool } => {
                    saw_store = true;
                    if !self.pools.contains_key(pool) {
                        let suggestion = self.suggest_pool_name(pool);
                        self.report.error(pipeline.span, "E-PIPELINE-POOL-UNDECLARED", format!("pipeline '{}' stores into undeclared pool '{}'{}", pipeline.name, pool, did_you_mean(suggestion)));
                    }
                }
                PipelineStep::VerifyRoundtrip => {}
            }
        }
        if saw_store && !saw_encode {
            self.report.error(pipeline.span, "E-PIPELINE-STORE-BEFORE-ENCODE", format!("pipeline '{}' stores data before an encode step", pipeline.name));
        }
        if saw_store && !saw_protect {
            self.report.warning(pipeline.span, "E-PIPELINE-STORE-WITHOUT-PROTECT", format!("pipeline '{}' stores without an explicit protect step", pipeline.name));
        }
    }
}

/// Levenshtein edit distance (case-insensitive), used only for "did you
/// mean X?" suggestions -- inputs are identifier-length strings against at
/// most a few dozen candidates per scope, so the classic O(nm) DP table is
/// plenty fast; no need for a fuzzy-matching crate at this scale.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_ascii_lowercase().chars().collect();
    let b: Vec<char> = b.to_ascii_lowercase().chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// The closest name to `target` among `candidates` within edit distance 2
/// (typo-sized) -- `None` if nothing is close enough to be worth
/// suggesting, so a "did you mean X?" never fires on a wild guess.
fn suggest_name(target: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (c.clone(), edit_distance(target, c)))
        .filter(|(_, dist)| *dist <= 2 && *dist > 0)
        .min_by_key(|(_, dist)| *dist)
        .map(|(name, _)| name)
}

fn did_you_mean(suggestion: Option<String>) -> String {
    match suggestion {
        Some(name) => format!(" (did you mean '{}'?)", name),
        None => String::new(),
    }
}

/// Substitutes every bare occurrence of `binding` (a pool name, file
/// variable, or `Expr::Variable`) with `value` inside a single `for`-loop
/// body declaration -- how `TypeChecker::check_for` "unrolls" a loop: one
/// substituted copy per item, each checked independently as if
/// hand-written. Declarations that carry no identifier fields at all
/// (`Import`/`Pool`/`Strand`/`Sequence`/`Pipeline`/`Function`) pass through
/// unchanged -- a loop is only useful for repeating operations/bindings
/// that reference the loop variable, and those are the variants handled
/// below.
fn substitute_declaration(decl: &Declaration, binding: &str, value: &str) -> Declaration {
    let sub = |s: &str| -> String { if s == binding { value.to_string() } else { s.to_string() } };
    match decl {
        Declaration::Let(d) => Declaration::Let(LetDecl {
            name: d.name.clone(),
            annotation: d.annotation.clone(),
            expr: substitute_expr(&d.expr, binding, value),
            span: d.span,
        }),
        Declaration::Operation(Operation::Store(op)) => Declaration::Operation(Operation::Store(StoreOp {
            simulate: op.simulate,
            file: sub(&op.file),
            pool: sub(&op.pool),
            options: op.options.clone(),
            span: op.span,
        })),
        Declaration::Operation(Operation::Retrieve(op)) => Declaration::Operation(Operation::Retrieve(RetrieveOp {
            pool: sub(&op.pool),
            query: op.query.clone(),
            span: op.span,
        })),
        Declaration::Operation(Operation::Delete(op)) => Declaration::Operation(Operation::Delete(DeleteOp {
            file: sub(&op.file),
            pool: sub(&op.pool),
            confirmed: op.confirmed,
            span: op.span,
        })),
        Declaration::Operation(Operation::Assert(op)) => Declaration::Operation(Operation::Assert(AssertOp {
            condition: substitute_expr(&op.condition, binding, value),
            message: op.message.clone(),
            span: op.span,
        })),
        Declaration::If(d) => Declaration::If(IfDecl {
            condition: substitute_expr(&d.condition, binding, value),
            then_branch: d.then_branch.iter().map(|inner| substitute_declaration(inner, binding, value)).collect(),
            else_branch: d
                .else_branch
                .as_ref()
                .map(|branch| branch.iter().map(|inner| substitute_declaration(inner, binding, value)).collect()),
            span: d.span,
        }),
        Declaration::For(d) => Declaration::For(ForDecl {
            binding: d.binding.clone(),
            items: d.items.iter().map(|item| sub(item)).collect(),
            // An inner loop reusing the same binding name shadows the
            // outer one within its own body, matching ordinary lexical
            // scoping -- don't substitute through it.
            body: if d.binding == binding {
                d.body.clone()
            } else {
                d.body.iter().map(|inner| substitute_declaration(inner, binding, value)).collect()
            },
            span: d.span,
        }),
        // `test`'s `name` is just a description string, not a binding
        // that could shadow the loop variable (unlike `ForDecl.binding`
        // above) -- always substitute through its body.
        Declaration::Test(d) => Declaration::Test(TestDecl {
            name: d.name.clone(),
            body: d.body.iter().map(|inner| substitute_declaration(inner, binding, value)).collect(),
            span: d.span,
        }),
        Declaration::Import(_)
        | Declaration::Pool(_)
        | Declaration::Strand(_)
        | Declaration::Sequence(_)
        | Declaration::Pipeline(_)
        | Declaration::Function(_) => decl.clone(),
    }
}

fn substitute_expr(expr: &Expr, binding: &str, value: &str) -> Expr {
    let sub = |s: &str| -> String { if s == binding { value.to_string() } else { s.to_string() } };
    match expr {
        Expr::SimulatePool { pool, profile } => Expr::SimulatePool { pool: sub(pool), profile: *profile },
        Expr::SynthesizePool { source, profile, confirmed } => {
            Expr::SynthesizePool { source: sub(source), profile: *profile, confirmed: *confirmed }
        }
        Expr::SequencePool { source, profile, confirmed } => {
            Expr::SequencePool { source: sub(source), profile: *profile, confirmed: *confirmed }
        }
        Expr::FunctionCall { name, args } => Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(|arg| substitute_expr(arg, binding, value)).collect(),
        },
        Expr::Variable(name) => Expr::Variable(sub(name)),
        Expr::StringLiteral(s) => Expr::StringLiteral(s.clone()),
        Expr::Number(n) => Expr::Number(*n),
        Expr::BinaryOp { op, left, right } => Expr::BinaryOp {
            op: *op,
            left: Box::new(substitute_expr(left, binding, value)),
            right: Box::new(substitute_expr(right, binding, value)),
        },
        Expr::Not(inner) => Expr::Not(Box::new(substitute_expr(inner, binding, value))),
        Expr::Try(inner) => Expr::Try(Box::new(substitute_expr(inner, binding, value))),
        // Mirrors the statement-form Operation::Store/Retrieve/Delete arms
        // in substitute_declaration above exactly -- same fields, same
        // `sub(...)` substitution, since both surface forms wrap the
        // identical StoreOp/RetrieveOp/DeleteOp struct.
        Expr::StoreExpr(op) => Expr::StoreExpr(StoreOp {
            simulate: op.simulate,
            file: sub(&op.file),
            pool: sub(&op.pool),
            options: op.options.clone(),
            span: op.span,
        }),
        Expr::RetrieveExpr(op) => Expr::RetrieveExpr(RetrieveOp {
            pool: sub(&op.pool),
            query: op.query.clone(),
            span: op.span,
        }),
        Expr::DeleteExpr(op) => Expr::DeleteExpr(DeleteOp {
            file: sub(&op.file),
            pool: sub(&op.pool),
            confirmed: op.confirmed,
            span: op.span,
        }),
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
                span: Span::default(),
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
                    span: Span::default(),
                    doc: None,
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
                    span: Span::default(),
                }),
                Declaration::Let(LetDecl {
                    name: "recovered".into(),
                    annotation: TypeExpr::Pool(PoolType {
                        state: PoolState::Recovered,
                        error_rate_percent: None,
                    }),
                    expr: Expr::FunctionCall {
                        name: "consensus_vote".into(),
                        args: vec![Expr::Variable("noisy".into()), Expr::Number(10.0)],
                    },
                    span: Span::default(),
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
                    span: Span::default(),
                    doc: None,
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
                    span: Span::default(),
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
                    span: Span::default(),
                    doc: None,
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
                    span: Span::default(),
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
                    span: Span::default(),
                    doc: None,
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
                    span: Span::default(),
                }),
                Declaration::Operation(Operation::Delete(DeleteOp {
                    file: "old.bin".into(),
                    pool: "archive".into(),
                    confirmed: true,
                    span: Span::default(),
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
                    span: Span::default(),
                    doc: None,
                }),
                Declaration::Operation(Operation::Delete(DeleteOp {
                    file: "old.bin".into(),
                    pool: "archive".into(),
                    confirmed: false,
                    span: Span::default(),
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
                span: Span::default(),
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
                span: Span::default(),
            })],
        };
        assert!(check_program(&program).has_errors());
    }
}
