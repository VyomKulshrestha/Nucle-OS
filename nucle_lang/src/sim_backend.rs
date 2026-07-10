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

    // Step 9 (`Result<T,E>`/`?`) additions never reach `MirOp` at all --
    // they run through a real interpreter in `codegen.rs`/never touch
    // MIR, since MIR has no notion of function bodies or control flow
    // (see that module's doc comment). This backend narrates what a real
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
    for declaration in &program.declarations {
        if let Declaration::Let(binding) = declaration {
            narrate_result_expr(&binding.expr, &pools, &funcs, &mut steps, &mut HashSet::new());
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
fn narrate_result_expr(
    expr: &Expr,
    pools: &HashMap<String, PoolDecl>,
    funcs: &FunctionTable,
    steps: &mut Vec<SimulationStep>,
    calling: &mut HashSet<String>,
) {
    match expr {
        Expr::Try(inner) => narrate_result_expr(inner, pools, funcs, steps, calling),
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
        // all three (scrutinee and both arms) unconditionally: describing
        // everything that could possibly run, not guessing which will.
        Expr::Match { scrutinee, ok_body, err_body, .. } => {
            narrate_result_expr(scrutinee, pools, funcs, steps, calling);
            narrate_result_expr(ok_body, pools, funcs, steps, calling);
            narrate_result_expr(err_body, pools, funcs, steps, calling);
        }
        Expr::FunctionCall { name, .. } => {
            if let Some(func) = funcs.get(name) {
                if calling.insert(name.clone()) {
                    for decl in &func.body {
                        if let Declaration::Let(binding) = decl {
                            narrate_result_expr(&binding.expr, pools, funcs, steps, calling);
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
