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
    FunctionCall { name: String, args: Vec<Expr> },
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
