//! Abstract syntax tree for NucleScript.

use serde::{Deserialize, Serialize};

/// A source location range, in 1-based line/column coordinates matching
/// `lexer::Token`. Every top-level declaration (and the operations nested
/// inside one) carries its own `Span` so diagnostics produced anywhere in
/// the pipeline (typeck, effects) can point back at the exact source text
/// that caused them, instead of just naming the construct by value (e.g.
/// "pool 'archive'") and leaving the reader to search for it.
///
/// `Span::default()` (all zeros) marks a synthetic node with no real
/// source position -- used only by hand-built `Program`s in tests, never
/// produced by the parser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl Span {
    /// A span covering just the single point `(line, column)` -- used
    /// before an end position is known.
    pub fn point(line: usize, column: usize) -> Self {
        Self { line, column, end_line: line, end_column: column }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Declaration {
    Import(ImportDecl),
    Pool(PoolDecl),
    Strand(StrandDecl),
    Sequence(SequenceDecl),
    Let(LetDecl),
    Operation(Operation),
    Pipeline(PipelineDecl),
    Function(FunctionDecl),
    If(IfDecl),
    For(ForDecl),
    Test(TestDecl),
    Enum(EnumDecl),
}

impl Declaration {
    /// The source span of this declaration, regardless of variant --
    /// callers that just need "where does this diagnostic point" shouldn't
    /// have to match on every declaration kind themselves.
    pub fn span(&self) -> Span {
        match self {
            Declaration::Import(d) => d.span,
            Declaration::Pool(d) => d.span,
            Declaration::Strand(d) => d.span,
            Declaration::Sequence(d) => d.span,
            Declaration::Let(d) => d.span,
            Declaration::Operation(op) => op.span(),
            Declaration::Pipeline(d) => d.span,
            Declaration::Function(d) => d.span,
            Declaration::If(d) => d.span,
            Declaration::For(d) => d.span,
            Declaration::Test(d) => d.span,
            Declaration::Enum(d) => d.span,
        }
    }
}

/// `test "description" { ... }` -- a named block of declarations run by
/// `nucle test` against a fresh, isolated `NucleOS` instance per test
/// (see `test_runner.rs`). Unlike `if`/`for`, a test's body is NOT
/// resolved away during type-checking: `typeck::check_and_desugar` still
/// desugars any `if`/`for` *inside* it, but the `TestDecl` itself survives
/// into the output program so the test runner has something to find and
/// execute. `assert` statements inside the body (see `AssertOp`) are
/// evaluated during type-checking, the same way an `if` condition is --
/// NucleScript's probabilistic properties are deterministic formulas
/// computed at compile time, not measured empirically, so there's nothing
/// to defer an assertion to a later "runtime" phase for. Real `store`/
/// `retrieve`/`delete` operations in the body still execute for real
/// against the VFS, so a test can also catch genuine execution failures
/// (a `retrieve`/`delete` erroring out), not just failed assertions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestDecl {
    pub name: String,
    pub body: Vec<Declaration>,
    pub span: Span,
}

/// `if condition { ... } else { ... }` -- resolved at compile time, not a
/// runtime branch: NucleScript's execution model is "compile a static
/// plan, then run it," and `condition` is always evaluable from
/// already-known probabilistic pool types, so there's no need to invent
/// runtime branching to support it. `typeck::check_program` evaluates
/// `condition` once, type-checks and keeps *only* the taken branch
/// (extending the enclosing scope, the same way a function's parameters
/// extend its body's scope) and produces a `Program` with every `If`
/// already resolved away -- `codegen`/`middle`/`sim_backend` never see
/// this variant. This mirrors Rust's `#[cfg(...)]` more than a runtime
/// `if`: the untaken branch is not type-checked at all, not merely
/// skipped at "runtime".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfDecl {
    pub condition: Expr,
    pub then_branch: Vec<Declaration>,
    pub else_branch: Option<Vec<Declaration>>,
    pub span: Span,
}

/// `for binding in [item, ...] { ... }` -- always over a literal,
/// statically-known list (of identifiers and/or string literals, matching
/// the same "StringLiteral | Identifier -> one String" convention
/// `StoreOp`/`DeleteOp`/etc. already use), never an open-ended `while`.
/// Resolved by `typeck::check_program` via substitution: `binding` is
/// textually replaced by each item's value in a fresh copy of `body`,
/// each copy is type-checked and concatenated into the output program --
/// same "compile-time construct, not runtime" reasoning as `IfDecl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForDecl {
    pub binding: String,
    pub items: Vec<String>,
    pub body: Vec<Declaration>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub source: String,
    pub items: Vec<ImportItem>,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolDecl {
    pub name: String,
    pub codec: Codec,
    pub redundancy: usize,
    pub profile: Profile,
    #[serde(default)]
    pub span: Span,
    /// The `///` doc comment immediately preceding this declaration, if
    /// any -- consumed by `docgen` (`nucle doc`), never by type-checking.
    /// Consecutive `///` lines are joined with `\n` into one string; see
    /// `lexer::TokenKind::DocComment`.
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrandDecl {
    pub name: String,
    pub sequence: String,
    #[serde(default)]
    pub span: Span,
    /// See `PoolDecl::doc`.
    #[serde(default)]
    pub doc: Option<String>,
}

/// `enum Name { Variant1, Variant2(PayloadType), ... }` -- a user-defined
/// sum type (Step 14). Unlike `Result<T, E>` (which stays its own
/// privileged `TypeExpr::Result`/`Expr::Ok`/`Expr::Err` machinery, never
/// registered here), an `EnumDecl` is looked up by name from
/// `typeck::TypeChecker::enums`, exactly the way `PoolDecl`/`FunctionDecl`
/// are looked up from `self.pools`/`self.functions`. `enum Result { ... }`
/// is rejected (`E-ENUM-RESERVED-NAME`) precisely because Result is not
/// an instance of this general mechanism, just uniformly *matchable*
/// alongside one (see `typeck::TypeChecker::check_match`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    #[serde(default)]
    pub span: Span,
    /// See `PoolDecl::doc`.
    #[serde(default)]
    pub doc: Option<String>,
}

/// One variant of a user-declared `enum`. At most one payload type --
/// mirrors `Ok(T)`/`Err(E)`'s own shape exactly, deliberately not a tuple
/// or struct-like multi-field variant (see the Step 14 plan's own scope
/// discussion for why: every downstream consumer -- pattern binding,
/// re-wrap construction, runtime `Value::EnumInstance` -- is written
/// around "zero or one payload value per variant," and nothing in this
/// language's actual domain needs more than that yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    /// `None` for a unit variant (`Retry`); `Some(ty)` for a
    /// single-payload variant (`GiveUp(Str)`).
    pub payload: Option<TypeExpr>,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceDecl {
    pub name: String,
    pub sequence: String,
    #[serde(default)]
    pub span: Span,
    /// See `PoolDecl::doc`.
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetDecl {
    pub name: String,
    pub annotation: TypeExpr,
    pub expr: Expr,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub name: String,
    /// `fn name<T, U>(...)` -- type parameter names, usable only as the
    /// `PoolState` slot inside a `Pool<T>` parameter/return type (see
    /// `PoolState::Var`). Empty for every non-generic function, which is
    /// every function that existed before this field was added.
    #[serde(default)]
    pub type_params: Vec<String>,
    pub params: Vec<FnParam>,
    pub return_type: TypeExpr,
    pub body: Vec<Declaration>,
    #[serde(default)]
    pub span: Span,
    /// See `PoolDecl::doc`.
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnParam {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeExpr {
    Pool(PoolType),
    Strand,
    Sequence,
    File,
    DnaFile,
    Recovery,
    Void,
    /// `Result<T, E>` -- the one generic type NucleScript has (no general
    /// `Type<...>` mechanism exists; `Pool<Illumina>` above is its own
    /// hardcoded parse path, unrelated to this). `Box` because `TypeExpr`
    /// is now recursive, matching how `Expr::BinaryOp` already boxes.
    Result(Box<TypeExpr>, Box<TypeExpr>),
    /// A plain string error message -- meaningful only as `Result<_,
    /// Str>`'s error slot (every VFS failure is a `String`; see
    /// `nucle_vfs::syscall`). Not a general string type: there is no
    /// string arithmetic, no other place `Str` is expected, and nothing
    /// enforces that restriction beyond it simply being useless anywhere
    /// else today -- deliberately the smallest addition that keeps a
    /// Result's error side real instead of collapsing it to `Void`.
    Str,
    /// `Fn(ParamType, ...) -> ReturnType` -- a closure/function's own
    /// type, usable as a `let` annotation or a function parameter's type
    /// (what makes a function "higher-order"). Non-generic: a closure
    /// literal (`Expr::Closure`) always has a fixed, concrete signature,
    /// never its own `type_params`. Capitalized to match `Pool`/`Result`/
    /// `Str`'s existing type-name convention, distinct from the lowercase
    /// `fn` keyword used for both named declarations and closure literals.
    Fn(Vec<TypeExpr>, Box<TypeExpr>),
    /// Names a user-declared `enum` by name (Step 14), resolved against
    /// `typeck::TypeChecker::enums` at type-check time
    /// (`E-ENUM-UNKNOWN` if it doesn't resolve). `Result<T, E>` is NOT
    /// represented this way -- it keeps its own dedicated
    /// `TypeExpr::Result` variant unchanged; only the *matching* logic
    /// (`check_match`/`eval_expr`) treats Result and a `TypeExpr::Enum`
    /// uniformly, never the type system itself.
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolType {
    pub state: PoolState,
    pub error_rate_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolState {
    Profile(Profile),
    Amplified,
    Recovered,
    /// An unbound generic type parameter (e.g. the `T` in `fn foo<T>(x:
    /// Pool<T>)`) -- never constructed by `PoolState::parse` (which only
    /// ever produces a concrete state from a string), only by the parser
    /// recognizing a name already declared in the enclosing function's
    /// `FunctionDecl::type_params`. Resolved to a concrete `PoolState` at
    /// each call site via unification against the argument's real
    /// inferred state (see `typeck::TypeChecker::infer_expr`'s
    /// `FunctionCall` arm) -- never exists past type-checking, so nothing
    /// downstream of typeck (effects, codegen, the interpreter) ever
    /// needs to handle it.
    Var(String),
}

impl PoolState {
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(profile) = Profile::parse(value) {
            return Some(Self::Profile(profile));
        }
        match value.to_ascii_lowercase().as_str() {
            "amplified" => Some(Self::Amplified),
            "recovered" => Some(Self::Recovered),
            _ => None,
        }
    }
}

impl std::fmt::Display for PoolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Profile(profile) => write!(f, "{}", profile),
            Self::Amplified => write!(f, "Amplified"),
            Self::Recovered => write!(f, "Recovered"),
            Self::Var(name) => write!(f, "{}", name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    SimulatePool { pool: String, profile: Profile },
    SynthesizePool {
        source: String,
        profile: Profile,
        confirmed: bool,
    },
    SequencePool {
        source: String,
        profile: Profile,
        confirmed: bool,
    },
    /// A call to a user-defined `fn`, or to a built-in like
    /// `consensus_vote`/`protect` -- both are ordinary `FunctionTable`
    /// entries (see `stdlib::builtin_functions`), resolved through the
    /// exact same lookup, so there's only ever this one call
    /// representation, not a separate AST node per built-in. The parser
    /// still accepts `consensus_vote(...)`'s and `protect ... for ...`'s
    /// friendly surface syntax, desugaring both to this variant at parse
    /// time (see `parser::parse_primary_expr`).
    FunctionCall {
        name: String,
        args: Vec<Expr>,
        /// `name::<Illumina, Nanopore>(...)` -- explicit type arguments,
        /// only needed when a generic function's type parameter can't be
        /// inferred from any argument (empty for every non-turbofish
        /// call, which is every call that existed before this field was
        /// added). Zipped against the callee's own `type_params` at the
        /// call site (`typeck::TypeChecker::infer_expr`'s `FunctionCall`
        /// arm) to seed the same unification `substitution` an inferred
        /// argument would otherwise populate.
        explicit_type_args: Vec<Profile>,
    },
    Variable(String),
    StringLiteral(String),
    /// A bare number literal in expression position, e.g. the `5.0` in
    /// `noisy > 5.0`. Distinct from the multiplier/percent/size-in-bytes
    /// literal forms parsed contextually elsewhere in the grammar
    /// (`3x`, `99.5%`, `10MB`) -- this is a plain, suffix-free number.
    Number(f64),
    BinaryOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Not(Box<Expr>),
    /// `expr?` -- unwraps a `Result<T,E>`-shaped `expr` to its `Ok(T)`
    /// value, or short-circuits the enclosing function with its `Err(E)`.
    /// See `typeck::TypeChecker::check_try` for the validity rules and
    /// `codegen::eval_expr`/`sim_backend`'s equivalent for the runtime
    /// short-circuit itself.
    Try(Box<Expr>),
    /// `store <file> into <pool> { ... }` used in *expression* position
    /// (e.g. the right-hand side of a `let`), reusing the exact same
    /// `StoreOp` the statement form (`Declaration::Operation(Operation::
    /// Store)`) already carries -- one struct, two surface positions.
    /// Produces a `Result<DnaFile, Str>` at runtime instead of the
    /// statement form's all-or-nothing abort-the-whole-program behavior.
    StoreExpr(StoreOp),
    /// `retrieve from <pool> where ...` in expression position. Parsed
    /// for symmetry with `StoreExpr`/`DeleteExpr`, but typeck never
    /// infers it as `Result`-shaped: `retrieve` already soft-fails today
    /// (an empty match list, never a VFS `Err`), so there's no real
    /// failure for a `Result` to carry.
    RetrieveExpr(RetrieveOp),
    /// `delete <file> from <pool> confirm ...` in expression position --
    /// same relationship to `Declaration::Operation(Operation::Delete)`
    /// as `StoreExpr` has to `Operation::Store`.
    DeleteExpr(DeleteOp),
    /// `match <scrutinee> { <arm>, ... }` -- the general pattern-matching
    /// engine (Step 14). `scrutinee` must resolve to either the built-in
    /// `Result<T, E>` "pseudo-enum" (its two implicit variants are always
    /// exactly `Ok(T)`/`Err(E)`) or a user-declared `TypeExpr::Enum`
    /// looked up in `self.enums` -- see `typeck::TypeChecker::check_match`
    /// for exactly how. Every declared variant needs exactly one arm (by
    /// name) or a trailing wildcard covering the rest; an arm naming an
    /// unknown variant, a non-exhaustive set with no wildcard, a wildcard
    /// that isn't last, or two arms naming the same variant are all
    /// rejected (see the `E-MATCH-*` diagnostics). Arm order in `arms` is
    /// preserved from source but is no longer semantically fixed the way
    /// the old two-field shape hardcoded "Ok then Err" -- exhaustiveness
    /// is checked by name against the scrutinee's own declared variant
    /// list, not by position, so `Err` may now appear before `Ok`.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `fn(params) -> ReturnType { body }` in expression position -- an
    /// anonymous closure literal, never a top-level `Declaration`. Has no
    /// `name` -- it's identified only by whatever `let`/parameter binds
    /// it, exactly like any other value. Capture is real and lexical:
    /// `typeck::TypeChecker::check_closure_expr` type-checks `body`
    /// against a snapshot of every binding already in scope at this
    /// literal's own position (see its doc comment for why capture-by-
    /// snapshot is simply correct here, not a design compromise --
    /// NucleScript's `let` bindings are single-assignment, so there is no
    /// "later mutation" a by-value/by-reference distinction could ever
    /// observe). `codegen::eval_expr`'s `Expr::Closure` arm is the runtime
    /// counterpart: capture there is just one `env.clone()`.
    Closure {
        /// `fn<T, U>(...)` -- mirrors `FunctionDecl::type_params` exactly
        /// (empty for every non-generic closure). Resolved the same way:
        /// call-site unification against a `Pool<T>`-typed argument's
        /// real concrete state, never a runtime representation.
        type_params: Vec<String>,
        params: Vec<FnParam>,
        return_type: TypeExpr,
        body: Vec<Declaration>,
        span: Span,
    },
    /// `Ok(<expr>)` -- constructs a successful `Result`. `<expr>`'s own
    /// type becomes the `Ok` side; the `Err` side defaults to `Str` (the
    /// only error type anywhere in the language) unless external context
    /// says otherwise. See `typeck::TypeChecker::infer_value_type` for
    /// what `<expr>` can be, and `codegen::eval_expr`'s `Expr::Ok` arm
    /// for the runtime counterpart.
    Ok(Box<Expr>),
    /// `Err(<string-literal>)` -- constructs a failed `Result`. The
    /// payload is restricted to a string literal: it's the only way to
    /// author a *new* `Str` value (an already-bound `Str`, e.g. a
    /// `match`-captured `Err(reason)` pattern variable, was deliberately
    /// never registered in any typeck scope map -- see `Expr::Match`'s
    /// doc comment). The `Ok` side has no sensible default and must come
    /// from context (an enclosing `let`'s annotation, a sibling `match`
    /// arm, or the enclosing function/closure's declared return type) --
    /// `E-ERR-CONSTRUCTOR-AMBIGUOUS` if none is available.
    Err(Box<Expr>),
    /// `EnumName::Variant(<expr>)` / `EnumName::Variant` (bare, for a unit
    /// variant) -- constructs an instance of a user-declared `enum`
    /// (Step 14). Reuses the `::` token already added for
    /// `name::<Illumina>(...)` turbofish calls (Step 13); disambiguated
    /// at parse time by what follows `::` (`<` means turbofish, an
    /// identifier means a variant name -- see
    /// `parser::parse_primary_expr`). Deliberately NOT how `Ok`/`Err` are
    /// represented -- those stay their own dedicated `Expr` variants,
    /// unprefixed, exactly as before (see their own doc comments for why:
    /// `Err`'s string-literal-only payload restriction and the unprefixed
    /// surface syntax are both irreducibly Result-specific). This is the
    /// general construction path for every *other* enum. See
    /// `typeck::TypeChecker::check_enum_construct` for validity rules and
    /// `codegen::eval_expr`'s `Expr::EnumConstruct` arm for the runtime
    /// counterpart.
    EnumConstruct {
        enum_name: String,
        variant: String,
        payload: Option<Box<Expr>>,
    },
}

/// One arm of a general `match` (Step 14). `variant: None` marks a
/// wildcard `_` arm, which must be the last arm if present
/// (`E-MATCH-ARM-AFTER-WILDCARD` otherwise) -- see `Expr::Match`'s doc
/// comment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    /// `Some("Ok")`/`Some("Err")` for Result, `Some("Fallback")` etc. for
    /// a user enum's variant, `None` for a wildcard `_` arm.
    pub variant: Option<String>,
    /// The pattern's bound name, e.g. `file` in `Ok(file) => ...`. `None`
    /// for a unit variant's arm (`Retry => ...`, nothing to bind) or a
    /// wildcard with no capture (`_ => ...`).
    pub binding: Option<String>,
    pub body: Box<Expr>,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Pure,
    Synthesis,
    Sequencing,
    Destructive,
}

impl std::fmt::Display for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Pure => "Pure",
            Self::Synthesis => "Synthesis",
            Self::Sequencing => "Sequencing",
            Self::Destructive => "Destructive",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Store(StoreOp),
    Retrieve(RetrieveOp),
    Delete(DeleteOp),
    Assert(AssertOp),
}

impl Operation {
    pub fn span(&self) -> Span {
        match self {
            Operation::Store(op) => op.span,
            Operation::Retrieve(op) => op.span,
            Operation::Delete(op) => op.span,
            Operation::Assert(op) => op.span,
        }
    }
}

/// `assert <condition>` or `assert <condition>, "message"` -- evaluated
/// during type-checking via the same `eval_condition` machinery an `if`
/// condition uses (see `TestDecl`'s doc comment for why that's the right
/// place, not a deferred "runtime" check). A false condition is reported
/// as an `E-ASSERTION-FAILED` diagnostic at this statement's span,
/// regardless of whether it's lexically inside a `test { ... }` block --
/// `nucle check` surfaces an always-false assertion anywhere in a program
/// as a real bug, and `nucle test` additionally groups the ones that fall
/// within each test's span into that test's pass/fail result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertOp {
    pub condition: Expr,
    pub message: Option<String>,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreOp {
    pub simulate: bool,
    pub file: String,
    pub pool: String,
    pub options: StoreOptions,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreOptions {
    pub redundancy: Option<usize>,
    pub coverage: Option<usize>,
    pub tags: Vec<String>,
    pub expect_recovery_gt: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrieveOp {
    pub pool: String,
    pub query: Vec<QueryPredicate>,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteOp {
    pub file: String,
    pub pool: String,
    pub confirmed: bool,
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryPredicate {
    pub field: String,
    pub op: QueryOp,
    pub value: QueryValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueryOp {
    Contains,
    Eq,
    Gt,
    Lt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueryValue {
    String(String),
    Number(f64),
    Date(String),
    SizeBytes(u64),
    Ident(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineDecl {
    pub name: String,
    pub steps: Vec<PipelineStep>,
    #[serde(default)]
    pub span: Span,
    /// See `PoolDecl::doc`.
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PipelineStep {
    Encode { path: String, codec: Codec },
    Protect { redundancy: usize },
    Store { pool: String },
    VerifyRoundtrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    YinYang,
    Ternary,
    Fountain,
}

impl Codec {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "yinyang" | "yin-yang" => Some(Self::YinYang),
            "ternary" | "ternary-rotating" | "ternary-rotating-cipher" => Some(Self::Ternary),
            "fountain" | "dna-fountain" => Some(Self::Fountain),
            _ => None,
        }
    }
}

impl std::fmt::Display for Codec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::YinYang => "YinYang",
            Self::Ternary => "Ternary",
            Self::Fountain => "Fountain",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Illumina,
    Nanopore,
    Twist,
}

impl Profile {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "illumina" => Some(Self::Illumina),
            "nanopore" | "oxfordnanopore" | "oxford-nanopore" => Some(Self::Nanopore),
            "twist" | "twistbioscience" | "twist-bioscience" => Some(Self::Twist),
            _ => None,
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Illumina => "Illumina",
            Self::Nanopore => "Nanopore",
            Self::Twist => "Twist",
        };
        write!(f, "{}", name)
    }
}
