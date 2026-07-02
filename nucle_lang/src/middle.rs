//! Bio-aware mid-level IR and optimizer for NucleScript.

use crate::ast::*;
use crate::effects::{expr_effect, function_table, operation_effect};
use crate::probabilistic::{consensus_error_rate_percent, profile_error_rate_percent};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct MirProgram {
    pub ops: Vec<MirOp>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirOp {
    PoolSchema {
        name: String,
        codec: Codec,
        redundancy: usize,
        profile: Profile,
    },
    ProbabilisticBind {
        name: String,
        state: PoolState,
        error_rate_percent: f64,
        effect: Effect,
    },
    Store {
        file: String,
        pool: String,
        codec: Codec,
        redundancy: usize,
        coverage: usize,
        profile: Profile,
        simulate: bool,
        verify_roundtrip: bool,
        effect: Effect,
    },
    Retrieve {
        pool: String,
        query: String,
        effect: Effect,
    },
    Delete {
        file: String,
        pool: String,
        effect: Effect,
    },
}

pub fn lower_program(program: &Program) -> MirProgram {
    let funcs = function_table(program);
    let pools: HashMap<_, _> = program
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Pool(pool) => Some((pool.name.clone(), pool.clone())),
            _ => None,
        })
        .collect();

    let mut bindings: HashMap<String, (PoolState, f64)> = HashMap::new();
    let mut ops = Vec::new();
    let notes = Vec::new();

    for declaration in &program.declarations {
        match declaration {
            Declaration::Pool(pool) => {
                ops.push(MirOp::PoolSchema {
                    name: pool.name.clone(),
                    codec: pool.codec,
                    redundancy: pool.redundancy,
                    profile: pool.profile,
                });
            }
            Declaration::Let(binding) => {
                let Some((state, error_rate_percent)) = infer_binding(&binding.expr, &bindings) else {
                    continue;
                };
                bindings.insert(binding.name.clone(), (state.clone(), error_rate_percent));
                ops.push(MirOp::ProbabilisticBind {
                    name: binding.name.clone(),
                    state,
                    error_rate_percent,
                    effect: expr_effect(&binding.expr, &funcs, &mut std::collections::HashSet::new()),
                });
            }
            Declaration::Operation(Operation::Store(store)) => {
                if let Some(pool) = pools.get(&store.pool) {
                    let redundancy = store.options.redundancy.unwrap_or(pool.redundancy);
                    ops.push(MirOp::Store {
                        file: store.file.clone(),
                        pool: store.pool.clone(),
                        codec: pool.codec,
                        redundancy,
                        coverage: store.options.coverage.unwrap_or(redundancy),
                        profile: pool.profile,
                        simulate: store.simulate,
                        verify_roundtrip: false,
                        effect: operation_effect(&Operation::Store(store.clone())),
                    });
                }
            }
            Declaration::Operation(Operation::Retrieve(retrieve)) => {
                ops.push(MirOp::Retrieve {
                    pool: retrieve.pool.clone(),
                    query: query_to_mir_search(&retrieve.query),
                    effect: operation_effect(&Operation::Retrieve(retrieve.clone())),
                });
            }
            Declaration::Operation(Operation::Delete(delete)) => {
                ops.push(MirOp::Delete {
                    file: delete.file.clone(),
                    pool: delete.pool.clone(),
                    effect: operation_effect(&Operation::Delete(delete.clone())),
                });
            }
            Declaration::Pipeline(pipeline) => {
                if let Some(op) = lower_pipeline(pipeline, &pools) {
                    ops.push(op);
                }
            }
            Declaration::Import(_) | Declaration::Strand(_) | Declaration::Sequence(_) | Declaration::Function(_) => {}
        }
    }

    optimise(MirProgram { ops, notes })
}

pub fn optimise(mut program: MirProgram) -> MirProgram {
    for op in &mut program.ops {
        match op {
            MirOp::Store {
                file,
                redundancy,
                coverage,
                profile,
                ..
            } => {
                let recommended = recommended_redundancy(*profile, *coverage);
                if *redundancy < recommended {
                    program.notes.push(format!(
                        "optimiser raised redundancy for '{}' from {}x to {}x under {}",
                        file, *redundancy, recommended, profile
                    ));
                    *redundancy = recommended;
                }
            }
            MirOp::ProbabilisticBind {
                name,
                state: PoolState::Recovered,
                error_rate_percent,
                ..
            } if *error_rate_percent > 0.01 => {
                program.notes.push(format!(
                    "optimiser notes recovered pool '{}' still carries {:.4}% residual error",
                    name, *error_rate_percent
                ));
            }
            _ => {}
        }
    }
    program
}

fn infer_binding(
    expr: &Expr,
    bindings: &HashMap<String, (PoolState, f64)>,
) -> Option<(PoolState, f64)> {
    match expr {
        Expr::SimulatePool { profile, .. }
        | Expr::SynthesizePool { profile, .. }
        | Expr::SequencePool { profile, .. } => {
            Some((PoolState::Profile(*profile), profile_error_rate_percent(*profile)))
        }
        Expr::ConsensusVote { source, coverage } => {
            let (_, error_rate_percent) = bindings.get(source)?;
            Some((
                PoolState::Recovered,
                consensus_error_rate_percent(*error_rate_percent, *coverage),
            ))
        }
        Expr::Variable(name) => bindings.get(name).cloned(),
        Expr::FunctionCall { .. } | Expr::Protect { .. } | Expr::StringLiteral(_) => None,
    }
}

fn lower_pipeline(pipeline: &PipelineDecl, pools: &HashMap<String, PoolDecl>) -> Option<MirOp> {
    let mut file = None;
    let mut codec = None;
    let mut redundancy = None;
    let mut target_pool = None;
    let mut verify_roundtrip = false;

    for step in &pipeline.steps {
        match step {
            PipelineStep::Encode { path, codec: step_codec } => {
                file = Some(path.clone());
                codec = Some(*step_codec);
            }
            PipelineStep::Protect { redundancy: value } => redundancy = Some(*value),
            PipelineStep::Store { pool } => target_pool = Some(pool.clone()),
            PipelineStep::VerifyRoundtrip => verify_roundtrip = true,
        }
    }

    let file = file?;
    let target_pool = target_pool?;
    let pool = pools.get(&target_pool)?;
    let redundancy = redundancy.unwrap_or(pool.redundancy);
    Some(MirOp::Store {
        file,
        pool: target_pool,
        codec: codec.unwrap_or(pool.codec),
        redundancy,
        coverage: redundancy,
        profile: pool.profile,
        simulate: false,
        verify_roundtrip,
        effect: Effect::Synthesis,
    })
}

fn recommended_redundancy(profile: Profile, coverage: usize) -> usize {
    match profile {
        Profile::Twist => 2,
        Profile::Illumina if coverage >= 10 => 3,
        Profile::Illumina => 4,
        Profile::Nanopore if coverage >= 10 => 6,
        Profile::Nanopore => 8,
    }
}

fn query_to_mir_search(query: &[QueryPredicate]) -> String {
    if query.is_empty() {
        return String::new();
    }
    query
        .iter()
        .map(|predicate| match (&predicate.op, &predicate.value) {
            (QueryOp::Contains, QueryValue::String(value)) => format!("{}:{}", predicate.field, value),
            (QueryOp::Eq, value) => format!("{}:{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Gt, value) => format!("{}>{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Lt, value) => format!("{}<{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Contains, value) => format!("{}:{}", predicate.field, query_value_to_string(value)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn query_value_to_string(value: &QueryValue) -> String {
    match value {
        QueryValue::String(value) | QueryValue::Date(value) | QueryValue::Ident(value) => value.clone(),
        QueryValue::Number(value) => value.to_string(),
        QueryValue::SizeBytes(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimiser_raises_nanopore_redundancy() {
        let program = MirProgram {
            ops: vec![MirOp::Store {
                file: "data.bin".into(),
                pool: "archive".into(),
                codec: Codec::Ternary,
                redundancy: 1,
                coverage: 1,
                profile: Profile::Nanopore,
                simulate: true,
                verify_roundtrip: false,
                effect: Effect::Pure,
            }],
            notes: Vec::new(),
        };
        let optimised = optimise(program);
        assert!(matches!(
            optimised.ops[0],
            MirOp::Store { redundancy: 8, .. }
        ));
        assert_eq!(optimised.notes.len(), 1);
    }
}
