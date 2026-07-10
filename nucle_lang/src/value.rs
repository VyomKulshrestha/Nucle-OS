//! The smallest runtime value representation NucleScript needs: just
//! enough to make `Result<T, E>` real. This is NOT a general value
//! system for the language -- there is still no `Value` for
//! `Pool<...>`/`Strand`/`Sequence`/`Number` bindings, which remain
//! purely compile-time-inferred (`typeck::ProbPoolType`) exactly as
//! before. Only the specific shapes that can flow through a `Result`
//! need a runtime representation, and only `codegen.rs` (the real
//! execution path) and `sim_backend.rs` (its narrating counterpart)
//! consume this -- every other pipeline stage is unaffected.

/// A value NucleScript code can bind and pass around at runtime. Never
/// constructed for `Pool`/`Strand`/`Sequence`/plain-number bindings --
/// those stay compile-time-only.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// The `Ok(T)` payload of a successful `store` expression --
    /// `nucle_vfs::syscall::WriteResult`'s filename/pool, not the full
    /// struct (nothing downstream needs more than this today).
    DnaFile { filename: String, pool: String },
    /// The `Ok(T)` payload of a successful `delete` expression.
    Deleted { filename: String, strands_removed: usize },
    /// `Void`'s one runtime inhabitant.
    Unit,
    /// A plain `Str` value -- today, only ever an `Err`'s message bound
    /// by a `match` arm's `Err(<name>)` pattern (see `codegen::eval_expr`'s
    /// `Expr::Match` arm). Contrast `EvalOutcome::Err`, which is the same
    /// message flowing as in-flight control-flow, not stored data; this
    /// is what it becomes once a `match` has "landed" it into an ordinary
    /// binding.
    Str(String),
    /// An unforced `Result<T, Str>` sitting in a binding or being passed
    /// around -- the only place Ok/Err-ness lives as *data*. Contrast
    /// `EvalOutcome`, which is about in-flight evaluation control flow,
    /// not storage: a `let` binding with no `?` applied stores a
    /// `Value::Result` inertly; only `Expr::Try` interprets one as
    /// control flow (see `codegen::eval_expr`'s `Expr::Try` arm).
    Result(Result<Box<Value>, String>),
}

/// What evaluating one `Expr` produces: an ordinary value, or a signal to
/// short-circuit the *enclosing function* with this `Err` message. Kept
/// separate from `Value` itself so only the statement-executing loop
/// (`exec_function_body`) needs to check for propagation -- exactly once
/// per statement, matching `?`'s actual control-flow semantics rather
/// than treating it as an ordinary data transform every consumer would
/// need to unwrap.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalOutcome {
    Value(Value),
    /// Propagating an `Err` out of the current function -- the message
    /// that becomes that function's `Result<_, Str>`'s `Err(...)`.
    Err(String),
}

impl EvalOutcome {
    /// Convenience for callers that don't care about the short-circuit
    /// distinction and just want "what value did this produce" (e.g. a
    /// top-level binding, which can't short-circuit anything since `?`
    /// is only valid inside a `Result`-returning function).
    pub fn into_value(self) -> Result<Value, String> {
        match self {
            EvalOutcome::Value(v) => Ok(v),
            EvalOutcome::Err(msg) => Err(msg),
        }
    }
}
