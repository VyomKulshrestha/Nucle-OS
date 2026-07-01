//! Effect classification and confirmation checks for NucleScript.

use crate::ast::{DeleteOp, Effect, Expr, Operation};

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

