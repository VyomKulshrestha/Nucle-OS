//! Hardware bridge boundary for effectful NucleScript plans.

use crate::ast::{Effect, Program};
use crate::middle::{lower_program, MirOp};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareRequest {
    pub effect: Effect,
    pub target: String,
    pub profile: Option<String>,
    pub confirmation: String,
}

pub trait HardwareBridge {
    fn submit(&self, request: &HardwareRequest) -> Result<String, String>;
}

pub fn collect_hardware_requests(program: &Program) -> Vec<HardwareRequest> {
    lower_program(program)
        .ops
        .into_iter()
        .filter_map(|op| match op {
            MirOp::ProbabilisticBind {
                name,
                state,
                effect: effect @ Effect::Synthesis,
                ..
            }
            | MirOp::ProbabilisticBind {
                name,
                state,
                effect: effect @ Effect::Sequencing,
                ..
            } => Some(HardwareRequest {
                effect,
                target: name,
                profile: Some(state.to_string()),
                confirmation: "hardware".into(),
            }),
            MirOp::Store {
                file,
                profile,
                effect: Effect::Synthesis,
                ..
            } => Some(HardwareRequest {
                effect: Effect::Synthesis,
                target: file,
                profile: Some(profile.to_string()),
                confirmation: "hardware".into(),
            }),
            MirOp::Delete {
                file,
                effect: Effect::Destructive,
                ..
            } => Some(HardwareRequest {
                effect: Effect::Destructive,
                target: file,
                profile: None,
                confirmation: "physical_key".into(),
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
