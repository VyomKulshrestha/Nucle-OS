//! Effect classification and confirmation checks for NucleScript.

use crate::ast::{DeleteOp, Effect, Expr, Operation, Program, Declaration};
use serde::{Deserialize, Serialize};

pub fn expr_effect(expr: &Expr) -> Effect {
    match expr {
        Expr::SimulatePool { .. } | Expr::ConsensusVote { .. } => Effect::Pure,
        Expr::SynthesizePool { .. } => Effect::Synthesis,
        Expr::SequencePool { .. } => Effect::Sequencing,
        Expr::FunctionCall { .. } | Expr::Protect { .. } | Expr::Variable(_) | Expr::StringLiteral(_) => Effect::Pure,
    }
}

pub fn operation_effect(operation: &Operation) -> Effect {
    match operation {
        Operation::Store(store) if store.simulate => Effect::Pure,
        Operation::Store(_) => Effect::Synthesis,
        Operation::Retrieve(_) => Effect::Pure,
        Operation::Delete(_) => Effect::Destructive,
    }
}

pub fn expr_has_required_confirmation(expr: &Expr) -> bool {
    match expr {
        Expr::SimulatePool { .. } | Expr::ConsensusVote { .. } => true,
        Expr::SynthesizePool { confirmed, .. } | Expr::SequencePool { confirmed, .. } => *confirmed,
        Expr::FunctionCall { .. } | Expr::Protect { .. } | Expr::Variable(_) | Expr::StringLiteral(_) => true,
    }
}

pub fn delete_has_required_confirmation(delete: &DeleteOp) -> bool {
    delete.confirmed
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeclEffect {
    pub name: String,
    pub kind: String,
    pub effect: Effect,
    pub confirmation_required: bool,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EffectSummary {
    pub declarations: Vec<DeclEffect>,
}

pub fn join_effects(a: Effect, b: Effect) -> Effect {
    match (a, b) {
        (Effect::Destructive, _) | (_, Effect::Destructive) => Effect::Destructive,
        (Effect::Synthesis, _) | (_, Effect::Synthesis) => Effect::Synthesis,
        (Effect::Sequencing, _) | (_, Effect::Sequencing) => Effect::Sequencing,
        _ => Effect::Pure,
    }
}

pub fn decl_effect_info(decl: &Declaration) -> DeclEffect {
    match decl {
        Declaration::Pool(pool) => DeclEffect {
            name: pool.name.clone(),
            kind: "pool".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
        Declaration::Strand(strand) => DeclEffect {
            name: strand.name.clone(),
            kind: "strand".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
        Declaration::Sequence(seq) => DeclEffect {
            name: seq.name.clone(),
            kind: "sequence".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
        Declaration::Let(binding) => {
            let eff = expr_effect(&binding.expr);
            let req = eff == Effect::Synthesis || eff == Effect::Sequencing;
            let conf = expr_has_required_confirmation(&binding.expr);
            DeclEffect {
                name: binding.name.clone(),
                kind: "let".into(),
                effect: eff,
                confirmation_required: req,
                confirmed: conf,
            }
        }
        Declaration::Operation(Operation::Store(store)) => {
            let eff = operation_effect(&Operation::Store(store.clone()));
            let req = eff == Effect::Synthesis && !store.simulate;
            DeclEffect {
                name: store.file.clone(),
                kind: "store".into(),
                effect: eff,
                confirmation_required: req,
                confirmed: true,
            }
        }
        Declaration::Operation(Operation::Retrieve(retrieve)) => DeclEffect {
            name: retrieve.pool.clone(),
            kind: "retrieve".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
        Declaration::Operation(Operation::Delete(delete)) => {
            let eff = Effect::Destructive;
            DeclEffect {
                name: delete.file.clone(),
                kind: "delete".into(),
                effect: eff,
                confirmation_required: true,
                confirmed: delete.confirmed,
            }
        }
        Declaration::Pipeline(pipeline) => DeclEffect {
            name: pipeline.name.clone(),
            kind: "pipeline".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
        Declaration::Function(func) => {
            let mut joint_effect = Effect::Pure;
            let mut req = false;
            let mut conf = true;
            for inner in &func.body {
                let info = decl_effect_info(inner);
                joint_effect = join_effects(joint_effect, info.effect);
                if info.confirmation_required {
                    req = true;
                    if !info.confirmed {
                        conf = false;
                    }
                }
            }
            DeclEffect {
                name: func.name.clone(),
                kind: "function".into(),
                effect: joint_effect,
                confirmation_required: req,
                confirmed: conf,
            }
        }
        Declaration::Import(import) => DeclEffect {
            name: import.source.clone(),
            kind: "import".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
    }
}

pub fn effect_summary(program: &Program) -> EffectSummary {
    let mut declarations = Vec::new();
    for decl in &program.declarations {
        declarations.push(decl_effect_info(decl));
    }
    EffectSummary { declarations }
}

