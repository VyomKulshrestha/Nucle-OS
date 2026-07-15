//! Simulation backend for NucleScript programs.

use crate::ast::*;
use crate::effects::{function_table, FunctionTable};
use crate::middle::{lower_program, MirOp};
use crate::typeck::TypeReport;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct SimulationPlan {
    pub steps: Vec<SimulationStep>,
    pub optimiser_notes: Vec<String>,
    pub type_report: TypeReport,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SimulationStep {
    Pool {
        name: String,
        profile: String,
        redundancy: usize,
    },
    ProbabilisticBinding {
        name: String,
        state: String,
        error_rate_percent: f64,
        effect: String,
    },
    Store {
        file: String,
        pool: String,
        redundancy: usize,
        coverage: usize,
        profile: String,
        hardware_free: bool,
    },
    Retrieve {
        pool: String,
        query: String,
    },
    Delete {
        file: String,
        pool: String,
        hardware_free: bool,
    },
}

pub fn compile_simulation(program: Program, type_report: TypeReport) -> SimulationPlan {
    let mir = lower_program(&program);
    let mut steps = Vec::new();

    for op in mir.ops {
        match op {
            MirOp::PoolSchema {
                name,
                redundancy,
                profile,
                ..
            } => steps.push(SimulationStep::Pool {
                name,
                profile: profile.to_string(),
                redundancy,
            }),
            MirOp::ProbabilisticBind {
                name,
                state,
                error_rate_percent,
                effect,
            } => steps.push(SimulationStep::ProbabilisticBinding {
                name,
                state: state.to_string(),
                error_rate_percent,
                effect: effect.to_string(),
            }),
            MirOp::Store {
                file,
                pool,
                redundancy,
                coverage,
                profile,
                simulate,
                ..
            } => steps.push(SimulationStep::Store {
                file,
                pool,
                redundancy,
                coverage,
                profile: profile.to_string(),
                hardware_free: simulate,
            }),
            MirOp::Retrieve { pool, query, .. } => {
                steps.push(SimulationStep::Retrieve { pool, query });
            }
            MirOp::Delete { file, pool, .. } => steps.push(SimulationStep::Delete {
                file,
                pool,
                hardware_free: true,
            }),
        }
    }

    // `Result<T,E>`/`?` additions never reach `MirOp` at all -- they run
    // through a real interpreter in `codegen.rs`/never touch MIR, since
    // MIR has no notion of function bodies or control flow (see that
    // module's doc comment). This backend narrates what a real
    // run WOULD do without ever touching hardware or a real VFS, so a
    // `store`/`delete` reached only through the new expression-position
    // syntax (directly, or through a call to a `Result`-returning
    // function) needs its own narration pass here too, or `nucle plan`/
    // `nucle explain` would silently show nothing for it while `nucle
    // run` does real work -- reuses the *existing* `SimulationStep::
    // Store`/`Delete` variants directly, since `StoreExpr`/`DeleteExpr`
    // wrap the identical `StoreOp`/`DeleteOp` payload the statement form
    // already narrates.
    let funcs = function_table(&program);
    let pools: HashMap<String, PoolDecl> = program
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Pool(pool) => Some((pool.name.clone(), pool.clone())),
            _ => None,
        })
        .collect();
    // Top-level closures, built incrementally as declarations are
    // walked in order -- see `narrate_result_expr`'s own doc comment.
    let mut closures: HashMap<String, Vec<Declaration>> = HashMap::new();
    for declaration in &program.declarations {
        if let Declaration::Let(binding) = declaration {
            if let Expr::Closure { body, .. } = &binding.expr {
                closures.insert(binding.name.clone(), body.clone());
            }
            narrate_result_expr(&binding.expr, &pools, &funcs, &mut closures, &mut steps, &mut HashSet::new());
        }
    }

    SimulationPlan {
        steps,
        optimiser_notes: mir.notes,
        type_report,
    }
}

/// Describes -- never executes -- what a `Result`-producing expression
/// would do: `StoreExpr`/`DeleteExpr` narrate directly (same shape as
/// the statement form); `Expr::Try` narrates its inner expression
/// unchanged (unwrapping doesn't change what would run); a call to a
/// `Result`-returning function narrates every store/delete in *that*
/// function's own body, one level at a time via ordinary recursion.
/// `calling` is a cycle guard mirroring `effects::ResolvingSet`'s pattern
/// -- a function that (mutually) recurses into itself is described once
/// per distinct name, not unrolled forever.
/// `closures` is a `let`-bound closure's name mapped to its own body,
/// built up *incrementally* by both call sites of this function (the
/// top-level loop in `compile_simulation`, and the per-called-function-
/// body loop in the `FunctionCall` arm below) as they walk declarations
/// in order -- so a call to a closure defined *earlier in the same
/// scope chain* narrates into its real body, mirroring the priority
/// `effects.rs`'s own `closures` fix already established (checked
/// before `funcs`). A closure received as a `Fn(...)`-typed *parameter*
/// is still unnarratable: its real body isn't known at this call site
/// either, only at runtime. Effect-annotated `Fn(...)` types
/// (`confirm hardware`/`confirm physical_key`) close the *effect/
/// confirmation-analysis* version of this gap (see `effects.rs`'s
/// `fn_param_effects`) -- a declared ceiling is enough to know a call
/// site's effect and whether it's properly confirmed -- but a ceiling
/// isn't a real body, so this narrator still can't synthesize a
/// concrete VFS step for it. Only whole-program flow analysis (the
/// option not taken) could close this specific, narrower remaining gap.
fn narrate_result_expr(
    expr: &Expr,
    pools: &HashMap<String, PoolDecl>,
    funcs: &FunctionTable,
    closures: &mut HashMap<String, Vec<Declaration>>,
    steps: &mut Vec<SimulationStep>,
    calling: &mut HashSet<String>,
) {
    match expr {
        Expr::Try(inner) => narrate_result_expr(inner, pools, funcs, closures, steps, calling),
        // Constructing a Result is inert on its own -- only what's
        // already inside could produce a step to narrate.
        Expr::Ok(inner) => narrate_result_expr(inner, pools, funcs, closures, steps, calling),
        Expr::Err(inner) => narrate_result_expr(inner, pools, funcs, closures, steps, calling),
        // Constructing a user enum instance is inert the same
        // way -- explicit, not left to the trailing wildcard below, since
        // a missed arm here would silently swallow a nested operation
        // (e.g. `MyEnum::Fallback(store "x" into pool)`) with no compile
        // error to catch it.
        Expr::EnumConstruct { payload: Some(inner), .. } => narrate_result_expr(inner, pools, funcs, closures, steps, calling),
        Expr::EnumConstruct { payload: None, .. } => {}
        Expr::StoreExpr(op) => {
            if let Some(pool) = pools.get(&op.pool) {
                let redundancy = op.options.redundancy.unwrap_or(pool.redundancy);
                let coverage = op.options.coverage.unwrap_or(redundancy);
                steps.push(SimulationStep::Store {
                    file: op.file.clone(),
                    pool: op.pool.clone(),
                    redundancy,
                    coverage,
                    profile: pool.profile.to_string(),
                    hardware_free: op.simulate,
                });
            }
        }
        Expr::DeleteExpr(op) => {
            steps.push(SimulationStep::Delete { file: op.file.clone(), pool: op.pool.clone(), hardware_free: true });
        }
        // The narrator can't know at plan-time which arm would actually
        // run, so -- like effects/confirmation above it -- it narrates
        // the scrutinee and every arm unconditionally (generalized from a
        // fixed two-arm walk to N arms): describing everything that could
        // possibly run, not guessing which will.
        Expr::Match { scrutinee, arms } => {
            narrate_result_expr(scrutinee, pools, funcs, closures, steps, calling);
            for arm in arms {
                narrate_result_expr(&arm.body, pools, funcs, closures, steps, calling);
            }
        }
        // Closures resolve first, mirroring `effects.rs`'s own priority
        // -- see this function's own doc comment.
        Expr::FunctionCall { name, .. } if closures.contains_key(name) => {
            if calling.insert(name.clone()) {
                let body = closures[name].clone();
                for decl in &body {
                    if let Declaration::Let(binding) = decl {
                        if let Expr::Closure { body: closure_body, .. } = &binding.expr {
                            closures.insert(binding.name.clone(), closure_body.clone());
                        }
                        narrate_result_expr(&binding.expr, pools, funcs, closures, steps, calling);
                    }
                }
                calling.remove(name);
            }
        }
        Expr::FunctionCall { name, .. } => {
            if let Some(func) = funcs.get(name) {
                if calling.insert(name.clone()) {
                    for decl in &func.body {
                        if let Declaration::Let(binding) = decl {
                            if let Expr::Closure { body: closure_body, .. } = &binding.expr {
                                closures.insert(binding.name.clone(), closure_body.clone());
                            }
                            narrate_result_expr(&binding.expr, pools, funcs, closures, steps, calling);
                        }
                    }
                    calling.remove(name);
                }
            }
        }
        // Never Result-shaped (retrieve has no real failure mode) or not
        // reachable in this position for a program that passed type-
        // checking -- nothing to narrate.
        _ => {}
    }
}

impl std::fmt::Display for SimulationPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.type_report.has_warnings() {
            writeln!(f, "NucleScript diagnostics:")?;
            writeln!(f, "{}", self.type_report)?;
        }
        if !self.optimiser_notes.is_empty() {
            writeln!(f, "Optimiser notes:")?;
            for note in &self.optimiser_notes {
                writeln!(f, "- {}", note)?;
            }
        }
        writeln!(f, "Simulation plan:")?;
        for step in &self.steps {
            writeln!(f, "- {}", step)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for SimulationStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pool {
                name,
                profile,
                redundancy,
            } => write!(f, "pool {} profile={} redundancy={}x", name, profile, redundancy),
            Self::ProbabilisticBinding {
                name,
                state,
                error_rate_percent,
                effect,
            } => write!(
                f,
                "bind {} as Pool<{}> error={:.4}% effect={}",
                name, state, error_rate_percent, effect
            ),
            Self::Store {
                file,
                pool,
                redundancy,
                coverage,
                profile,
                hardware_free,
            } => write!(
                f,
                "store {} into {} redundancy={}x coverage={}x profile={} hardware_free={}",
                file, pool, redundancy, coverage, profile, hardware_free
            ),
            Self::Retrieve { pool, query } => write!(f, "retrieve from {} where {}", pool, query),
            Self::Delete {
                file,
                pool,
                hardware_free,
            } => write!(f, "delete {} from {} hardware_free={}", file, pool, hardware_free),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn simulation_backend_emits_probabilistic_steps() {
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
            ],
        };
        let plan = compile_simulation(program, TypeReport::default());
        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(
            plan.steps[1],
            SimulationStep::ProbabilisticBinding { .. }
        ));
    }
}
