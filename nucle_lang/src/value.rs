//! The smallest runtime value representation NucleScript needs: just
//! enough to make `Result<T, E>` real. This is NOT a general value
//! system for the language -- there is still no `Value` for
//! `Pool<...>`/`Strand`/`Sequence`/`Number` bindings, which remain
//! purely compile-time-inferred (`typeck::ProbPoolType`) exactly as
//! before. Only the specific shapes that can flow through a `Result`
//! need a runtime representation, and only `codegen.rs` (the real
//! execution path) and `sim_backend.rs` (its narrating counterpart)
//! consume this -- every other pipeline stage is unaffected.

use crate::ast::{Declaration, FnEffectAnnotation, FnParam, TypeExpr};
use std::collections::HashMap;

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
    /// A declared pool passed as an argument to a `Pool<...>` parameter;
    /// `store`/`delete`/`retrieve` against the parameter's name resolve to
    /// this pool.
    PoolRef(String),
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
    /// A closure literal's runtime value -- `params`/`return_type`/`body`
    /// are the literal's own AST verbatim; `captured_env` is a snapshot
    /// of the environment at the exact point the literal was evaluated
    /// (see `codegen::eval_expr`'s `Expr::Closure` arm). This snapshot
    /// *is* capture -- see `Expr::Closure`'s doc comment in ast.rs for
    /// why capture-by-snapshot is simply correct for this language
    /// (single-assignment `let` bindings mean there is no "later
    /// mutation" a by-value/by-reference distinction could ever observe).
    Closure {
        params: Vec<FnParam>,
        return_type: TypeExpr,
        body: Vec<Declaration>,
        captured_env: HashMap<String, Value>,
        /// The effect ceilings (`Fn(...) -> T confirm hardware`) of every
        /// slot this value was bound into on its way here. The checker
        /// promised the body stays within each; the interpreter holds it
        /// to them while it runs (see `codegen::observe`).
        ceilings: Vec<FnEffectAnnotation>,
    },
    /// A user-declared `enum`'s runtime instance. Deliberately
    /// NOT how `Result` itself is represented (see `Value::Result` above)
    /// -- keeping them separate means the large amount of existing
    /// runtime code that already matches `Value::Result(Ok(_)/Err(_))`
    /// directly needs zero changes; only the general `Expr::Match`
    /// evaluator (`codegen::eval_expr`) needs to look at both shapes
    /// uniformly, extracting `(variant name, payload)` from either.
    EnumInstance {
        enum_name: String,
        variant: String,
        payload: Option<Box<Value>>,
    },
}

impl Value {
    /// Whether this value is one the type checker allows for `ty`. The
    /// interpreter checks every argument, annotated binding and return
    /// value against it while a program runs (see `docs/soundness.md`),
    /// so a value of the wrong shape shows up as a fault instead of
    /// flowing on. `Strand`, `Sequence` and `Recovery` have no runtime
    /// shape and accept anything that is not a `Result`, closure or enum
    /// instance.
    pub fn conforms_to(&self, ty: &TypeExpr) -> bool {
        match (ty, self) {
            (TypeExpr::Result(ok, _), Value::Result(Ok(inner))) => inner.conforms_to(ok),
            (TypeExpr::Result(_, err), Value::Result(Err(_))) => **err == TypeExpr::Str,
            (TypeExpr::Fn(params, _, _), Value::Closure { params: actual, .. }) => params.len() == actual.len(),
            (TypeExpr::Enum(name), Value::EnumInstance { enum_name, .. }) => name == enum_name,
            (TypeExpr::Result(..) | TypeExpr::Fn(..) | TypeExpr::Enum(_), _) => false,
            (_, Value::Result(_) | Value::Closure { .. } | Value::EnumInstance { .. }) => false,
            (TypeExpr::Void, value) => matches!(value, Value::Unit | Value::Deleted { .. }),
            (TypeExpr::Str, value) => matches!(value, Value::Str(_)),
            // A file is named by a string until it is stored; `protect`
            // hands its argument back unchanged, so a `DnaFile` can still
            // be one.
            (TypeExpr::File, value) => matches!(value, Value::Str(_) | Value::DnaFile { .. }),
            (TypeExpr::DnaFile, value) => matches!(value, Value::Str(_) | Value::DnaFile { .. }),
            // A declared pool passed by name, or a simulated pool, which
            // exists only at compile time.
            (TypeExpr::Pool(_), value) => matches!(value, Value::PoolRef(_) | Value::Unit),
            (TypeExpr::Strand | TypeExpr::Sequence | TypeExpr::Recovery, _) => true,
        }
    }
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
