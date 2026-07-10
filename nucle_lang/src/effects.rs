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

/// Every function a program can call: the built-ins
/// (`stdlib::builtin_functions`) plus whatever `fn` declarations the
/// program itself declares. Built-ins are seeded first so a program
/// redeclaring one of their names (unusual, but not forbidden) shadows
/// it, the same precedence an ordinary language stdlib gives user code.
pub fn function_table(program: &Program) -> FunctionTable {
    let mut table = crate::stdlib::builtin_functions();
    table.extend(program.declarations.iter().filter_map(|decl| match decl {
        Declaration::Function(func) => Some((func.name.clone(), func.clone())),
        _ => None,
    }));
    table
}

/// `closures` resolves a call to a `let`-bound closure literal to its own
/// real effect, computed by recursing into its actual body -- the one
/// case this analysis CAN see through, because the body is right there
/// in the AST. It is deliberately *not* populated for a `Fn(...)`-typed
/// *parameter*'s call: whatever closure a caller actually passes isn't
/// knowable at all here, so that case falls through to
/// `function_call_effect`'s existing "can't resolve" fallback -- no
/// worse than the pre-existing behavior for any other unresolvable name,
/// just now also covering this one. `effect_summary`'s own top-level
/// pass (used by `nucle explain`) always passes an empty table here: it
/// has no per-scope tracking of its own, and correctness matters more
/// than completeness -- see `decl_effect_info`'s `Declaration::Let` arm.
pub fn expr_effect(expr: &Expr, funcs: &FunctionTable, closures: &FunctionTable, resolving: &mut ResolvingSet) -> Effect {
    match expr {
        Expr::SimulatePool { .. } => Effect::Pure,
        Expr::SynthesizePool { .. } => Effect::Synthesis,
        Expr::SequencePool { .. } => Effect::Sequencing,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, closures, resolving).0,
        Expr::Variable(_) | Expr::StringLiteral(_) | Expr::Number(_) => Effect::Pure,
        // Defining a closure is always inert -- nothing runs until it's
        // called. The call site (`Expr::FunctionCall` above) is where its
        // real effect gets resolved, via `closures`.
        Expr::Closure { .. } => Effect::Pure,
        Expr::BinaryOp { left, right, .. } => {
            join_effects(expr_effect(left, funcs, closures, resolving), expr_effect(right, funcs, closures, resolving))
        }
        Expr::Not(inner) => expr_effect(inner, funcs, closures, resolving),
        // Wrapping something in `?` never changes its effect classification
        // -- `?` is purely a control-flow/unwrap operator, not an operation
        // in its own right, so this forwards to the inner expression.
        Expr::Try(inner) => expr_effect(inner, funcs, closures, resolving),
        // The expression-position and statement-position forms of these
        // operations share the identical `StoreOp`/`RetrieveOp`/`DeleteOp`
        // payload, so they get the identical effect via the same
        // `operation_effect` the statement form already uses -- one
        // resolution path, not two.
        Expr::StoreExpr(op) => operation_effect(&Operation::Store(op.clone())),
        Expr::RetrieveExpr(op) => operation_effect(&Operation::Retrieve(op.clone())),
        Expr::DeleteExpr(op) => operation_effect(&Operation::Delete(op.clone())),
        // Joins the scrutinee and both arms unconditionally, mirroring
        // `Declaration::If`'s existing branch-join: this analysis has
        // never modeled "this branch might not run" (an `If`'s untaken
        // branch already counts), so a `Destructive` operation in only
        // the `Err` arm still requires confirmation.
        Expr::Match { scrutinee, ok_body, err_body, .. } => join_effects(
            join_effects(expr_effect(scrutinee, funcs, closures, resolving), expr_effect(ok_body, funcs, closures, resolving)),
            expr_effect(err_body, funcs, closures, resolving),
        ),
    }
}

pub fn operation_effect(operation: &Operation) -> Effect {
    match operation {
        Operation::Store(store) if store.simulate => Effect::Pure,
        Operation::Store(_) => Effect::Synthesis,
        Operation::Retrieve(_) => Effect::Pure,
        Operation::Delete(_) => Effect::Destructive,
        Operation::Assert(_) => Effect::Pure,
    }
}

pub fn expr_has_required_confirmation(expr: &Expr, funcs: &FunctionTable, closures: &FunctionTable, resolving: &mut ResolvingSet) -> bool {
    match expr {
        Expr::SimulatePool { .. } => true,
        Expr::SynthesizePool { confirmed, .. } | Expr::SequencePool { confirmed, .. } => *confirmed,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, closures, resolving).1,
        Expr::Variable(_) | Expr::StringLiteral(_) | Expr::Number(_) => true,
        // Same reasoning as `expr_effect`'s `Closure` arm: defining one is
        // always inert.
        Expr::Closure { .. } => true,
        Expr::BinaryOp { left, right, .. } => {
            expr_has_required_confirmation(left, funcs, closures, resolving) && expr_has_required_confirmation(right, funcs, closures, resolving)
        }
        Expr::Not(inner) => expr_has_required_confirmation(inner, funcs, closures, resolving),
        Expr::Try(inner) => expr_has_required_confirmation(inner, funcs, closures, resolving),
        // Store's "confirmed" is always true today (see decl_effect_info's
        // Operation::Store arm below) -- store never hard-blocks on
        // confirmation the way Delete/Synthesize/Sequence do, it only
        // drives a separate warning. Retrieve is always Pure, so it's
        // trivially "confirmed". Delete mirrors the statement form exactly.
        Expr::StoreExpr(_) => true,
        Expr::RetrieveExpr(_) => true,
        Expr::DeleteExpr(op) => op.confirmed,
        // All three (scrutinee, both arms) must already be confirmed --
        // same conservative "every declaration in this join counts"
        // reasoning as `expr_effect`'s `Match` arm above.
        Expr::Match { scrutinee, ok_body, err_body, .. } => {
            expr_has_required_confirmation(scrutinee, funcs, closures, resolving)
                && expr_has_required_confirmation(ok_body, funcs, closures, resolving)
                && expr_has_required_confirmation(err_body, funcs, closures, resolving)
        }
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
fn function_call_effect(name: &str, funcs: &FunctionTable, closures: &FunctionTable, resolving: &mut ResolvingSet) -> (Effect, bool) {
    // Global functions take priority (matches typeck's own closures-
    // *before*-global lookup order for resolving *what* gets called --
    // but here it doesn't actually matter which table wins first, since
    // a name can only ever be in one of the two by the time typeck has
    // validated the program: `self.closures`' own duplicate-binding
    // check already rejects a closure shadowing anything, so this `or`
    // is never ambiguous in practice).
    let Some(func) = funcs.get(name).or_else(|| closures.get(name)) else {
        // Undeclared function, or a `Fn(...)`-typed *parameter*'s call
        // (its real body isn't knowable here, only at runtime): typeck's
        // infer_expr already reports the former as its own error; the
        // latter is a real, documented gap (see `expr_effect`'s doc
        // comment) -- treat both as inert rather than guessing wrong.
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
            // No scope-tracking of its own here (unlike `typeck::
            // TypeChecker::check_let`, which passes its real, current
            // `self.closure_decls`) -- an empty table means a call to a
            // `let`-bound closure is treated as inert by this pass alone.
            // This only affects `nucle explain`'s effect summary, a
            // secondary reporting tool; the actual compilation-gating
            // confirmation check (`E-SYNTHESIS-UNCONFIRMED`) runs through
            // `check_let` directly, with real closure information.
            let empty_closures = FunctionTable::new();
            let eff = expr_effect(&binding.expr, funcs, &empty_closures, resolving);
            let req = eff == Effect::Synthesis || eff == Effect::Sequencing || eff == Effect::Destructive;
            let conf = expr_has_required_confirmation(&binding.expr, funcs, &empty_closures, resolving);
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
        Declaration::Operation(Operation::Assert(assert)) => DeclEffect {
            name: assert.message.clone().unwrap_or_else(|| "assert".into()),
            kind: "assert".into(),
            effect: Effect::Pure,
            confirmation_required: false,
            confirmed: true,
        },
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
            // Note for `?`/Result: a body like `let x = risky()?;
            // delete_something();` needs no special-casing here -- this
            // loop already joins EVERY declaration in the body
            // unconditionally, regardless of whether an earlier `?` might
            // short-circuit before `delete_something()` ever runs (it
            // doesn't model "declaration N might not execute" at all,
            // exactly like `If`'s untaken branch above still counts). So a
            // Destructive effect after a `?` still requires confirmation --
            // conservatively correct, and free: `Expr::Try`'s own
            // `expr_effect` arm just forwards to its inner expression's
            // effect, contributing nothing extra to this join.
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
        // `if`/`for` are resolved away by `typeck::check_and_desugar` before
        // codegen ever runs, but effect classification can still see the
        // pre-desugared form (e.g. via `nucle-cli explain` on raw source),
        // so this joins across every branch/the loop body conservatively --
        // a function's effect is the worst case over anything it might run.
        Declaration::If(if_decl) => {
            let mut joint_effect = Effect::Pure;
            let mut req = false;
            let mut conf = true;
            let branches = if_decl.then_branch.iter().chain(if_decl.else_branch.iter().flatten());
            for inner in branches {
                let info = decl_effect_info(inner, funcs, resolving);
                joint_effect = join_effects(joint_effect, info.effect);
                if info.confirmation_required {
                    req = true;
                    if !info.confirmed {
                        conf = false;
                    }
                }
            }
            DeclEffect { name: "if".into(), kind: "if".into(), effect: joint_effect, confirmation_required: req, confirmed: conf }
        }
        Declaration::For(for_decl) => {
            let mut joint_effect = Effect::Pure;
            let mut req = false;
            let mut conf = true;
            for inner in &for_decl.body {
                let info = decl_effect_info(inner, funcs, resolving);
                joint_effect = join_effects(joint_effect, info.effect);
                if info.confirmation_required {
                    req = true;
                    if !info.confirmed {
                        conf = false;
                    }
                }
            }
            DeclEffect { name: for_decl.binding.clone(), kind: "for".into(), effect: joint_effect, confirmation_required: req, confirmed: conf }
        }
        // A test's own effect is the worst case over its body, same
        // reasoning as `if`/`for` above -- a test that stores/deletes
        // something for real should still surface as non-`Pure` in an
        // effect summary, not be hidden just because it's wrapped in
        // `test { ... }`.
        Declaration::Test(test) => {
            let mut joint_effect = Effect::Pure;
            let mut req = false;
            let mut conf = true;
            for inner in &test.body {
                let info = decl_effect_info(inner, funcs, resolving);
                joint_effect = join_effects(joint_effect, info.effect);
                if info.confirmation_required {
                    req = true;
                    if !info.confirmed {
                        conf = false;
                    }
                }
            }
            DeclEffect { name: test.name.clone(), kind: "test".into(), effect: joint_effect, confirmation_required: req, confirmed: conf }
        }
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
            type_params: vec![],
            params: vec![],
            return_type: TypeExpr::Void,
            body: vec![Declaration::Operation(Operation::Delete(DeleteOp {
                file: "archive.bin".into(),
                pool: "archive".into(),
                confirmed,
                span: Span::default(),
            }))],
            span: Span::default(),
            doc: None,
        }
    }

    #[test]
    fn function_call_inherits_destructive_effect_from_body() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", true));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()), Effect::Destructive);
    }

    #[test]
    fn unconfirmed_destructive_call_is_not_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", false));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert!(!expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn confirmed_destructive_call_is_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", true));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![] };
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn pure_function_call_needs_no_confirmation() {
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "noop".into(),
            FunctionDecl { name: "noop".into(), type_params: vec![], params: vec![], return_type: TypeExpr::Void, body: vec![], span: Span::default(), doc: None },
        );
        let call = Expr::FunctionCall { name: "noop".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn undeclared_function_call_does_not_panic() {
        let funcs = FunctionTable::new();
        let call = Expr::FunctionCall { name: "missing".into(), args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn self_recursive_function_is_treated_conservatively() {
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "loop_fn".into(),
            FunctionDecl {
                name: "loop_fn".into(),
                type_params: vec![],
                params: vec![],
                return_type: TypeExpr::Void,
                body: vec![Declaration::Let(LetDecl {
                    name: "x".into(),
                    annotation: TypeExpr::Void,
                    expr: Expr::FunctionCall { name: "loop_fn".into(), args: vec![] },
                    span: Span::default(),
                })],
                span: Span::default(),
                doc: None,
            },
        );
        let call = Expr::FunctionCall { name: "loop_fn".into(), args: vec![] };
        // Must terminate (no stack overflow) and must not report Pure/confirmed.
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()), Effect::Destructive);
        assert!(!expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &mut ResolvingSet::new()));
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
