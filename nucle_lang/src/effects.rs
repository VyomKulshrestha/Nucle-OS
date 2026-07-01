//! Effect classification and confirmation checks for NucleScript.

use crate::ast::{DeleteOp, Declaration, Effect, Expr, FunctionDecl, Operation, Program};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Maps a function's declared name to its declaration, built once per
/// program and threaded through effect classification so a call site can
/// see what the callee's body actually does instead of assuming `Pure`.
pub type FunctionTable = HashMap<String, FunctionDecl>;

/// Tracks functions currently being resolved, so a (mutually-)recursive
/// call can be detected instead of recursing forever. Must be the SAME set
/// threaded through every nested call in one top-level resolution — never
/// a fresh one per call, or the cycle guard can't see the call in progress.
pub type ResolvingSet = HashSet<String>;

pub fn function_table(program: &Program) -> FunctionTable {
    program
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Function(func) => Some((func.name.clone(), func.clone())),
            _ => None,
        })
        .collect()
}

pub fn expr_effect(expr: &Expr, funcs: &FunctionTable, resolving: &mut ResolvingSet) -> Effect {
    match expr {
        Expr::SimulatePool { .. } | Expr::ConsensusVote { .. } => Effect::Pure,
        Expr::SynthesizePool { .. } => Effect::Synthesis,
        Expr::SequencePool { .. } => Effect::Sequencing,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, resolving).0,
        Expr::Protect { .. } | Expr::Variable(_) | Expr::StringLiteral(_) => Effect::Pure,
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

pub fn expr_has_required_confirmation(expr: &Expr, funcs: &FunctionTable, resolving: &mut ResolvingSet) -> bool {
    match expr {
        Expr::SimulatePool { .. } | Expr::ConsensusVote { .. } => true,
        Expr::SynthesizePool { confirmed, .. } | Expr::SequencePool { confirmed, .. } => *confirmed,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, resolving).1,
        Expr::Protect { .. } | Expr::Variable(_) | Expr::StringLiteral(_) => true,
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

/// The effect and confirmation state of calling `name`, computed by joining
/// the effects of every declaration in its body. `resolving` guards against
/// infinite recursion on a (mutually-)recursive function: a cycle is
/// reported as `Destructive` and unconfirmed — the conservative choice — so
/// a recursive function can never silently look `Pure`/pre-confirmed just
/// because its own effect couldn't be fully resolved. The SAME `resolving`
/// set must be threaded through every nested lookup within one resolution
/// (never a fresh one per call), or the cycle can't be detected.
fn function_call_effect(name: &str, funcs: &FunctionTable, resolving: &mut ResolvingSet) -> (Effect, bool) {
    let Some(func) = funcs.get(name) else {
        // Undeclared function: typeck's infer_expr already reports this as
        // its own error. Treat as inert here rather than compounding it.
        return (Effect::Pure, true);
    };
    if !resolving.insert(name.to_string()) {
        return (Effect::Destructive, false);
    }
    let mut joint_effect = Effect::Pure;
    let mut confirmed = true;
    for inner in &func.body {
        let info = decl_effect_info(inner, funcs, resolving);
        joint_effect = join_effects(joint_effect, info.effect);
        if info.confirmation_required && !info.confirmed {
            confirmed = false;
        }
    }
    resolving.remove(name);
    (joint_effect, confirmed)
}

pub fn decl_effect_info(decl: &Declaration, funcs: &FunctionTable, resolving: &mut ResolvingSet) -> DeclEffect {
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
            let eff = expr_effect(&binding.expr, funcs, resolving);
            let req = eff == Effect::Synthesis || eff == Effect::Sequencing || eff == Effect::Destructive;
            let conf = expr_has_required_confirmation(&binding.expr, funcs, resolving);
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
            if !resolving.insert(func.name.clone()) {
                return DeclEffect {
                    name: func.name.clone(),
                    kind: "function".into(),
                    effect: Effect::Destructive,
                    confirmation_required: true,
                    confirmed: false,
                };
            }
            let mut joint_effect = Effect::Pure;
            let mut req = false;
            let mut conf = true;
            for inner in &func.body {
                let info = decl_effect_info(inner, funcs, resolving);
                joint_effect = join_effects(joint_effect, info.effect);
                if info.confirmation_required {
                    req = true;
                    if !info.confirmed {
                        conf = false;
                    }
                }
            }
            resolving.remove(&func.name);
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
    let funcs = function_table(program);
    let mut declarations = Vec::new();
    for decl in &program.declarations {
        declarations.push(decl_effect_info(decl, &funcs, &mut ResolvingSet::new()));
    }
    EffectSummary { declarations }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    fn destructive_delete_fn(name: &str, confirmed: bool) -> FunctionDecl {
        FunctionDecl {
            name: name.to_string(),
            params: vec![],
            return_type: TypeExpr::Void,
            body: vec![Declaration::Operation(Operation::Delete(DeleteOp {
                file: "archive.bin".into(),
                pool: "archive".into(),
                confirmed,
            }))],
        }
    }

    #[test]
    fn function_call_inherits_destructive_effect_from_body() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", true));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), Effect::Destructive);
    }

    #[test]
    fn unconfirmed_destructive_call_is_not_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", false));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert!(!expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }

    #[test]
    fn confirmed_destructive_call_is_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", true));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert!(expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }

    #[test]
    fn pure_function_call_needs_no_confirmation() {
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "noop".into(),
            FunctionDecl { name: "noop".into(), params: vec![], return_type: TypeExpr::Void, body: vec![] },
        );
        let call = Expr::FunctionCall { name: "noop".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }

    #[test]
    fn undeclared_function_call_does_not_panic() {
        let funcs = FunctionTable::new();
        let call = Expr::FunctionCall { name: "missing".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }

    #[test]
    fn self_recursive_function_is_treated_conservatively() {
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "loop_fn".into(),
            FunctionDecl {
                name: "loop_fn".into(),
                params: vec![],
                return_type: TypeExpr::Void,
                body: vec![Declaration::Let(LetDecl {
                    name: "x".into(),
                    annotation: TypeExpr::Void,
                    expr: Expr::FunctionCall { name: "loop_fn".into(), args: vec![] },
                })],
            },
        );
        let call = Expr::FunctionCall { name: "loop_fn".into(), args: vec![] };
        // Must terminate (no stack overflow) and must not report Pure/confirmed.
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), Effect::Destructive);
        assert!(!expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }

    #[test]
    fn effect_summary_reflects_function_body_effect() {
        let program = Program {
            declarations: vec![Declaration::Function(destructive_delete_fn("wipe", false))],
        };
        let summary = effect_summary(&program);
        let wipe = summary.declarations.iter().find(|d| d.name == "wipe").unwrap();
        assert_eq!(wipe.effect, Effect::Destructive);
        assert!(wipe.confirmation_required);
        assert!(!wipe.confirmed);
    }
}
