//! Hardware bridge boundary for effectful NucleScript plans.

use crate::ast::{Effect, PoolState, Program};
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
    /// A read-only quality-control check, derived from a `pipeline { ...,
    /// verify roundtrip }` stage -- not cost-bearing or destructive, so it
    /// never needs `--confirm` (see `collect_hardware_requests`).
    Qc {
        file_name: String,
        checks: Vec<String>,
    },
    /// A read-only recovery signal, derived from a `consensus_vote(...)`
    /// call -- every call produces a `PoolState::Recovered` binding, which
    /// is exactly the "recovery happened" event worth surfacing to a
    /// hardware bridge, without inventing new NucleScript syntax for it.
    /// `binding_name` is the `let`-binding's own name; there's no runtime
    /// archive ID available at this compile-time-only collection pass.
    Recovery {
        binding_name: String,
        consensus_method: String,
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
        .flat_map(|op| -> Vec<HardwareRequest> {
            match op {
                MirOp::ProbabilisticBind {
                    name,
                    state,
                    effect,
                    ..
                } => {
                    let mut requests = Vec::new();
                    if effect == Effect::Synthesis || effect == Effect::Sequencing {
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
                        requests.push(HardwareRequest {
                            effect,
                            target: name.clone(),
                            profile: Some(state.to_string()),
                            confirmation: "hardware".into(),
                            detail,
                        });
                    }
                    // Every `consensus_vote(...)` call produces a
                    // `PoolState::Recovered` binding -- a read-only
                    // recovery signal, not a cost-bearing or destructive
                    // one, so it's `Effect::Pure` and needs no `--confirm`.
                    if state == PoolState::Recovered {
                        requests.push(HardwareRequest {
                            effect: Effect::Pure,
                            target: name.clone(),
                            profile: None,
                            confirmation: String::new(),
                            detail: RequestType::Recovery {
                                binding_name: name,
                                consensus_method: "majority-vote".into(),
                            },
                        });
                    }
                    requests
                }
                MirOp::Store {
                    file,
                    profile,
                    effect: Effect::Synthesis,
                    verify_roundtrip,
                    ..
                } => {
                    let mut requests = vec![HardwareRequest {
                        effect: Effect::Synthesis,
                        target: file.clone(),
                        profile: Some(profile.to_string()),
                        confirmation: "hardware".into(),
                        detail: RequestType::Synthesis {
                            file_name: file.clone(),
                            profile: profile.to_string(),
                        },
                    }];
                    // A `pipeline { ..., verify roundtrip }` stage is a
                    // read-only QC check on top of the synthesis request
                    // above -- again `Effect::Pure`, no `--confirm` needed.
                    if verify_roundtrip {
                        requests.push(HardwareRequest {
                            effect: Effect::Pure,
                            target: file.clone(),
                            profile: None,
                            confirmation: String::new(),
                            detail: RequestType::Qc {
                                file_name: file,
                                checks: vec!["roundtrip".to_string()],
                            },
                        });
                    }
                    requests
                }
                MirOp::Delete {
                    file,
                    effect: Effect::Destructive,
                    ..
                } => vec![HardwareRequest {
                    effect: Effect::Destructive,
                    target: file.clone(),
                    profile: None,
                    confirmation: "physical_key".into(),
                    detail: RequestType::Destructive {
                        file_name: file,
                    },
                }],
                _ => Vec::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    /// Compiles a real `.nsl` source string, panicking with a readable
    /// message on any lex/parse failure -- these tests care about MIR
    /// derivation, not hand-building `Program` AST nodes for cases the
    /// parser already covers.
    fn compile(source: &str) -> Program {
        let tokens = crate::Lexer::new(source).tokenize().expect("lex error");
        crate::Parser::new(tokens).parse_program().expect("parse error")
    }

    #[test]
    fn consensus_vote_produces_a_pure_recovery_request() {
        let program = compile(
            r#"
            pool archive: DnaPool {
                codec: Ternary,
                redundancy: 3x,
                profile: Illumina
            }

            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
            "#,
        );
        let requests = collect_hardware_requests(&program);
        let recovery = requests
            .iter()
            .find(|r| matches!(r.detail, RequestType::Recovery { .. }))
            .expect("expected a Recovery request");
        assert_eq!(recovery.effect, Effect::Pure);
        assert_eq!(recovery.confirmation, "");
        match &recovery.detail {
            RequestType::Recovery { binding_name, consensus_method } => {
                assert_eq!(binding_name, "recovered");
                assert_eq!(consensus_method, "majority-vote");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn pipeline_verify_roundtrip_produces_a_pure_qc_request_alongside_synthesis() {
        let program = compile(
            r#"
            pool archive: DnaPool {
                codec: Ternary,
                redundancy: 3x,
                profile: Illumina
            }

            pipeline archive_it {
                encode "data.bin" using Ternary,
                protect with redundancy 3x,
                store into archive,
                verify roundtrip
            }
            "#,
        );
        let requests = collect_hardware_requests(&program);
        assert!(
            requests.iter().any(|r| matches!(r.detail, RequestType::Synthesis { .. })),
            "expected the pipeline's own Synthesis request to still be collected"
        );
        let qc = requests
            .iter()
            .find(|r| matches!(r.detail, RequestType::Qc { .. }))
            .expect("expected a Qc request");
        assert_eq!(qc.effect, Effect::Pure);
        assert_eq!(qc.confirmation, "");
        match &qc.detail {
            RequestType::Qc { file_name, checks } => {
                assert_eq!(file_name, "data.bin");
                assert_eq!(checks, &["roundtrip".to_string()]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn pipeline_without_verify_roundtrip_produces_no_qc_request() {
        let program = compile(
            r#"
            pool archive: DnaPool {
                codec: Ternary,
                redundancy: 3x,
                profile: Illumina
            }

            pipeline archive_it {
                encode "data.bin" using Ternary,
                protect with redundancy 3x,
                store into archive
            }
            "#,
        );
        let requests = collect_hardware_requests(&program);
        assert!(!requests.iter().any(|r| matches!(r.detail, RequestType::Qc { .. })));
    }

    #[test]
    fn qc_and_recovery_requests_do_not_require_confirmation() {
        // Regression guard for a deliberate design decision: Qc/Recovery
        // are read-only, so nucle_hardware::confirm's
        // is_effectful/count_effectful must never treat them as
        // cost-bearing/destructive. Exercised for real here rather than
        // only in nucle_hardware's own tests, since the Effect::Pure choice
        // is made in this module.
        let program = compile(
            r#"
            pool archive: DnaPool {
                codec: Ternary,
                redundancy: 3x,
                profile: Illumina
            }

            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
            "#,
        );
        let requests = collect_hardware_requests(&program);
        assert!(requests.iter().all(|r| r.effect == Effect::Pure));
    }

    #[test]
    fn collects_effectful_requests() {
        let program = Program {
            declarations: vec![
                Declaration::Pool(PoolDecl {
                    name: "archive".into(),
                    codec: Codec::Ternary,
                    redundancy: 3,
                    profile: Profile::Twist,
                    span: Span::default(),
                    doc: None,
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
                    span: Span::default(),
                }),
            ],
        };
        let requests = collect_hardware_requests(&program);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].effect, Effect::Synthesis);
    }
}
