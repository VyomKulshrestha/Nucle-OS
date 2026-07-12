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
        enums: checker.enums,
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
    pub enums: HashMap<String, EnumDecl>,
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
    /// Bindings whose inferred type is `Fn(params) -> Ok` -- a closure
    /// (from a `let`) or a `Fn(...)`-typed function parameter, parallel
    /// to `result_bindings`/`pool_bindings`. Only the signature is kept
    /// (a call site only ever needs to validate args/return type against
    /// it); the closure's actual body/captured values live purely at
    /// runtime in `codegen::Value::Closure`, never here. The `Vec<String>`
    /// is the closure's own declared type parameters (`fn<T>(...)`,
    /// mirroring `FunctionDecl::type_params`) -- always empty for a
    /// `Fn(...)`-typed *parameter*, since `Fn(...)`'s own type syntax has
    /// no `<T>` list of its own; any `PoolState::Var` it contains always
    /// refers to the *enclosing* function/closure's own type parameter,
    /// resolved through that outer scope's own call-site unification,
    /// not this one.
    closures: HashMap<String, (Vec<String>, Vec<TypeExpr>, TypeExpr)>,
    /// Real, callable bodies for `let`-bound closures ONLY -- a synthetic
    /// `FunctionDecl` per closure (`name`/`type_params`/`doc` unused,
    /// `params`/`return_type`/`body` real), so `effects::expr_effect`'s
    /// call-site resolution can compute the closure's *actual* effect by
    /// recursing into it, exactly like a named function already can.
    /// Deliberately NOT populated for a `Fn(...)`-typed *parameter* --
    /// whatever closure a caller passes isn't knowable here at all, only
    /// at runtime, so that case is a real, documented gap (see
    /// `effects::expr_effect`'s doc comment), not silently guessed at.
    closure_decls: HashMap<String, FunctionDecl>,
    /// A `Fn(...)`-typed *parameter*'s declared effect ceiling (from an
    /// `Fn(...) -> T confirm hardware`/`confirm physical_key` annotation
    /// on its type), keyed by parameter name -- the counterpart to
    /// `closure_decls` for the one case that has no real body to recurse
    /// into. `Hardware`/`PhysicalKey` are converted to the `Effect` they
    /// stand in for (see `FnEffectAnnotation::to_effect`) before storage,
    /// since this is purely internal bookkeeping consumed by
    /// `effects.rs`'s own `fn_param_effects` threading. Only ever
    /// populated for an *annotated* `Fn(...)`-typed parameter -- an
    /// unannotated one contributes no entry here, preserving today's
    /// exact "unresolvable call, assumed Pure" behavior unchanged.
    fn_param_effects: HashMap<String, Effect>,
    /// The `(Ok, Err)` pair of the function/closure currently being
    /// type-checked, if it declares a `Result<...>` return type -- `None`
    /// at top level (a `?` outside any function) and for a non-`Result`-
    /// returning function/closure, both of which make `?` invalid. Set
    /// once in `check_function`/`check_closure_expr` before the body is
    /// checked, from that function/closure's *own* declared return type
    /// -- a closure's `?` always validates against the closure's own
    /// signature, never an outer function's, even though the closure's
    /// checker otherwise inherits the outer scope's bindings for capture.
    enclosing_result_return: Option<(TypeExpr, TypeExpr)>,
    /// User-declared `enum`s (Step 14), registered incrementally as
    /// `check_declaration_single` walks `Declaration::Enum` entries --
    /// same "declare before use" convention `pools`/`functions` already
    /// have. `Result<T, E>` is never an entry here; see `TypeExpr::Enum`'s
    /// own doc comment for why it stays its own privileged type.
    enums: HashMap<String, EnumDecl>,
    /// User-enum-typed `let` bindings AND function/closure *parameters*
    /// (Step 14) -- parallel to `pool_bindings`/`result_bindings`, but
    /// specifically for the case a parameter has no `LetDecl` of its own
    /// to register in `self.bindings` (which only ever holds real `let`
    /// declarations, so a `RecoveryPlan`-typed parameter would otherwise
    /// be invisible to `infer_scrutinee_kind`'s `Expr::Variable` lookup).
    /// A `let`-bound enum variable is tracked here too (redundantly with
    /// `self.bindings`) purely so `infer_scrutinee_kind` has one map to
    /// check regardless of which kind of binding it is.
    enum_bindings: HashMap<String, String>,
    report: TypeReport,
}

/// One variant's name and (optional) payload type, in declaration order
/// -- the "shape" a `match`'s scrutinee is checked against (Step 14),
/// whether that shape comes from the built-in Result pseudo-enum or a
/// real user `EnumDecl`. See `TypeChecker::check_match`.
struct VariantSignature {
    name: String,
    payload_ty: Option<TypeExpr>,
}

/// What `TypeChecker::infer_scrutinee_kind` resolves a `match`'s
/// scrutinee to.
enum ScrutineeKind {
    /// `Result<ok_ty, err_ty>`-shaped (via the existing, unchanged
    /// `infer_result_expr`). Its "declared variants" are always exactly
    /// `[Ok(ok_ty), Err(err_ty)]` -- never looked up in `self.enums`
    /// (Result is never registered there; see `check_enum`'s
    /// `E-ENUM-RESERVED-NAME` guard).
    Result { ok_ty: TypeExpr, err_ty: TypeExpr },
    /// A real user-declared enum, looked up by name.
    UserEnum { decl: EnumDecl },
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
            Declaration::Enum(enum_decl) => {
                self.check_enum(enum_decl);
                vec![declaration.clone()]
            }
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
        body_checker.enums = self.enums.clone();
        body_checker.enum_bindings = self.enum_bindings.clone();

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

    /// `enum Name { Variant1, Variant2(Type), ... }` (Step 14). `Result`
    /// is reserved -- it's never an instance of this general mechanism,
    /// just uniformly *matchable* alongside one (see `check_match`).
    /// Whether a variant's own payload type (if it names another enum)
    /// actually resolves is checked lazily, at the point something tries
    /// to construct/match it (`check_enum_construct`/`check_match`) --
    /// exactly like `Pool<...>`-typed parameters aren't independently
    /// re-validated against "does this pool exist" at declaration time
    /// either.
    fn check_enum(&mut self, decl: &EnumDecl) {
        if decl.name == "Result" {
            self.report.error(decl.span, "E-ENUM-RESERVED-NAME", "'Result' is a built-in type and cannot be redeclared as an enum");
            return;
        }
        if self.enums.contains_key(&decl.name) {
            self.report.error(decl.span, "E-ENUM-DUPLICATE", format!("enum '{}' is declared more than once", decl.name));
            return;
        }
        if decl.variants.is_empty() {
            self.report.error(decl.span, "E-ENUM-EMPTY", format!("enum '{}' has no variants", decl.name));
            return;
        }
        let mut seen = HashSet::new();
        for variant in &decl.variants {
            if !seen.insert(&variant.name) {
                self.report.error(variant.span, "E-ENUM-VARIANT-DUPLICATE", format!(
                    "enum '{}' declares variant '{}' more than once", decl.name, variant.name
                ));
            }
        }
        self.enums.insert(decl.name.clone(), decl.clone());
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
            || self.closures.contains_key(&binding.name)
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
        let effect = expr_effect(&binding.expr, &self.functions, &self.closure_decls, &self.fn_param_effects, &mut std::collections::HashSet::new());
        if !expr_has_required_confirmation(&binding.expr, &self.functions, &self.closure_decls, &self.fn_param_effects, &mut std::collections::HashSet::new()) {
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

        // `let x: T = match <result-expr> { Ok(...) => ..., Err(...) => ... }`.
        // Checked before the un-unwrapped Result path below (and before
        // `Expr::Try`'s check above, though the two can never both match
        // the same `binding.expr` shape) since `match` needs its own
        // arm-unification logic, not either of the other two dispatches.
        if let Expr::Match { .. } = &binding.expr {
            if let Some(matched_ty) = self.check_match(&binding.expr, binding.span) {
                if matched_ty != binding.annotation {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but 'match' produces {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&matched_ty)
                    ));
                } else {
                    if let TypeExpr::Result(ok, err) = &matched_ty {
                        self.result_bindings.insert(binding.name.clone(), ((**ok).clone(), (**err).clone()));
                    }
                    self.bindings.insert(binding.name.clone(), binding.clone());
                }
            }
            return;
        }

        // `let x: Result<T, E> = Ok(<expr>)` / `Err(<string-literal>)` --
        // the enclosing binding's own annotation supplies whichever side
        // the constructor's own expression can't (see `check_ok_expr`/
        // `check_err_expr`). Checked before the un-unwrapped Result path
        // below for the same reason `Match`/`Closure` are.
        if let Expr::Ok(inner) = &binding.expr {
            let expected_err = if let TypeExpr::Result(_, err) = &binding.annotation { Some(err.as_ref()) } else { None };
            if let Some((ok, err)) = self.check_ok_expr(inner, expected_err, binding.span) {
                let constructed = TypeExpr::Result(Box::new(ok.clone()), Box::new(err.clone()));
                if constructed != binding.annotation {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but Ok(...) produces {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&constructed)
                    ));
                } else {
                    self.result_bindings.insert(binding.name.clone(), (ok, err));
                    self.bindings.insert(binding.name.clone(), binding.clone());
                }
            }
            return;
        }
        if let Expr::Err(inner) = &binding.expr {
            let expected_ok = if let TypeExpr::Result(ok, _) = &binding.annotation { Some(ok.as_ref()) } else { None };
            if let Some((ok, err)) = self.check_err_expr(inner, expected_ok, binding.span) {
                let constructed = TypeExpr::Result(Box::new(ok.clone()), Box::new(err.clone()));
                if constructed != binding.annotation {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but Err(...) produces {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&constructed)
                    ));
                } else {
                    self.result_bindings.insert(binding.name.clone(), (ok, err));
                    self.bindings.insert(binding.name.clone(), binding.clone());
                }
            }
            return;
        }

        // `let x: EnumName = EnumName::Variant(<expr>)` / `EnumName::
        // Variant` -- a direct user-enum construction (Step 14). Checked
        // before the un-unwrapped Result path below for the same reason
        // `Ok`/`Err`/`Match`/`Closure` are: without this branch, neither
        // `infer_result_expr` nor `infer_expr` recognizes `Expr::
        // EnumConstruct` at all, so the binding would silently fall
        // through `None => return` below with no diagnostic and never
        // register in `self.bindings`.
        if let Expr::EnumConstruct { enum_name, variant, payload } = &binding.expr {
            if let Some(constructed) = self.check_enum_construct(enum_name, variant, payload.as_deref(), binding.span) {
                if constructed != binding.annotation {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but this constructs {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&constructed)
                    ));
                } else {
                    self.bindings.insert(binding.name.clone(), binding.clone());
                    self.enum_bindings.insert(binding.name.clone(), enum_name.clone());
                }
            }
            return;
        }

        // `let f: Fn(...) -> T = fn(params) -> T { body }` -- a closure
        // literal. Checked before the un-unwrapped Result path below for
        // the same reason `Match` is: it needs its own dispatch, not
        // either of the other two.
        if let Expr::Closure { type_params, params, return_type, body, span } = &binding.expr {
            // Self-recursion: pre-register this binding's own name using
            // the ANNOTATION's declared signature (the body hasn't been
            // checked yet) so `check_closure_expr`'s internal clone of
            // `self.closures`/`self.closure_decls` (capture) already
            // includes this closure's own name pointing at itself --
            // exactly how a self-recursive named function already works,
            // just via `let`-binding instead of a top-level declaration.
            // Rolled back below if the body turns out not to actually
            // match its own declared type, so a broken self-reference
            // never leaks into whatever's checked next.
            let pre_registered = if let TypeExpr::Fn(param_types, ret, _) = &binding.annotation {
                self.closures.insert(binding.name.clone(), (type_params.clone(), param_types.clone(), (**ret).clone()));
                self.closure_decls.insert(binding.name.clone(), FunctionDecl {
                    name: binding.name.clone(),
                    type_params: type_params.clone(),
                    params: params.clone(),
                    return_type: return_type.clone(),
                    body: body.clone(),
                    span: *span,
                    doc: None,
                });
                true
            } else {
                false
            };
            let closure_ty = self.check_closure_expr(type_params, params, return_type, body, *span);
            // A closure literal's own inferred type never carries an
            // effect annotation (see `check_closure_expr`'s own
            // construction site) -- only the *slot* it's bound into
            // does. So the annotation field is erased from the
            // BINDING's own declared type before this structural
            // comparison, and validated separately, right after, via
            // `check_fn_effect_compatibility` -- comparing them directly
            // would make any annotated `Fn(...)` binding permanently
            // unsatisfiable (`Some(...)` can never structurally equal
            // `None`).
            let annotation_erased = match &binding.annotation {
                TypeExpr::Fn(p, r, _) => TypeExpr::Fn(p.clone(), r.clone(), None),
                other => other.clone(),
            };
            let declared_effect = match &binding.annotation {
                TypeExpr::Fn(_, _, effect) => *effect,
                _ => None,
            };
            match closure_ty {
                Some(closure_ty) if closure_ty == annotation_erased => {
                    if let TypeExpr::Fn(param_types, ret, _) = closure_ty {
                        self.check_fn_effect_compatibility(declared_effect, &binding.expr, binding.span);
                        self.closures.insert(binding.name.clone(), (type_params.clone(), param_types, *ret));
                        self.closure_decls.insert(binding.name.clone(), FunctionDecl {
                            name: binding.name.clone(),
                            type_params: type_params.clone(),
                            params: params.clone(),
                            return_type: return_type.clone(),
                            body: body.clone(),
                            span: *span,
                            doc: None,
                        });
                        self.bindings.insert(binding.name.clone(), binding.clone());
                    }
                }
                Some(closure_ty) => {
                    self.report.error(binding.span, "E-BINDING-TYPE-MISMATCH", format!(
                        "binding '{}' is annotated as {} but this closure's own type is {}",
                        binding.name, crate::docgen::render_type(&binding.annotation), crate::docgen::render_type(&closure_ty)
                    ));
                    if pre_registered {
                        self.closures.remove(&binding.name);
                        self.closure_decls.remove(&binding.name);
                    }
                }
                None => {
                    if pre_registered {
                        self.closures.remove(&binding.name);
                        self.closure_decls.remove(&binding.name);
                    }
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
        if let Some((ok, err)) = self.infer_result_expr(&binding.expr, binding.span) {
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
    fn infer_result_expr(&mut self, expr: &Expr, span: Span) -> Option<(TypeExpr, TypeExpr)> {
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
            // No external context available in this generic path -- see
            // `check_ok_expr`/`check_err_expr`'s own doc comments for
            // where context-aware callers (`check_let`, `check_match`,
            // `check_function`/`check_closure_expr`'s tail validation)
            // supply it instead.
            Expr::Ok(inner) => self.check_ok_expr(inner, None, span),
            Expr::Err(inner) => self.check_err_expr(inner, None, span),
            // Nested `match`/`?` composability: a `match` expression can
            // itself be `Result`-shaped (when both arms are still-
            // wrapped), so delegating to `check_match` here is what makes
            // `match (match a {...}) {...}` and `(match a {...})?` both
            // resolve -- every caller of this function (`check_try`,
            // `check_match`'s own scrutinee check, `check_match_arm`)
            // benefits automatically.
            Expr::Match { .. } => match self.check_match(expr, span) {
                Some(TypeExpr::Result(ok, err)) => Some(((*ok).clone(), (*err).clone())),
                _ => None,
            },
            Expr::Variable(name) => self.result_bindings.get(name).cloned(),
            // Closures resolve first, same priority as `infer_expr`'s own
            // `FunctionCall` arm -- see its comment for why.
            Expr::FunctionCall { name, args, .. } if self.closures.contains_key(name) => {
                let (type_params, param_types, return_type) = self.closures[name].clone();
                let resolved_return = self.check_closure_call_args(name, &type_params, &param_types, &return_type, args, span);
                match resolved_return {
                    Some(TypeExpr::Result(ok, err)) => Some(((*ok).clone(), (*err).clone())),
                    _ => None,
                }
            }
            // Argument validation for non-`Fn`-typed parameters is
            // intentionally skipped here, matching the pre-existing
            // (unrelated to closures) gap this path has always had --
            // but a `Fn(...)`-typed parameter's own argument *is*
            // validated (see `check_fn_typed_arg`): unlike an unchecked
            // `Pool`/`Str`/etc. argument, an unchecked closure argument
            // would mean its entire body -- and whatever it does at
            // runtime -- was never type-checked at all.
            Expr::FunctionCall { name, args, .. } => {
                let func = self.lookup_function(name)?;
                for (param, arg) in func.params.iter().zip(args.iter()) {
                    if let TypeExpr::Fn(expected_params, expected_return, expected_effect) = &param.ty {
                        self.check_fn_typed_arg(expected_params, expected_return, *expected_effect, arg, &param.name, span);
                    }
                }
                match func.return_type {
                    TypeExpr::Result(ok, err) => Some((*ok, *err)),
                    _ => None,
                }
            }
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
        let Some((ok_ty, err_ty)) = self.infer_result_expr(inner, span) else {
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

    /// What a `match`'s scrutinee resolved to -- either the built-in
    /// `Result<T, E>` "pseudo-enum" (never registered in `self.enums`) or
    /// a real user-declared `enum`. See `check_match`.
    fn infer_scrutinee_kind(&mut self, expr: &Expr, span: Span) -> Option<ScrutineeKind> {
        // Reuses the EXISTING, unchanged `infer_result_expr` first --
        // Result resolution is completely unaffected by user enums; every
        // expr shape it already recognizes (StoreExpr/DeleteExpr/Ok/Err/
        // nested Match/Variable/FunctionCall) keeps working exactly as
        // before.
        if let Some((ok_ty, err_ty)) = self.infer_result_expr(expr, span) {
            return Some(ScrutineeKind::Result { ok_ty, err_ty });
        }
        // Otherwise, is it enum-shaped? Mirrors `infer_result_expr`'s own
        // Variable/FunctionCall/nested-Match cases, but resolving against
        // `self.enums` instead of a fixed Ok/Err pair.
        let enum_name = match expr {
            Expr::EnumConstruct { enum_name, .. } => Some(enum_name.clone()),
            // Checks `enum_bindings` (covers both a `let`-bound enum
            // variable and a function/closure *parameter*, which has no
            // `LetDecl` of its own to look up in `self.bindings`) -- see
            // `TypeChecker::enum_bindings`'s own doc comment.
            Expr::Variable(name) => self.enum_bindings.get(name).cloned(),
            Expr::FunctionCall { name, .. } if self.closures.contains_key(name) => match &self.closures[name].2 {
                TypeExpr::Enum(n) => Some(n.clone()),
                _ => None,
            },
            Expr::FunctionCall { name, .. } => match self.lookup_function(name).map(|f| f.return_type.clone()) {
                Some(TypeExpr::Enum(n)) => Some(n),
                _ => None,
            },
            // A nested match whose own unified arm type is an enum --
            // composability, mirroring `infer_result_expr`'s own nested-
            // Match delegation.
            Expr::Match { .. } => match self.check_match(expr, span) {
                Some(TypeExpr::Enum(n)) => Some(n),
                _ => None,
            },
            _ => None,
        }?;
        self.enums.get(&enum_name).cloned().map(|decl| ScrutineeKind::UserEnum { decl })
    }

    /// `match <scrutinee> { <arm>, ... }`'s validity (Step 14) --
    /// generalizes the original Result-only, fixed-two-arm check to any
    /// number of arms over either Result's own two implicit variants
    /// (`Ok`/`Err`) or a user-declared `enum`'s real variant list.
    /// `scrutinee` must resolve to one of those (`E-MATCH-UNRECOGNIZED-
    /// SCRUTINEE` if not); every declared variant needs exactly one arm
    /// (by name) or a trailing wildcard covering the rest
    /// (`E-MATCH-NON-EXHAUSTIVE`); an arm naming an unknown variant
    /// (`E-MATCH-UNKNOWN-VARIANT`), two arms naming the same variant
    /// (`E-MATCH-DUPLICATE-ARM`), or a wildcard that isn't last
    /// (`E-MATCH-ARM-AFTER-WILDCARD`) are all rejected. Returns the
    /// unified arm type on success (`E-MATCH-ARM-TYPE-MISMATCH` if arms
    /// disagree) -- exactly what a `match` expression "is", the same way
    /// `check_try` returns `?`'s unwrapped type.
    fn check_match(&mut self, m: &Expr, span: Span) -> Option<TypeExpr> {
        let Expr::Match { scrutinee, arms } = m else {
            unreachable!("check_match called on a non-Match expression");
        };

        let Some(kind) = self.infer_scrutinee_kind(scrutinee, span) else {
            self.report.error(span, "E-MATCH-UNRECOGNIZED-SCRUTINEE", "'match' can only scrutinize a Result<T, E>-typed expression or a declared enum-typed expression");
            return None;
        };
        let declared_variants: Vec<VariantSignature> = match &kind {
            ScrutineeKind::Result { ok_ty, err_ty } => vec![
                VariantSignature { name: "Ok".to_string(), payload_ty: Some(ok_ty.clone()) },
                VariantSignature { name: "Err".to_string(), payload_ty: Some(err_ty.clone()) },
            ],
            ScrutineeKind::UserEnum { decl } => {
                decl.variants.iter().map(|v| VariantSignature { name: v.name.clone(), payload_ty: v.payload.clone() }).collect()
            }
        };

        // Pass 1: shape-check the arm list itself -- unknown variant
        // names, a wildcard that isn't last, duplicate arms for the same
        // variant -- independent of body types.
        let mut wildcard_seen = false;
        for arm in arms {
            if wildcard_seen {
                self.report.error(arm.span, "E-MATCH-ARM-AFTER-WILDCARD", "no arms are allowed after a wildcard '_' arm");
            }
            match &arm.variant {
                None => wildcard_seen = true,
                Some(name) if !declared_variants.iter().any(|v| &v.name == name) => {
                    self.report.error(arm.span, "E-MATCH-UNKNOWN-VARIANT", format!("'{}' is not a variant of this match's scrutinee type", name));
                }
                Some(_) => {}
            }
        }
        let mut seen_names: HashSet<&String> = HashSet::new();
        for arm in arms {
            if let Some(name) = &arm.variant {
                if !seen_names.insert(name) {
                    self.report.error(arm.span, "E-MATCH-DUPLICATE-ARM", format!("variant '{}' is matched by more than one arm", name));
                }
            }
        }

        // Pass 2: exhaustiveness -- every declared variant needs a named
        // arm, unless a wildcard covers whatever's missing.
        if !wildcard_seen {
            for v in &declared_variants {
                if !seen_names.contains(&v.name) {
                    self.report.error(span, "E-MATCH-NON-EXHAUSTIVE", format!("match is missing an arm for variant '{}'", v.name));
                }
            }
        }

        // Pass 3: type-check each present arm's body, in source order.
        // `declared_variants` is static context known up front (the
        // scrutinee's own type), which is what makes the old two-arm
        // "hand Err the Ok arm's context" trick generalize cleanly to N
        // arms with no incremental accumulator needed -- see
        // `check_match_arm_general`'s own doc comment.
        let mut resolved: Vec<(Span, TypeExpr)> = Vec::new();
        let mut any_failed = false;
        for arm in arms {
            let pattern_ty = arm.variant.as_ref().and_then(|n| declared_variants.iter().find(|v| &v.name == n)).and_then(|v| v.payload_ty.clone());
            match self.check_match_arm_general(&arm.body, arm.binding.as_deref(), pattern_ty.as_ref(), &declared_variants, arm.span) {
                Some(ty) => resolved.push((arm.span, ty)),
                None => any_failed = true,
            }
        }
        if any_failed || resolved.is_empty() {
            return None;
        }

        // Pass 4: unify all resolved arm types -- generalizes the old
        // 2-way Ok/Err equality check to N arms.
        let (_, first_ty) = resolved[0].clone();
        for (arm_span, ty) in &resolved[1..] {
            if ty != &first_ty {
                self.report.error(*arm_span, "E-MATCH-ARM-TYPE-MISMATCH", format!(
                    "match arms produce different types: expected {}, got {}",
                    crate::docgen::render_type(&first_ty), crate::docgen::render_type(ty)
                ));
                return None;
            }
        }
        Some(first_ty)
    }

    /// A match arm's body is one of: the pattern variable itself (the
    /// trivial "use the unwrapped value as-is" arm), `Ok(<pattern>)`/
    /// `Err(<pattern>)` (Result-specific re-wrap) or `EnumName::Variant(
    /// <pattern>)` (the general form, for a user enum) -- re-wrapping the
    /// arm's own bound value; see below for why this is special-cased
    /// rather than routed through `check_ok_expr`/`check_err_expr`/
    /// `check_enum_construct`'s generic path -- `?` (reuses `check_try`),
    /// a Result-shaped expression (reuses `infer_result_expr` -- an arm
    /// can produce a still-wrapped `Result`, e.g. a fallback `store`), an
    /// enum-shaped expression (mirrors the Result case for user enums),
    /// or a Pool-shaped expression (reuses `infer_expr`) -- the same
    /// dispatch `check_let` already runs for a `let`'s RHS, plus the
    /// pattern-name cases a `let` doesn't need.
    ///
    /// `declared_variants` is the scrutinee's own full variant list,
    /// known statically before any arm is checked (see `check_match`).
    /// This is what the old two-arm code's "hand the Err arm the Ok
    /// arm's context" trick actually generalizes to: re-reading that
    /// code closely, `Err(reason) => Err(reason)`'s missing `Ok` type
    /// never needed the *Ok arm's own resolved value*, only the
    /// scrutinee's *declared* Ok-side type -- which is already part of
    /// `declared_variants`, with no need to thread an incremental
    /// "resolved so far" accumulator through arm checking. A general
    /// `EnumName::Variant(pattern) => EnumName::Variant(pattern)` re-wrap
    /// needs no "other side" context at all -- just its own variant's
    /// declared payload type, already known via `pattern_ty`.
    fn check_match_arm_general(
        &mut self,
        body: &Expr,
        binding: Option<&str>,
        pattern_ty: Option<&TypeExpr>,
        declared_variants: &[VariantSignature],
        span: Span,
    ) -> Option<TypeExpr> {
        // 1. The pattern's own bound name, used directly.
        if let (Some(pattern), Expr::Variable(name)) = (binding, body) {
            if name == pattern {
                return match pattern_ty {
                    Some(ty) => Some(ty.clone()),
                    None => {
                        self.report.error(span, "E-MATCH-ARM-UNTYPABLE", "this arm's variant has no payload to bind, but its body uses the pattern name as a value");
                        None
                    }
                };
            }
        }
        // 2. `Ok(<pattern>)`/`Err(<pattern>)` -- Result-specific re-wrap.
        // The pattern's own name is deliberately never registered in any
        // typeck scope map (see `Expr::Match`'s doc comment in ast.rs),
        // so `check_ok_expr`/`check_err_expr`'s generic,
        // externally-resolvable-expression path can't see it.
        if let (Some(pattern), Expr::Ok(inner)) = (binding, body) {
            if matches!(inner.as_ref(), Expr::Variable(name) if name == pattern) {
                let err_ty = declared_variants.iter().find(|v| v.name == "Err").and_then(|v| v.payload_ty.clone()).unwrap_or(TypeExpr::Str);
                let Some(ok_ty) = pattern_ty.cloned() else {
                    self.report.error(span, "E-MATCH-ARM-UNTYPABLE", "this arm's variant has no payload to bind for Ok(...) to re-wrap");
                    return None;
                };
                return Some(TypeExpr::Result(Box::new(ok_ty), Box::new(err_ty)));
            }
        }
        if let (Some(pattern), Expr::Err(inner)) = (binding, body) {
            if matches!(inner.as_ref(), Expr::Variable(name) if name == pattern) {
                let Some(ok_ty) = declared_variants.iter().find(|v| v.name == "Ok").and_then(|v| v.payload_ty.clone()) else {
                    self.report.error(span, "E-ERR-CONSTRUCTOR-AMBIGUOUS", "cannot infer Err(...)'s Ok type here -- this match's scrutinee has no declared Ok side");
                    return None;
                };
                let Some(err_ty) = pattern_ty.cloned() else {
                    self.report.error(span, "E-MATCH-ARM-UNTYPABLE", "this arm's variant has no payload to bind for Err(...) to re-wrap");
                    return None;
                };
                return Some(TypeExpr::Result(Box::new(ok_ty), Box::new(err_ty)));
            }
        }
        // 3. `EnumName::Variant(<pattern>)` -- the general form of (2),
        // for user enums. Needs no "other side" at all: constructing
        // `EnumName::Variant` just needs `EnumName`'s own name and this
        // variant's own declared payload type, both already known.
        if let (Some(pattern), Expr::EnumConstruct { enum_name, variant, payload: Some(inner) }) = (binding, body) {
            if matches!(inner.as_ref(), Expr::Variable(name) if name == pattern) {
                return self.check_enum_construct(enum_name, variant, Some(inner.as_ref()), span);
            }
        }
        // 4. `?`.
        if let Expr::Try(inner) = body {
            return self.check_try(inner, span);
        }
        // 5. A nested `match` -- delegates to `check_match` and accepts
        // *whatever* type it resolves to (Result-shaped, enum-shaped,
        // Pool-shaped, or already-unwrapped/bare), not just a
        // Result-shaped one. This is more general than routing through
        // `infer_result_expr`'s own nested-`Match` case (used elsewhere
        // for genuine `Result`/`?` composability), which only accepts a
        // still-wrapped `Result`: an arm whose body is itself a `match`
        // with every inner arm already unwrapped via `?` (a bare
        // `DnaFile`, say) is just as valid an arm body as one that
        // produces a still-wrapped `Result` -- exactly the same
        // "unwrapped vs. still-wrapped" choice a single arm already has.
        if let Expr::Match { .. } = body {
            return self.check_match(body, span);
        }
        // 6. Result-shaped body.
        if let Some((ok, err)) = self.infer_result_expr(body, span) {
            return Some(TypeExpr::Result(Box::new(ok), Box::new(err)));
        }
        // 7. Enum-shaped body -- new, mirrors (6) for user enums.
        if let Expr::EnumConstruct { enum_name, variant, payload } = body {
            if let Some(ty) = self.check_enum_construct(enum_name, variant, payload.as_deref(), span) {
                return Some(ty);
            }
            return None;
        }
        // 8. Pool-shaped body.
        if let Some(pool_ty) = self.infer_expr(body, span) {
            return Some(TypeExpr::Pool(PoolType { state: pool_ty.state, error_rate_percent: Some(pool_ty.error_rate_percent) }));
        }
        self.report.error(span, "E-MATCH-ARM-UNTYPABLE", "match arm's body must be the pattern's own bound name, a re-wrapped construction of its own variant, a nested match, a Result-shaped expression, an enum-shaped expression, or a Pool-shaped expression");
        None
    }

    /// `EnumName::Variant(<expr>)`/`EnumName::Variant`'s validity (Step
    /// 14) -- looks up the enum (`E-ENUM-CONSTRUCT-UNKNOWN-ENUM` if not
    /// declared), the variant (`E-ENUM-CONSTRUCT-UNKNOWN-VARIANT` if the
    /// enum has no such variant), then checks payload-presence and (if
    /// present) payload-type agreement against the variant's declaration
    /// (`E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH` covers all three failure
    /// shapes: missing a required payload, an unexpected payload on a
    /// unit variant, or a payload of the wrong type).
    fn check_enum_construct(&mut self, enum_name: &str, variant: &str, payload: Option<&Expr>, span: Span) -> Option<TypeExpr> {
        let Some(decl) = self.enums.get(enum_name).cloned() else {
            self.report.error(span, "E-ENUM-CONSTRUCT-UNKNOWN-ENUM", format!("enum '{}' is not declared", enum_name));
            return None;
        };
        let Some(variant_decl) = decl.variants.iter().find(|v| v.name == variant) else {
            self.report.error(span, "E-ENUM-CONSTRUCT-UNKNOWN-VARIANT", format!("'{}' is not a variant of enum '{}'", variant, enum_name));
            return None;
        };
        match (&variant_decl.payload, payload) {
            (None, None) => Some(TypeExpr::Enum(enum_name.to_string())),
            (None, Some(_)) => {
                self.report.error(span, "E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH", format!("'{}::{}' is a unit variant and takes no payload", enum_name, variant));
                None
            }
            (Some(_), None) => {
                self.report.error(span, "E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH", format!("'{}::{}' requires a payload", enum_name, variant));
                None
            }
            (Some(expected_ty), Some(payload_expr)) => match self.infer_value_type(payload_expr, span) {
                Some(ref ty) if ty == expected_ty => Some(TypeExpr::Enum(enum_name.to_string())),
                Some(ty) => {
                    self.report.error(span, "E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH", format!(
                        "'{}::{}' expects a payload of type {}, got {}",
                        enum_name, variant, crate::docgen::render_type(expected_ty), crate::docgen::render_type(&ty)
                    ));
                    None
                }
                None => None, // infer_value_type already reported its own diagnostic
            },
        }
    }

    /// Infers a "value type" for an expression that's meant to become
    /// `Ok(...)`'s payload -- a bare `Variable` (any bound shape:
    /// `Pool`/`Result`/`Str`/whatever `self.bindings` already tracks),
    /// `?`, a `Result`-shaped expression, or a `Pool`-shaped one. Shared
    /// by `check_ok_expr`; not by `check_match_arm`, which has its own
    /// pattern-name special case this helper doesn't need to know about.
    fn infer_value_type(&mut self, expr: &Expr, span: Span) -> Option<TypeExpr> {
        if let Expr::Try(inner) = expr {
            return self.check_try(inner, span);
        }
        // A bare variable is looked up directly against `self.bindings`
        // (every `let`'s own declared annotation, regardless of shape --
        // see its own doc comment) rather than falling through to
        // `infer_result_expr`/`infer_expr` below: those two report a
        // real diagnostic (`E-VARIABLE-UNDECLARED`) when a name isn't
        // Pool-shaped, which is correct when Pool-shaped-ness is actually
        // expected, but wrong here where a `DnaFile`/`File`/`Str`-typed
        // variable (neither Pool- nor Result-shaped) is equally valid.
        if let Expr::Variable(name) = expr {
            return self.bindings.get(name).map(|b| b.annotation.clone());
        }
        if let Some((ok, err)) = self.infer_result_expr(expr, span) {
            return Some(TypeExpr::Result(Box::new(ok), Box::new(err)));
        }
        if let Some(pool_ty) = self.infer_expr(expr, span) {
            return Some(TypeExpr::Pool(PoolType { state: pool_ty.state, error_rate_percent: Some(pool_ty.error_rate_percent) }));
        }
        // A direct (not variable-mediated) user-enum construction --
        // e.g. `Ok(MyEnum::Variant(x))` -- mirrors the Result-shaped case
        // above for user enums (Step 14).
        if let Expr::EnumConstruct { enum_name, variant, payload } = expr {
            return self.check_enum_construct(enum_name, variant, payload.as_deref(), span);
        }
        None
    }

    /// `Ok(<expr>)`'s validity: `<expr>`'s own type becomes the `Ok`
    /// side (via `infer_value_type`); the `Err` side is `expected_err`
    /// if the caller has context (`check_let`'s annotation,
    /// `check_function`/`check_closure_expr`'s declared return type) or
    /// defaults to `Str` -- the only error type anywhere in the language
    /// -- when it doesn't (`infer_result_expr`'s own generic path).
    fn check_ok_expr(&mut self, inner: &Expr, expected_err: Option<&TypeExpr>, span: Span) -> Option<(TypeExpr, TypeExpr)> {
        let Some(ok_ty) = self.infer_value_type(inner, span) else {
            self.report.error(span, "E-OK-CONSTRUCTOR-INVALID", "Ok(...)'s payload must be a bound variable, '?', a Result-shaped expression, or a Pool-shaped expression");
            return None;
        };
        Some((ok_ty, expected_err.cloned().unwrap_or(TypeExpr::Str)))
    }

    /// `Err(<expr>)`'s validity: `<expr>` must be a string literal --
    /// the only way to author a *new* `Str` value (see `Expr::Err`'s doc
    /// comment in ast.rs for why an already-bound `Str` can't be
    /// referenced by name here). The `Ok` side has no sensible default
    /// and must come from `expected_ok` (context) -- `E-ERR-CONSTRUCTOR-
    /// AMBIGUOUS` if there isn't any.
    fn check_err_expr(&mut self, inner: &Expr, expected_ok: Option<&TypeExpr>, span: Span) -> Option<(TypeExpr, TypeExpr)> {
        if !matches!(inner, Expr::StringLiteral(_)) {
            self.report.error(span, "E-ERR-CONSTRUCTOR-INVALID", "Err(...)'s payload must be a string literal -- the only way to author a new Str value");
            return None;
        }
        let Some(ok_ty) = expected_ok.cloned() else {
            self.report.error(span, "E-ERR-CONSTRUCTOR-AMBIGUOUS", "cannot infer Err(...)'s Ok type from context -- annotate the enclosing let binding's Result<T, E>, or use it as the tail of a Result-returning function/closure");
            return None;
        };
        Some((ok_ty, TypeExpr::Str))
    }

    /// `fn(params) -> return_type { body }` in expression position -- see
    /// `Expr::Closure`'s doc comment in ast.rs for the capture rationale.
    /// Returns the closure's own `Fn(param_types, return_type)` type on
    /// success. Deliberately its own small validation block rather than
    /// sharing `check_function`'s return-type-validation verbatim -- see
    /// this project's stated preference for additive changes over
    /// invasive refactors of already-well-tested code.
    /// `type_params` (`fn<T, U>(...)`) needs no special handling in this
    /// function's own body-checking: a `Pool<T>`-typed parameter's
    /// `PoolState::Var` already flows through `pool_bindings` completely
    /// generically (confirmed for named functions when generics first
    /// shipped), so the only place `type_params` actually matters is at
    /// the *call site* (`check_closure_call_args`), which needs the
    /// declared list to check every one was actually resolved -- the
    /// same reason `FunctionDecl::type_params` exists.
    fn check_closure_expr(&mut self, _type_params: &[String], params: &[FnParam], return_type: &TypeExpr, body: &[Declaration], span: Span) -> Option<TypeExpr> {
        let mut param_names = HashSet::new();
        for param in params {
            if !param_names.insert(&param.name) {
                self.report.error(span, "E-PARAM-DUPLICATE", format!("duplicate parameter name '{}' in this closure", param.name));
            }
        }

        // Capture: the closure's own checker starts as a full snapshot of
        // everything already in scope at this literal's own position --
        // params, `let` bindings of any shape, and other already-defined
        // closures. This *is* capture (see `Expr::Closure`'s doc comment
        // for why capture-by-snapshot is simply correct here); `pools`/
        // `functions` are cloned the same way `check_function` already
        // clones them for a top-level function with no outer scope.
        let mut closure_checker = TypeChecker::default();
        closure_checker.pools = self.pools.clone();
        closure_checker.functions = self.functions.clone();
        closure_checker.pool_bindings = self.pool_bindings.clone();
        closure_checker.bindings = self.bindings.clone();
        closure_checker.result_bindings = self.result_bindings.clone();
        closure_checker.strands = self.strands.clone();
        closure_checker.sequences = self.sequences.clone();
        closure_checker.closures = self.closures.clone();
        closure_checker.closure_decls = self.closure_decls.clone();
        closure_checker.fn_param_effects = self.fn_param_effects.clone();
        closure_checker.enums = self.enums.clone();
        closure_checker.enum_bindings = self.enum_bindings.clone();
        // Set from the CLOSURE's own return type, not inherited from the
        // outer scope's `self.enclosing_result_return` -- a `?` inside
        // this closure's body validates against this closure's own
        // signature, exactly like `check_function` sets it from `func`'s
        // own return type regardless of any caller's context.
        if let TypeExpr::Result(ok, err) = return_type {
            closure_checker.enclosing_result_return = Some(((**ok).clone(), (**err).clone()));
        }

        for param in params {
            match &param.ty {
                TypeExpr::Pool(pool_type) => {
                    closure_checker.pool_bindings.insert(
                        param.name.clone(),
                        ProbPoolType { state: pool_type.state.clone(), error_rate_percent: pool_type.error_rate_percent.unwrap_or(0.0) },
                    );
                }
                TypeExpr::Sequence => {
                    closure_checker.sequences.insert(param.name.clone(), span);
                }
                TypeExpr::Strand => {
                    closure_checker.strands.insert(param.name.clone(), span);
                }
                TypeExpr::Fn(param_types, ret, effect) => {
                    closure_checker.closures.insert(param.name.clone(), (Vec::new(), param_types.clone(), (**ret).clone()));
                    if let Some(effect) = effect {
                        closure_checker.fn_param_effects.insert(param.name.clone(), effect.to_effect());
                    }
                }
                // A user-enum-typed parameter (Step 14) -- tracked
                // separately from `bindings` (which only ever holds real
                // `let` declarations) since a parameter has no `LetDecl`
                // of its own. See `TypeChecker::enum_bindings`'s own doc
                // comment.
                TypeExpr::Enum(enum_name) => {
                    closure_checker.enum_bindings.insert(param.name.clone(), enum_name.clone());
                }
                _ => {}
            }
        }

        let mut desugared_body = Vec::new();
        for decl in body {
            desugared_body.extend(closure_checker.check_declaration_single(decl));
        }

        self.report.diagnostics.extend(closure_checker.report.diagnostics);

        // Mirrors `check_function`'s own return-type validation exactly
        // (same two shapes it validates, same "unwrapped tail auto-wraps
        // at the boundary" rule for `Result`) -- see its own doc comment
        // for the full rationale. Anything else (`Void`/`DnaFile`/`File`/
        // `Str`/`Fn`) isn't validated here either, the same pre-existing
        // gap `check_function` already has for those return types.
        let ok = match return_type {
            TypeExpr::Result(expected_ok, expected_err) => match desugared_body.last() {
                Some(Declaration::Let(last_binding))
                    if matches!(last_binding.expr, Expr::Try(_))
                        || (matches!(last_binding.expr, Expr::Match { .. }) && !matches!(last_binding.annotation, TypeExpr::Result(_, _))) =>
                {
                    &last_binding.annotation == expected_ok.as_ref()
                }
                Some(Declaration::Let(last_binding)) => closure_checker
                    .result_bindings
                    .get(&last_binding.name)
                    .is_some_and(|(ok, err)| ok == expected_ok.as_ref() && err == expected_err.as_ref()),
                _ => false,
            },
            TypeExpr::Pool(expected) => match desugared_body.last() {
                Some(Declaration::Let(last_binding)) => {
                    closure_checker.pool_bindings.get(&last_binding.name).is_some_and(|actual| actual.state == expected.state)
                }
                _ => false,
            },
            _ => true,
        };

        if !ok {
            self.report.error(span, "E-CLOSURE-RETURN-TYPE-MISMATCH", format!(
                "this closure is declared to return {} but its body does not produce that",
                crate::docgen::render_type(return_type)
            ));
            return None;
        }
        Some(TypeExpr::Fn(params.iter().map(|p| p.ty.clone()).collect(), Box::new(return_type.clone()), None))
    }

    /// Validates a call's arguments against a closure's own `(param_types,
    /// return_type)` signature -- arity via the same `E-FUNCTION-ARITY` a
    /// named function call already uses, each `Pool<...>`-typed
    /// parameter's argument via the same `E-ARG-TYPE-MISMATCH`, and each
    /// `Fn(...)`-typed parameter's argument via `check_fn_typed_arg`. No
    /// generics (closures are never generic) and no `consensus_vote`-
    /// style intrinsic recognition -- both are named-function-only
    /// concerns. Non-`Pool`/`Fn` parameter types aren't validated here,
    /// the same honest, pre-existing gap the named-function arg-checking
    /// loop above already has for its own non-`Pool`/`Fn` parameters.
    /// Validates a call's arguments against a closure's own signature --
    /// arity via the same `E-FUNCTION-ARITY` a named function call
    /// already uses, each `Pool<...>`-typed parameter's argument via the
    /// *same unification* `infer_expr`'s named-function `FunctionCall`
    /// arm already does (a `PoolState::Var` unifies against the
    /// argument's real concrete state, rather than a flat equality check
    /// -- needed now that a closure can itself be generic, `fn<T>(...)`),
    /// and each `Fn(...)`-typed parameter's argument via
    /// `check_fn_typed_arg`. No `consensus_vote`-style intrinsic
    /// recognition (a named-function-only concern). Returns the
    /// closure's return type with every resolved `PoolState::Var`
    /// substituted in (via the existing `substitute_pool_state` helper
    /// generics already built) -- `None` on an arity mismatch or an
    /// unresolved type parameter (`E-TYPE-PARAM-UNRESOLVED`).
    fn check_closure_call_args(&mut self, name: &str, type_params: &[String], param_types: &[TypeExpr], return_type: &TypeExpr, args: &[Expr], span: Span) -> Option<TypeExpr> {
        if param_types.len() != args.len() {
            self.report.error(span, "E-FUNCTION-ARITY", format!(
                "'{}' expects {} arguments, but {} were provided",
                name, param_types.len(), args.len()
            ));
            return None;
        }
        let mut substitution: HashMap<String, PoolState> = HashMap::new();
        for (expected, arg) in param_types.iter().zip(args.iter()) {
            match expected {
                TypeExpr::Pool(expected_pool) => {
                    let Some(inferred_arg) = self.infer_expr(arg, span) else {
                        self.report.error(span, "E-ARG-TYPE-INVALID", format!("an argument for '{}' must be a Pool type", name));
                        continue;
                    };
                    match &expected_pool.state {
                        PoolState::Var(t) => match substitution.get(t) {
                            Some(bound) if *bound != inferred_arg.state => {
                                self.report.error(span, "E-TYPE-PARAM-CONFLICT", format!(
                                    "type parameter '{}' was already resolved to Pool<{}> by an earlier argument, but an argument for '{}' implies Pool<{}>",
                                    t, bound, name, inferred_arg.state
                                ));
                            }
                            Some(_) => {}
                            None => {
                                substitution.insert(t.clone(), inferred_arg.state.clone());
                            }
                        },
                        _ => {
                            if expected_pool.state != inferred_arg.state {
                                self.report.error(span, "E-ARG-TYPE-MISMATCH", format!(
                                    "an argument for '{}' expects Pool<{}>, but got Pool<{}>",
                                    name, expected_pool.state, inferred_arg.state
                                ));
                            }
                        }
                    }
                }
                TypeExpr::Fn(expected_params, expected_return, expected_effect) => {
                    self.check_fn_typed_arg(expected_params, expected_return, *expected_effect, arg, name, span);
                }
                _ => {}
            }
        }
        for t in type_params {
            if !substitution.contains_key(t) {
                self.report.error(span, "E-TYPE-PARAM-UNRESOLVED", format!(
                    "type parameter '{}' of closure '{}' could not be resolved from any argument",
                    t, name
                ));
                return None;
            }
        }
        Some(substitute_pool_state(return_type, &substitution))
    }

    /// Validates one `Fn(...)`-typed argument -- either a bare
    /// `Expr::Variable` naming an already-bound closure (looked up in
    /// `self.closures`) or an inline `Expr::Closure` literal (type-checked
    /// on the spot via `check_closure_expr`) -- against the parameter's
    /// declared signature. Reuses `E-ARG-TYPE-MISMATCH` for both "wrong
    /// signature" and "not a closure at all" -- the same code an ordinary
    /// mismatched argument already reports.
    fn check_fn_typed_arg(&mut self, expected_params: &[TypeExpr], expected_return: &TypeExpr, expected_effect: Option<FnEffectAnnotation>, arg: &Expr, param_name: &str, span: Span) {
        let actual = match arg {
            // Only the signature matters for this structural comparison
            // -- `type_params` is dropped. A generic closure argument's
            // own `Var`s are compared by name via `PartialEq` (`Var("T")
            // == Var("T")`), the same structural check that already lets
            // two identically-named-but-independently-declared type
            // parameters compare equal elsewhere in this file.
            Expr::Variable(name) => self.closures.get(name).cloned().map(|(_, params, ret)| (params, ret)),
            Expr::Closure { type_params, params, return_type, body, span: closure_span } => {
                match self.check_closure_expr(type_params, params, return_type, body, *closure_span) {
                    Some(TypeExpr::Fn(actual_params, actual_return, _)) => Some((actual_params, *actual_return)),
                    _ => return,
                }
            }
            _ => None,
        };
        match actual {
            Some((actual_params, actual_return)) => {
                if actual_params != expected_params || &actual_return != expected_return {
                    self.report.error(span, "E-ARG-TYPE-MISMATCH", format!(
                        "argument for parameter '{}' expects Fn({}) -> {}, but got Fn({}) -> {}",
                        param_name,
                        expected_params.iter().map(crate::docgen::render_type).collect::<Vec<_>>().join(", "),
                        crate::docgen::render_type(expected_return),
                        actual_params.iter().map(crate::docgen::render_type).collect::<Vec<_>>().join(", "),
                        crate::docgen::render_type(&actual_return),
                    ));
                } else {
                    self.check_fn_effect_compatibility(expected_effect, arg, span);
                }
            }
            None => {
                self.report.error(span, "E-ARG-TYPE-MISMATCH", format!("argument for parameter '{}' must be a closure", param_name));
            }
        }
    }

    /// The soundness-critical check that makes an effect-annotated
    /// `Fn(...)` type actually sound, not just decorative: whenever a
    /// concrete closure expression is bound into an annotated slot (an
    /// argument passed to an annotated parameter, or a closure literal
    /// assigned to an annotated `let`), its own *real* effect -- computed
    /// the identical way any named function's/let-bound closure's call
    /// already is, via `effects::function_call_effect`/`effects::
    /// body_effect` -- must already be internally confirmed and must
    /// fall within the declared ceiling. This single check, reused at
    /// every concrete-binding site, is what lets a call to the
    /// *parameter itself* later be trusted as `(declared_effect,
    /// confirmed=true)` inside `effects.rs` (see `fn_param_effects`)
    /// without ever needing to see the real body at that inner call site
    /// -- by induction, every value that could ever reach it was already
    /// checked here, wherever it was concretely bound.
    ///
    /// `expr` is resolved the same way regardless of shape -- a literal
    /// closure's body is joined directly via `effects::body_effect`; a
    /// named reference resolves via `effects::function_call_effect`,
    /// which itself already checks `closure_decls` (a real body) before
    /// `fn_param_effects` (a trusted ceiling from an *enclosing*
    /// annotated parameter) before falling back to today's unchanged
    /// `Pure`/confirmed default for anything unannotated -- so this one
    /// call correctly handles a literal, a `let`-bound closure, a
    /// captured outer parameter, and a forwarded parameter alike, with
    /// no special-casing for any of them.
    fn check_fn_effect_compatibility(&mut self, expected: Option<FnEffectAnnotation>, expr: &Expr, span: Span) {
        let Some(expected) = expected else { return };
        let (actual_effect, actual_confirmed) = match expr {
            Expr::Closure { params, body, .. } => {
                // The literal's own annotated `Fn(...)`-typed parameters
                // (if it takes any) are scoped in too, exactly like
                // `function_call_effect` already does for a named
                // function/closure -- so a closure literal that itself
                // takes an effect-annotated parameter resolves calls to
                // it correctly, not just calls to whatever it captures.
                let scoped = crate::effects::scoped_fn_param_effects(params, &self.fn_param_effects);
                crate::effects::body_effect(body, &self.functions, &scoped, &mut HashSet::new())
            }
            Expr::Variable(name) => {
                crate::effects::function_call_effect(name, &self.functions, &self.closure_decls, &self.fn_param_effects, &mut HashSet::new())
            }
            _ => return,
        };
        let confirmed_enough = actual_effect == Effect::Pure || actual_confirmed;
        if !crate::effects::effect_satisfies_annotation(actual_effect, expected) {
            self.report.error(span, "E-FN-EFFECT-ARG-MISMATCH", format!(
                "this closure's real effect is {}, which doesn't fall within the parameter's declared {}",
                actual_effect,
                match expected { FnEffectAnnotation::Hardware => "confirm hardware", FnEffectAnnotation::PhysicalKey => "confirm physical_key" },
            ));
        } else if !confirmed_enough {
            self.report.error(span, "E-FN-EFFECT-ARG-MISMATCH", format!(
                "this closure has {} effect but isn't itself confirmed (missing its own confirm inside its body)",
                actual_effect,
            ));
        }
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
            Expr::FunctionCall { name, args, explicit_type_args } => {
                // Closures resolve first: a name bound to a `let`-closure
                // or a `Fn(...)`-typed parameter shadows a same-named
                // global function/stdlib entry, exactly like typeck
                // resolves it consistently for `codegen::eval_expr`'s
                // own env-before-`funcs` lookup order. A name in neither
                // table falls through to the pre-existing
                // `E-FUNCTION-UNDECLARED` below, completely unaffected.
                if let Some((type_params, param_types, return_type)) = self.closures.get(name).cloned() {
                    let resolved_return = self.check_closure_call_args(name, &type_params, &param_types, &return_type, args, span);
                    return match resolved_return {
                        Some(TypeExpr::Pool(pool_type)) => Some(ProbPoolType {
                            state: pool_type.state.clone(),
                            error_rate_percent: pool_type.error_rate_percent.unwrap_or(0.0),
                        }),
                        _ => None,
                    };
                }
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
                // Generics: a `Pool<T>`-typed parameter's `PoolState::Var`
                // is unified against each argument's real concrete state
                // here, building `substitution`. The pre-existing
                // concrete-vs-concrete branch (now the `_` arm below) is
                // completely untouched.
                let mut substitution: HashMap<String, PoolState> = HashMap::new();
                // `name::<Illumina, Nanopore>(...)` -- explicit type
                // arguments, seeded *before* the per-argument inference
                // loop below so an explicit argument counts exactly like
                // an inferred one (a later inferred argument that
                // disagrees still reports `E-TYPE-PARAM-CONFLICT`). Only
                // needed when a type parameter can't be inferred from any
                // argument; a wrong count is `E-TYPE-PARAM-ARITY`.
                if !explicit_type_args.is_empty() {
                    if explicit_type_args.len() != func.type_params.len() {
                        self.report.error(span, "E-TYPE-PARAM-ARITY", format!(
                            "'{}' declares {} type parameter(s), but {} explicit type argument(s) were provided",
                            name, func.type_params.len(), explicit_type_args.len()
                        ));
                    } else {
                        for (t, profile) in func.type_params.iter().zip(explicit_type_args.iter()) {
                            substitution.insert(t.clone(), PoolState::Profile(*profile));
                        }
                    }
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
                        match &expected_pool.state {
                            PoolState::Var(t) => match substitution.get(t) {
                                Some(bound) if *bound != inferred_arg.state => {
                                    self.report.error(span, "E-TYPE-PARAM-CONFLICT", format!(
                                        "type parameter '{}' was already resolved to Pool<{}> by an earlier argument, but argument for parameter '{}' implies Pool<{}>",
                                        t, bound, param.name, inferred_arg.state
                                    ));
                                }
                                Some(_) => {}
                                None => {
                                    substitution.insert(t.clone(), inferred_arg.state.clone());
                                }
                            },
                            _ => {
                                if expected_pool.state != inferred_arg.state {
                                    self.report.error(span, "E-ARG-TYPE-MISMATCH", format!(
                                        "argument for parameter '{}' expects Pool<{}>, but got Pool<{}>",
                                        param.name, expected_pool.state, inferred_arg.state
                                    ));
                                }
                            }
                        }
                    } else if let TypeExpr::Fn(expected_params, expected_return, expected_effect) = &param.ty {
                        self.check_fn_typed_arg(expected_params, expected_return, *expected_effect, arg, &param.name, span);
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

                // A type parameter that no argument ever bound can't
                // produce a concrete return type -- there's no explicit
                // type-argument syntax to fall back on, so this is a
                // real error, not a case to silently leave unresolved.
                for t in &func.type_params {
                    if !substitution.contains_key(t) {
                        self.report.error(span, "E-TYPE-PARAM-UNRESOLVED", format!(
                            "type parameter '{}' of function '{}' could not be resolved from any argument",
                            t, name
                        ));
                        return None;
                    }
                }

                let return_type = substitute_pool_state(&func.return_type, &substitution);
                match &return_type {
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
            // `Match` can (rarely) itself be Pool-shaped -- see
            // `check_match_arm`'s own `infer_expr` fallback -- but that's
            // resolved through `check_match`, never by recursing into
            // this function, so it's `None` here too, same as `Try`.
            // `Closure` is `Fn(...)`-shaped, never `Pool<...>`-shaped --
            // resolved through `check_closure_expr`/`self.closures`,
            // never by recursing into this function.
            // `Ok`/`Err` are `Result`-shaped, never `Pool<...>`-shaped --
            // resolved through `infer_result_expr`/`infer_value_type`,
            // never by recursing into this function.
            Expr::Try(_)
            | Expr::StoreExpr(_)
            | Expr::RetrieveExpr(_)
            | Expr::DeleteExpr(_)
            | Expr::Match { .. }
            | Expr::Closure { .. }
            | Expr::Ok(_)
            | Expr::Err(_)
            | Expr::EnumConstruct { .. } => None,
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
        body_checker.enums = self.enums.clone();
        body_checker.enum_bindings = self.enum_bindings.clone();
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
                // A `Fn(...)`-typed parameter -- what makes calling
                // `func` "higher-order": whatever closure the caller
                // passes becomes callable by this parameter's own name
                // inside the body, resolved through `infer_expr`/
                // `infer_result_expr`'s closures-first `FunctionCall`
                // lookup exactly like a `let`-bound closure already is.
                TypeExpr::Fn(param_types, return_type, effect) => {
                    body_checker.closures.insert(param.name.clone(), (Vec::new(), param_types.clone(), (**return_type).clone()));
                    if let Some(effect) = effect {
                        body_checker.fn_param_effects.insert(param.name.clone(), effect.to_effect());
                    }
                }
                // A user-enum-typed parameter (Step 14) -- see
                // `TypeChecker::enum_bindings`'s own doc comment.
                TypeExpr::Enum(enum_name) => {
                    body_checker.enum_bindings.insert(param.name.clone(), enum_name.clone());
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
                Some(Declaration::Let(last_binding))
                    if matches!(last_binding.expr, Expr::Try(_))
                        || (matches!(last_binding.expr, Expr::Match { .. }) && !matches!(last_binding.annotation, TypeExpr::Result(_, _))) =>
                {
                    // The inner fallible expression's Err type was already
                    // checked against `expected_err` by `check_try` (via
                    // `enclosing_result_return`, set above) while checking
                    // this binding -- only the Ok side is left to verify
                    // here, against the function's declared Ok type
                    // specifically (not just "whatever `?` unwrapped to",
                    // which `check_let` already confirmed equals this
                    // binding's own annotation). A `match` tail whose
                    // unified type isn't itself `Result<...>` (i.e. both
                    // arms already unwrapped, e.g. `Ok(x) => x`) is the
                    // exact same shape -- `check_match`/`check_let` already
                    // validated it, only the Ok side needs checking here.
                    // A `match` tail whose unified type *is* `Result<...>`
                    // (both arms still-wrapped) instead falls through to
                    // the generic arm below, since `check_let`'s `Match`
                    // branch already populated `result_bindings` for it.
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
            type_params: func.type_params.clone(),
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
        | Declaration::Function(_)
        | Declaration::Enum(_) => decl.clone(),
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
        Expr::FunctionCall { name, args, explicit_type_args } => Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(|arg| substitute_expr(arg, binding, value)).collect(),
            explicit_type_args: explicit_type_args.clone(),
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
        // The pattern names (`MatchArm::binding`) are a separate,
        // function-body-local namespace from the `for`-loop's substituted
        // `binding` -- only the scrutinee and arm bodies can reference
        // it, so only they get recursed into.
        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(substitute_expr(scrutinee, binding, value)),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    variant: arm.variant.clone(),
                    binding: arm.binding.clone(),
                    body: Box::new(substitute_expr(&arm.body, binding, value)),
                    span: arm.span,
                })
                .collect(),
        },
        // Params are their own local namespace (same shadowing model as
        // any nested `let` reusing the loop variable's name) -- only the
        // body's declarations get recursed into.
        Expr::Closure { type_params, params, return_type, body, span } => Expr::Closure {
            type_params: type_params.clone(),
            params: params.clone(),
            return_type: return_type.clone(),
            body: body.iter().map(|decl| substitute_declaration(decl, binding, value)).collect(),
            span: *span,
        },
        Expr::Ok(inner) => Expr::Ok(Box::new(substitute_expr(inner, binding, value))),
        Expr::Err(inner) => Expr::Err(Box::new(substitute_expr(inner, binding, value))),
        Expr::EnumConstruct { enum_name, variant, payload } => Expr::EnumConstruct {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: payload.as_ref().map(|inner| Box::new(substitute_expr(inner, binding, value))),
        },
    }
}

/// Resolves a generic function's return type for one specific call, by
/// replacing any `PoolState::Var` it contains with the concrete state
/// `substitution` unified for that variable at this call site (see
/// `infer_expr`'s `FunctionCall` arm). A no-op for a non-generic
/// function's return type (nothing to substitute), and for a `Var` that
/// -- despite `E-TYPE-PARAM-UNRESOLVED` guarding against this at the one
/// call site above -- somehow isn't in `substitution`, in which case the
/// type is returned unchanged rather than panicking. Only `TypeExpr::Pool`
/// can ever contain a `PoolState`, so this is the only case that does
/// anything.
fn substitute_pool_state(ty: &TypeExpr, substitution: &HashMap<String, PoolState>) -> TypeExpr {
    match ty {
        TypeExpr::Pool(pool_type) => match &pool_type.state {
            PoolState::Var(t) => match substitution.get(t) {
                Some(resolved) => TypeExpr::Pool(PoolType { state: resolved.clone(), error_rate_percent: pool_type.error_rate_percent }),
                None => ty.clone(),
            },
            _ => ty.clone(),
        },
        _ => ty.clone(),
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
                        explicit_type_args: Vec::new(),
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
