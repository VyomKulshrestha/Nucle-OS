//! Simulation backend for NucleScript programs.

use crate::ast::Program;
use crate::middle::{lower_program, MirOp};
use crate::typeck::TypeReport;

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

    SimulationPlan {
        steps,
        optimiser_notes: mir.notes,
        type_report,
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
