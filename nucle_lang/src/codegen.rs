//! VFS backend for NucleScript.
//!
//! Two execution paths live here, and it's important to keep straight
//! which is which:
//!   - The pre-existing "replay a flat op list" path (`VfsCall`,
//!     `execute_vfs_call`) drives every `Declaration::Operation`/
//!     `Declaration::Pipeline` in a program exactly as it always has --
//!     unchanged code, just reached through `execute_program`'s
//!     per-declaration dispatch now instead of a standalone loop.
//!   - The new interpreter (`eval_expr`/`exec_function_body`/
//!     `call_user_function`, `value::Value`/`value::EvalOutcome`) is what
//!     makes `Result<T, E>`/`?` (Step 9) real: it runs directly off the
//!     already-desugared `Program`, never through `middle::MirOp` at
//!     all -- MIR still has zero notion of control flow or function
//!     bodies, and stays that way. A user-defined function's body is
//!     executed here for the first time in this compiler's history;
//!     before this, a function call was purely a compile-time signature
//!     lookup (see `typeck::TypeChecker::infer_expr`'s `FunctionCall`
//!     arm), never something that actually ran.

use crate::ast::*;
use crate::effects::{function_table, FunctionTable};
use crate::middle::{lower_program, MirOp};
use crate::typeck::TypeReport;
use crate::value::{EvalOutcome, Value};
use nucle_synth::noise::SimulationConfig;
use nucle_synth::profiles::HardwareProfile;
use nucle_vfs::syscall::{CodecKind, NucleOS, PoolStatus};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CompiledPlan {
    pub program: Program,
    pub calls: Vec<VfsCall>,
    pub type_report: TypeReport,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VfsCall {
    Store {
        file: String,
        pool: String,
        codec: Codec,
        redundancy: usize,
        simulate: bool,
        coverage: usize,
        profile: Profile,
        verify_roundtrip: bool,
    },
    Retrieve {
        pool: String,
        query: String,
    },
    Delete {
        file: String,
        pool: String,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub type_report: TypeReport,
    pub steps: Vec<String>,
    pub pool_status: PoolStatus,
}

impl std::fmt::Display for ExecutionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.type_report.has_warnings() {
            writeln!(f, "NucleScript diagnostics:")?;
            writeln!(f, "{}", self.type_report)?;
        }
        for step in &self.steps {
            writeln!(f, "{}", step)?;
        }
        writeln!(f, "\n{}", self.pool_status)
    }
}

pub fn compile_program(program: Program, type_report: TypeReport) -> CompiledPlan {
    let mir = lower_program(&program);
    let mut calls = Vec::new();
    for op in mir.ops {
        match op {
            MirOp::Store {
                file,
                pool,
                codec,
                redundancy,
                simulate,
                coverage,
                profile,
                verify_roundtrip,
                ..
            } => calls.push(VfsCall::Store {
                file,
                pool,
                codec,
                redundancy,
                simulate,
                coverage,
                profile,
                verify_roundtrip,
            }),
            MirOp::Retrieve { pool, query, .. } => calls.push(VfsCall::Retrieve { pool, query }),
            MirOp::Delete { file, pool, .. } => calls.push(VfsCall::Delete { file, pool }),
            MirOp::PoolSchema { .. } | MirOp::ProbabilisticBind { .. } => {}
        }
    }

    CompiledPlan { program, calls, type_report }
}

/// Runs one already-lowered `VfsCall` against `os`, exactly as the old
/// flat `for call in &plan.calls` loop did before `execute_program` was
/// restructured to walk declarations directly -- copied verbatim (same
/// `.map_err(...)?` calls, same abort-the-whole-run-on-first-failure
/// semantics, same `steps.push(...)` messages), not rewritten, so a
/// program using none of Step 9's new syntax produces byte-identical
/// output (see `nucle_lang/tests/result_backward_compat.rs`).
fn execute_vfs_call(call: &VfsCall, os: &mut NucleOS, base_dir: &Path, steps: &mut Vec<String>) -> Result<(), String> {
    match call {
        VfsCall::Store { file, pool, codec, redundancy, simulate, coverage, profile, verify_roundtrip } => {
            let path = resolve_source_path(base_dir, file);
            let data = std::fs::read(&path)
                .map_err(|err| format!("failed to read '{}': {}", path.display(), err))?;
            let filename = Path::new(file)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(file);

            if *simulate {
                os.simulate_noise = true;
                os.noise_config = SimulationConfig {
                    seed: 42,
                    coverage_depth: *coverage as u32,
                    synthesis_profile: profile_to_hardware(*profile),
                    sequencing_profile: profile_to_hardware(*profile),
                    simulate_decay: false,
                    decay_rate: 0.0,
                    storage_time: 0.0,
                };
            }

            let codec_kind = codec_to_vfs_kind(*codec)?;
            let result = os.dna_write_with_codec(filename, &data, *redundancy, codec_kind)?;
            steps.push(format!("✓ store into {}: {}", pool, result));

            if *verify_roundtrip {
                let recovered = os.dna_read(filename)?;
                if recovered == data {
                    steps.push(format!("✓ verify roundtrip: '{}' recovered exactly", filename));
                } else {
                    return Err(format!("roundtrip verification failed for '{}'", filename));
                }
            }
        }
        VfsCall::Retrieve { pool, query } => {
            let results = os.dna_search(query, 10);
            if results.is_empty() {
                steps.push(format!("- retrieve from {} where {}: no matches", pool, query));
            } else {
                steps.push(format!("✓ retrieve from {} where {}: {} match(es)", pool, query, results.len()));
                for result in results {
                    steps.push(format!("  - {}", result));
                }
            }
        }
        VfsCall::Delete { file, pool } => {
            let filename = Path::new(file)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(file);
            let result = os.dna_delete(filename)?;
            steps.push(format!(
                "delete from {}: removed '{}' ({} strands)",
                pool, result.filename, result.strands_removed
            ));
        }
    }
    Ok(())
}

pub fn execute_program(
    os: &mut NucleOS,
    plan: &mut CompiledPlan,
    base_dir: &Path,
) -> Result<ExecutionReport, String> {
    let mut steps = Vec::new();
    let funcs = function_table(&plan.program);
    let pools: HashMap<String, PoolDecl> = plan
        .program
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Pool(pool) => Some((pool.name.clone(), pool.clone())),
            _ => None,
        })
        .collect();
    let mut env: HashMap<String, Value> = HashMap::new();
    // `plan.calls` is a strict, order-preserving subsequence of
    // `plan.program.declarations` (built by `compile_program` via
    // `middle::lower_program`, which walks the same declarations in the
    // same order and skips everything that isn't a Store/Retrieve/Delete/
    // Pipeline) -- so walking declarations and pulling the next `VfsCall`
    // whenever one of those shapes is encountered reproduces the exact
    // same execution order as the old flat replay loop, without
    // duplicating pool-lookup/pipeline-lowering logic here.
    let mut calls = plan.calls.iter();

    for declaration in &plan.program.declarations {
        match declaration {
            Declaration::Operation(Operation::Store(_))
            | Declaration::Operation(Operation::Retrieve(_))
            | Declaration::Operation(Operation::Delete(_))
            | Declaration::Pipeline(_) => {
                if let Some(call) = calls.next() {
                    execute_vfs_call(call, os, base_dir, &mut steps)?;
                }
            }
            Declaration::Let(binding) if is_result_producing(&binding.expr, &funcs) => {
                // Top-level `?` is impossible here: typeck rejects `?`
                // outside a `Result`-returning function
                // (`E-TRY-OUTSIDE-RESULT-FN`), so a program that passed
                // type-checking can only reach this with a directly
                // Result-producing expression (`StoreExpr`/`DeleteExpr`/
                // a call to a `Result`-returning function) -- never a
                // bare `Expr::Try`, which needs an enclosing function to
                // even parse-check successfully at this position. Nothing
                // here can produce `EvalOutcome::Err`, but `into_value()`
                // degrades harmlessly if that invariant is ever violated.
                let mut calling = HashSet::new();
                let outcome = eval_expr(&binding.expr, &env, &funcs, &pools, os, base_dir, &mut steps, &mut calling);
                if let Ok(value) = outcome.into_value() {
                    env.insert(binding.name.clone(), value);
                }
            }
            _ => {}
        }
    }

    Ok(ExecutionReport { type_report: plan.type_report.clone(), steps, pool_status: os.dna_stat() })
}

/// Whether a `let` binding's expression needs the new interpreter
/// (`eval_expr`) rather than being left alone exactly as before -- true
/// only for the handful of shapes Step 9 actually introduces. Everything
/// else (`Pool<...>`-shaped bindings, `SimulatePool`/`SynthesizePool`/
/// `SequencePool`, calls to non-`Result`-returning functions) is
/// untouched by this function and untouched by `execute_program`'s new
/// per-declaration loop -- exactly as before, since none of those ever
/// produced a `VfsCall`/needed runtime action either.
fn is_result_producing(expr: &Expr, funcs: &FunctionTable) -> bool {
    match expr {
        Expr::StoreExpr(_) | Expr::DeleteExpr(_) | Expr::Try(_) | Expr::Match { .. } => true,
        // A closure literal's own scrutinee is always `Result`-shaped in
        // the sense that it always needs real evaluation: capture is a
        // real `env.clone()` at the exact point of the literal, which
        // only happens by actually running `eval_expr` on it.
        Expr::Closure { .. } => true,
        Expr::FunctionCall { name, .. } => matches!(funcs.get(name).map(|f| &f.return_type), Some(TypeExpr::Result(_, _))),
        _ => false,
    }
}

/// Evaluates one `Expr` to a `Value`/short-circuiting `Err` -- the core
/// of the new interpreter. `Expr::Try` is the only variant that can turn
/// a `Value::Result(Err(_))` into an `EvalOutcome::Err` (the actual
/// short-circuit); every other Result-producing arm always yields
/// `EvalOutcome::Value(Value::Result(_))`, since storing a `Result`
/// without unwrapping it via `?` is legal and inert.
fn eval_expr(
    expr: &Expr,
    env: &HashMap<String, Value>,
    funcs: &FunctionTable,
    pools: &HashMap<String, PoolDecl>,
    os: &mut NucleOS,
    base_dir: &Path,
    steps: &mut Vec<String>,
    calling: &mut HashSet<String>,
) -> EvalOutcome {
    match expr {
        Expr::Try(inner) => match eval_expr(inner, env, funcs, pools, os, base_dir, steps, calling) {
            EvalOutcome::Value(Value::Result(Ok(v))) => EvalOutcome::Value(*v),
            EvalOutcome::Value(Value::Result(Err(msg))) => EvalOutcome::Err(msg),
            // Unreachable for a program that passed type-checking (`?`'s
            // operand is guaranteed Result-shaped by `check_try`) --
            // passed through defensively rather than panicking.
            other => other,
        },
        Expr::StoreExpr(op) => eval_store_expr(op, pools, os, base_dir, steps),
        Expr::DeleteExpr(op) => eval_delete_expr(op, os, steps),
        // Evaluates the scrutinee, then picks and evaluates the matching
        // arm in a *cloned* environment with the pattern bound -- unlike
        // `Try`, this never turns an `Err` into a short-circuit, since
        // both arms are ordinary (not propagating) code paths. `env`
        // itself is never mutated, so the pattern binding can't leak into
        // the surrounding function's own bindings once the arm returns.
        Expr::Match { scrutinee, ok_pattern, ok_body, err_pattern, err_body } => {
            match eval_expr(scrutinee, env, funcs, pools, os, base_dir, steps, calling) {
                EvalOutcome::Value(Value::Result(Ok(v))) => {
                    let mut arm_env = env.clone();
                    arm_env.insert(ok_pattern.clone(), *v);
                    eval_expr(ok_body, &arm_env, funcs, pools, os, base_dir, steps, calling)
                }
                EvalOutcome::Value(Value::Result(Err(msg))) => {
                    let mut arm_env = env.clone();
                    arm_env.insert(err_pattern.clone(), Value::Str(msg));
                    eval_expr(err_body, &arm_env, funcs, pools, os, base_dir, steps, calling)
                }
                // Unreachable for a program that passed type-checking
                // (`check_match` guarantees the scrutinee is Result-shaped)
                // -- passed through defensively rather than panicking.
                other => other,
            }
        }
        // Never Result-shaped (see the type's own doc comment in
        // ast.rs) -- nothing meaningful to produce.
        Expr::RetrieveExpr(_) => EvalOutcome::Value(Value::Unit),
        Expr::Variable(name) => match env.get(name) {
            Some(value) => EvalOutcome::Value(value.clone()),
            // Unreachable post-typecheck (E-VARIABLE-UNDECLARED would
            // have already fired) -- surfaced as a runtime error string
            // rather than panicking, matching how `effects.rs` treats an
            // unresolvable name as inert-but-shouldn't-happen.
            None => EvalOutcome::Err(format!("internal error: undeclared variable '{}' reached execution", name)),
        },
        // Capture *is* this `env.clone()` -- see `Value::Closure`'s doc
        // comment in value.rs. Nothing to filter: a captured `Pool`/
        // `Strand`/`Sequence` binding (if present in `env` at all --
        // inside a function body every `Let` reaches `eval_expr`
        // unconditionally, landing here as an inert `Value::Unit`
        // placeholder, same as it already is for any other expression;
        // see `eval_expr`'s trailing wildcard arm) is just as harmless to
        // capture as it already is to pass around anywhere else.
        Expr::Closure { params, return_type, body, .. } => EvalOutcome::Value(Value::Closure {
            params: params.clone(),
            return_type: return_type.clone(),
            body: body.clone(),
            captured_env: env.clone(),
        }),
        Expr::FunctionCall { name, args } => {
            // Closures resolve first, same priority `typeck::TypeChecker`
            // already validated the program against (a closure-bound
            // name shadows a same-named global function) -- see
            // `infer_expr`'s own `FunctionCall` arm comment for why.
            if let Some(Value::Closure { params, return_type, body, captured_env }) = env.get(name).cloned() {
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    match eval_expr(arg, env, funcs, pools, os, base_dir, steps, calling) {
                        EvalOutcome::Value(v) => arg_values.push(v),
                        err @ EvalOutcome::Err(_) => return err,
                    }
                }
                return call_closure(&params, &return_type, &body, &captured_env, arg_values, funcs, pools, os, base_dir, steps, calling);
            }
            let Some(func) = funcs.get(name).cloned() else {
                return EvalOutcome::Err(format!("internal error: undeclared function '{}' reached execution", name));
            };
            let mut arg_values = Vec::with_capacity(args.len());
            for arg in args {
                match eval_expr(arg, env, funcs, pools, os, base_dir, steps, calling) {
                    EvalOutcome::Value(v) => arg_values.push(v),
                    err @ EvalOutcome::Err(_) => return err,
                }
            }
            call_user_function(&func, arg_values, funcs, pools, os, base_dir, steps, calling)
        }
        // Everything else (`SimulatePool`/`SynthesizePool`/`SequencePool`/
        // `StringLiteral`/`Number`/`BinaryOp`/`Not`) is never Result-
        // shaped, and `is_result_producing` never routes a binding with
        // one of these as its top-level expression into `eval_expr` at
        // all -- this arm only matters for a nested occurrence (e.g. a
        // function-call argument), where a placeholder is enough since
        // nothing downstream inspects it as anything but an opaque value.
        _ => EvalOutcome::Value(Value::Unit),
    }
}

/// `store <file> into <pool> { ... }` in expression position. Mirrors
/// `execute_vfs_call`'s `VfsCall::Store` arm's actual VFS-calling logic
/// (same pool-lookup-derived redundancy/coverage defaults, same
/// simulate-mode wiring, same codec resolution) since both surface forms
/// ultimately need to do the identical thing to `os` -- the only
/// difference is that a real failure here becomes a `Value::Result(Err)`
/// instead of a hard `Result::Err` that aborts the whole run.
fn eval_store_expr(op: &StoreOp, pools: &HashMap<String, PoolDecl>, os: &mut NucleOS, base_dir: &Path, steps: &mut Vec<String>) -> EvalOutcome {
    let Some(pool) = pools.get(&op.pool) else {
        // Unreachable for a program that passed type-checking
        // (`E-STORE-POOL-UNDECLARED` would already have fired) --
        // defensive only.
        let msg = format!("pool '{}' is not declared", op.pool);
        steps.push(format!("✗ store into {}: {}", op.pool, msg));
        return EvalOutcome::Value(Value::Result(Err(msg)));
    };
    let redundancy = op.options.redundancy.unwrap_or(pool.redundancy);
    let coverage = op.options.coverage.unwrap_or(redundancy);

    let path = resolve_source_path(base_dir, &op.file);
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            let msg = format!("failed to read '{}': {}", path.display(), err);
            steps.push(format!("✗ store into {}: {}", op.pool, msg));
            return EvalOutcome::Value(Value::Result(Err(msg)));
        }
    };
    let filename = Path::new(&op.file).file_name().and_then(|name| name.to_str()).unwrap_or(&op.file);

    if op.simulate {
        os.simulate_noise = true;
        os.noise_config = SimulationConfig {
            seed: 42,
            coverage_depth: coverage as u32,
            synthesis_profile: profile_to_hardware(pool.profile),
            sequencing_profile: profile_to_hardware(pool.profile),
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
    }

    let codec_kind = match codec_to_vfs_kind(pool.codec) {
        Ok(kind) => kind,
        Err(msg) => {
            steps.push(format!("✗ store into {}: {}", op.pool, msg));
            return EvalOutcome::Value(Value::Result(Err(msg)));
        }
    };

    match os.dna_write_with_codec(filename, &data, redundancy, codec_kind) {
        Ok(result) => {
            steps.push(format!("✓ store into {}: {}", op.pool, result));
            EvalOutcome::Value(Value::Result(Ok(Box::new(Value::DnaFile { filename: result.filename, pool: op.pool.clone() }))))
        }
        Err(msg) => {
            steps.push(format!("✗ store into {}: {}", op.pool, msg));
            EvalOutcome::Value(Value::Result(Err(msg)))
        }
    }
}

/// `delete <file> from <pool> confirm ...` in expression position --
/// same relationship to `execute_vfs_call`'s `VfsCall::Delete` arm as
/// `eval_store_expr` has to its `VfsCall::Store` arm.
fn eval_delete_expr(op: &DeleteOp, os: &mut NucleOS, steps: &mut Vec<String>) -> EvalOutcome {
    let filename = Path::new(&op.file).file_name().and_then(|name| name.to_str()).unwrap_or(&op.file);
    match os.dna_delete(filename) {
        Ok(result) => {
            steps.push(format!("delete from {}: removed '{}' ({} strands)", op.pool, result.filename, result.strands_removed));
            EvalOutcome::Value(Value::Result(Ok(Box::new(Value::Deleted { filename: result.filename, strands_removed: result.strands_removed }))))
        }
        Err(msg) => {
            steps.push(format!("✗ delete from {}: {}", op.pool, msg));
            EvalOutcome::Value(Value::Result(Err(msg)))
        }
    }
}

/// Runs a NucleScript user function's body for real -- the first place
/// in this compiler's history a function body actually executes
/// statement-by-statement, rather than being purely a compile-time
/// signature lookup. Rust's own call stack serves as NucleScript's; no
/// bytecode VM is introduced. `calling` is a cycle guard mirroring
/// `effects::ResolvingSet`'s pattern exactly -- it must be the SAME set
/// threaded through every nested call, never a fresh one, or the guard
/// can't see a call already in progress.
fn call_user_function(
    func: &FunctionDecl,
    args: Vec<Value>,
    funcs: &FunctionTable,
    pools: &HashMap<String, PoolDecl>,
    os: &mut NucleOS,
    base_dir: &Path,
    steps: &mut Vec<String>,
    calling: &mut HashSet<String>,
) -> EvalOutcome {
    if !calling.insert(func.name.clone()) {
        return EvalOutcome::Err(format!("'{}' recurses without terminating", func.name));
    }
    let mut env: HashMap<String, Value> = HashMap::new();
    for (param, arg) in func.params.iter().zip(args) {
        env.insert(param.name.clone(), arg);
    }
    let outcome = exec_function_body(&func.body, &mut env, funcs, pools, os, base_dir, steps, calling);
    calling.remove(&func.name);

    // A `?` inside this function's own body resolves entirely within
    // this call -- the caller sees an ordinary (wrapped) Result value at
    // the call site, never an automatic propagation of its own. Whether
    // wrapping is needed at all depends on which of the two tail shapes
    // `check_function`'s return-type validation allows actually ran:
    // still-wrapped (`let x: Result<T,E> = store f into p`, no `?`,
    // already a `Value::Result`) needs none; already-unwrapped (`let x:
    // T = <fallible>?`) needs its plain `T` value wrapped into `Ok(T)`
    // here, at the boundary, matching `typeck::TypeChecker::
    // check_function`'s doc comment for exactly this function.
    match outcome {
        EvalOutcome::Err(msg) if matches!(func.return_type, TypeExpr::Result(_, _)) => EvalOutcome::Value(Value::Result(Err(msg))),
        // Not reachable for a program that passed type-checking (`?` is
        // only valid inside a `Result`-returning function) -- passed
        // through as a genuine short-circuit rather than silently
        // swallowed, if it ever is.
        EvalOutcome::Err(msg) => EvalOutcome::Err(msg),
        EvalOutcome::Value(Value::Result(r)) => EvalOutcome::Value(Value::Result(r)),
        EvalOutcome::Value(v) if matches!(func.return_type, TypeExpr::Result(_, _)) => EvalOutcome::Value(Value::Result(Ok(Box::new(v)))),
        EvalOutcome::Value(v) => EvalOutcome::Value(v),
    }
}

/// Calls a closure -- a close mirror of `call_user_function`, except it
/// starts from `captured_env.clone()` (the snapshot taken when the
/// closure literal was evaluated) instead of an empty environment before
/// binding params on top (params intentionally shadow a captured name of
/// the same name). No cycle guard: a closure literal has no name to
/// reference inside its own body, so it can call an *earlier*-defined
/// closure/function but never itself, and two distinct closures can
/// never be mutually recursive either (each only ever sees what was
/// already bound *before* its own literal) -- there is no cycle this
/// could ever need to detect, unlike `call_user_function`'s
/// `calling`/`func.name` guard.
fn call_closure(
    params: &[FnParam],
    return_type: &TypeExpr,
    body: &[Declaration],
    captured_env: &HashMap<String, Value>,
    args: Vec<Value>,
    funcs: &FunctionTable,
    pools: &HashMap<String, PoolDecl>,
    os: &mut NucleOS,
    base_dir: &Path,
    steps: &mut Vec<String>,
    calling: &mut HashSet<String>,
) -> EvalOutcome {
    let mut env = captured_env.clone();
    for (param, arg) in params.iter().zip(args) {
        env.insert(param.name.clone(), arg);
    }
    let outcome = exec_function_body(body, &mut env, funcs, pools, os, base_dir, steps, calling);
    // Same tail-wrapping logic as `call_user_function`'s own boundary --
    // see its doc comment for the full rationale.
    match outcome {
        EvalOutcome::Err(msg) if matches!(return_type, TypeExpr::Result(_, _)) => EvalOutcome::Value(Value::Result(Err(msg))),
        EvalOutcome::Err(msg) => EvalOutcome::Err(msg),
        EvalOutcome::Value(Value::Result(r)) => EvalOutcome::Value(Value::Result(r)),
        EvalOutcome::Value(v) if matches!(return_type, TypeExpr::Result(_, _)) => EvalOutcome::Value(Value::Result(Ok(Box::new(v)))),
        EvalOutcome::Value(v) => EvalOutcome::Value(v),
    }
}

/// Runs `body` sequentially against `env`, executing each `Let`
/// declaration's expression for real and returning early the moment one
/// produces `EvalOutcome::Err` -- this early return *is* `?`'s
/// short-circuit (see `eval_expr`'s `Expr::Try` arm, which is what turns
/// a `Value::Result(Err(_))` into the `EvalOutcome::Err` this loop reacts
/// to). The last `Let`'s value is the function's implicit return,
/// matching `typeck::TypeChecker::check_function`'s "the body's last
/// binding is the return value" convention for both `Pool<...>` and
/// `Result<...>` return types. Non-`Let` declarations don't appear in a
/// function body per the grammar (a body is a sequence of statements,
/// and every current statement form that "produces" something is a
/// `Let`), so there's nothing else to execute here.
fn exec_function_body(
    body: &[Declaration],
    env: &mut HashMap<String, Value>,
    funcs: &FunctionTable,
    pools: &HashMap<String, PoolDecl>,
    os: &mut NucleOS,
    base_dir: &Path,
    steps: &mut Vec<String>,
    calling: &mut HashSet<String>,
) -> EvalOutcome {
    let mut last = EvalOutcome::Value(Value::Unit);
    for decl in body {
        if let Declaration::Let(binding) = decl {
            match eval_expr(&binding.expr, env, funcs, pools, os, base_dir, steps, calling) {
                EvalOutcome::Err(msg) => return EvalOutcome::Err(msg),
                EvalOutcome::Value(value) => {
                    env.insert(binding.name.clone(), value.clone());
                    last = EvalOutcome::Value(value);
                }
            }
        }
    }
    last
}

/// NucleScript's `Fountain` codec has no VFS-backend implementation yet
/// (`nucle check` warns about this at compile time); executing a pipeline
/// that reaches this point with it fails clearly instead of silently
/// falling back to a different codec.
fn codec_to_vfs_kind(codec: Codec) -> Result<CodecKind, String> {
    match codec {
        Codec::Ternary => Ok(CodecKind::Ternary),
        Codec::YinYang => Ok(CodecKind::YinYang),
        Codec::Fountain => Err("codec 'Fountain' has no VFS execution backend yet — use Ternary or YinYang".into()),
    }
}

fn resolve_source_path(base_dir: &Path, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn profile_to_hardware(profile: Profile) -> HardwareProfile {
    match profile {
        Profile::Illumina => HardwareProfile::Illumina,
        Profile::Nanopore => HardwareProfile::OxfordNanopore,
        Profile::Twist => HardwareProfile::TwistBioscience,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_pipeline_to_store_call() {
        let pool = PoolDecl { name: "archive".into(), codec: Codec::Ternary, redundancy: 3, profile: Profile::Illumina, span: Span::default(), doc: None };
        let pipeline = PipelineDecl {
            name: "backup".into(),
            steps: vec![
                PipelineStep::Encode { path: "records.tar".into(), codec: Codec::Ternary },
                PipelineStep::Protect { redundancy: 4 },
                PipelineStep::Store { pool: "archive".into() },
                PipelineStep::VerifyRoundtrip,
            ],
            span: Span::default(),
            doc: None,
        };
        let program = Program {
            declarations: vec![Declaration::Pool(pool), Declaration::Pipeline(pipeline)],
        };
        let plan = compile_program(program, TypeReport::default());
        let call = plan.calls.first().unwrap();
        assert!(matches!(call, VfsCall::Store { redundancy: 4, verify_roundtrip: true, .. }));
    }
}
