//! Hardware bridge boundary for effectful NucleScript plans.

use crate::ast::{Effect, Program};
use crate::middle::{lower_program, MirOp};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestType {
    Synthesis {
        file_name: String,
        profile: String,
    },
    Sequencing {
        file_name: String,
        profile: String,
    },
    Destructive {
        file_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareRequest {
    pub effect: Effect,
    pub target: String,
    pub profile: Option<String>,
    pub confirmation: String,
    pub detail: RequestType,
}

// The execution-side trait for submitting these requests to a provider
// lives in `nucle_hardware::Provider`, not here — this module only ever
// defines and collects request *types*. An earlier `HardwareBridge` trait
// duplicated that concern with zero implementations; it was removed rather
// than kept alongside `Provider` as a second, unrelated execution trait.

pub fn collect_hardware_requests(program: &Program) -> Vec<HardwareRequest> {
    lower_program(program)
        .ops
        .into_iter()
        .filter_map(|op| match op {
            MirOp::ProbabilisticBind {
                name,
                state,
                effect,
                ..
            } if effect == Effect::Synthesis || effect == Effect::Sequencing => {
                let detail = if effect == Effect::Synthesis {
                    RequestType::Synthesis {
                        file_name: name.clone(),
                        profile: state.to_string(),
                    }
                } else {
                    RequestType::Sequencing {
                        file_name: name.clone(),
                        profile: state.to_string(),
                    }
                };
                Some(HardwareRequest {
                    effect,
                    target: name,
                    profile: Some(state.to_string()),
                    confirmation: "hardware".into(),
                    detail,
                })
            }
            MirOp::Store {
                file,
                profile,
                effect: Effect::Synthesis,
                ..
            } => Some(HardwareRequest {
                effect: Effect::Synthesis,
                target: file.clone(),
                profile: Some(profile.to_string()),
                confirmation: "hardware".into(),
                detail: RequestType::Synthesis {
                    file_name: file,
                    profile: profile.to_string(),
                },
            }),
            MirOp::Delete {
                file,
                effect: Effect::Destructive,
                ..
            } => Some(HardwareRequest {
                effect: Effect::Destructive,
                target: file.clone(),
                profile: None,
                confirmation: "physical_key".into(),
                detail: RequestType::Destructive {
                    file_name: file,
                },
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn collects_effectful_requests() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Twist,
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
                }),
            ],
        };
        let requests = collect_hardware_requests(&program);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].effect, Effect::Synthesis);
    }
}
