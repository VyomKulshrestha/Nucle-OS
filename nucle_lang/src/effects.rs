//! Effect classification and confirmation checks for NucleScript.

use crate::ast::{DeleteOp, Declaration, Effect, Expr, FnEffectAnnotation, FnParam, FunctionDecl, Operation, Program, TypeExpr};
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
/// in the AST. `fn_param_effects` resolves a call to an *effect-
/// annotated* `Fn(...)`-typed parameter (`Fn(...) -> T confirm
/// hardware`/`confirm physical_key`) to the ceiling its declaration
/// promised -- sound because every concrete closure ever bound into such
/// an annotated slot was already checked against that ceiling at its own
/// binding site (see `typeck::TypeChecker::check_fn_effect_compatibility`).
/// An *unannotated* `Fn(...)`-typed parameter's call still falls through
/// to `function_call_effect`'s "can't resolve" fallback -- no worse than
/// the pre-existing behavior for any other unresolvable name, just an
/// explicit, still-accepted opt-in boundary now rather than the only
/// option. `effect_summary`'s own top-level pass (used by `nucle
/// explain`) always passes empty tables here: it has no per-scope
/// tracking of its own, and correctness matters more than completeness
/// -- see `decl_effect_info`'s `Declaration::Let` arm.
pub fn expr_effect(expr: &Expr, funcs: &FunctionTable, closures: &FunctionTable, fn_param_effects: &HashMap<String, Effect>, resolving: &mut ResolvingSet) -> Effect {
    match expr {
        Expr::SimulatePool { .. } => Effect::Pure,
        Expr::SynthesizePool { .. } => Effect::Synthesis,
        Expr::SequencePool { .. } => Effect::Sequencing,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, closures, fn_param_effects, resolving).0,
        Expr::Variable(_) | Expr::StringLiteral(_) | Expr::Number(_) => Effect::Pure,
        // Defining a closure is always inert -- nothing runs until it's
        // called. The call site (`Expr::FunctionCall` above) is where its
        // real effect gets resolved, via `closures`.
        Expr::Closure { .. } => Effect::Pure,
        Expr::BinaryOp { left, right, .. } => {
            join_effects(expr_effect(left, funcs, closures, fn_param_effects, resolving), expr_effect(right, funcs, closures, fn_param_effects, resolving))
        }
        Expr::Not(inner) => expr_effect(inner, funcs, closures, fn_param_effects, resolving),
        // Wrapping something in `?` never changes its effect classification
        // -- `?` is purely a control-flow/unwrap operator, not an operation
        // in its own right, so this forwards to the inner expression.
        Expr::Try(inner) => expr_effect(inner, funcs, closures, fn_param_effects, resolving),
        // Constructing a Result is inert on its own -- same reasoning as
        // `Try`'s forwarding arm; only what's already inside could have
        // an effect.
        Expr::Ok(inner) => expr_effect(inner, funcs, closures, fn_param_effects, resolving),
        Expr::Err(inner) => expr_effect(inner, funcs, closures, fn_param_effects, resolving),
        // Constructing a user enum instance (Step 14) is inert the same
        // way -- only its payload (if any) could have an effect.
        Expr::EnumConstruct { payload, .. } => payload.as_deref().map_or(Effect::Pure, |inner| expr_effect(inner, funcs, closures, fn_param_effects, resolving)),
        // The expression-position and statement-position forms of these
        // operations share the identical `StoreOp`/`RetrieveOp`/`DeleteOp`
        // payload, so they get the identical effect via the same
        // `operation_effect` the statement form already uses -- one
        // resolution path, not two.
        Expr::StoreExpr(op) => operation_effect(&Operation::Store(op.clone())),
        Expr::RetrieveExpr(op) => operation_effect(&Operation::Retrieve(op.clone())),
        Expr::DeleteExpr(op) => operation_effect(&Operation::Delete(op.clone())),
        // Joins the scrutinee and every arm's body unconditionally
        // (Step 14 generalizes this from a fixed two-arm join to N arms),
        // mirroring `Declaration::If`'s existing branch-join: this
        // analysis has never modeled "this branch might not run" (an
        // `If`'s untaken branch already counts), so a `Destructive`
        // operation in any one arm still requires confirmation.
        Expr::Match { scrutinee, arms } => arms
            .iter()
            .fold(expr_effect(scrutinee, funcs, closures, fn_param_effects, resolving), |acc, arm| join_effects(acc, expr_effect(&arm.body, funcs, closures, fn_param_effects, resolving))),
    }
}

/// Does `actual` fall within what `declared` promises? `Pure` always
/// satisfies any ceiling (it's the empty case); `Synthesis`/`Sequencing`
/// only satisfy `Hardware` (the language's own `confirm hardware` already
/// treats them identically); `Destructive` only satisfies `PhysicalKey`.
pub fn effect_satisfies_annotation(actual: Effect, declared: FnEffectAnnotation) -> bool {
    match actual {
        Effect::Pure => true,
        Effect::Synthesis | Effect::Sequencing => declared == FnEffectAnnotation::Hardware,
        Effect::Destructive => declared == FnEffectAnnotation::PhysicalKey,
    }
}

/// The `fn_param_effects` a callable's *own* body should be resolved
/// against: its own annotated `Fn(...)`-typed parameters (always
/// authoritative, since a name collision with an outer scope's own
/// `fn_param_effects` entry could only ever type-check if this callable
/// *itself* also declares that name -- a named function's body can never
/// reference an outer scope's binding at all, and a closure's own
/// parameter shadows whatever it captured) layered over whatever the
/// enclosing scope already had (meaningful only for a closure, which
/// really does capture lexically; harmless-but-unreachable for a named
/// function, which never captures anything, so nothing in `outer` could
/// ever be referenced by its body regardless).
pub fn scoped_fn_param_effects(params: &[FnParam], outer: &HashMap<String, Effect>) -> HashMap<String, Effect> {
    let mut scoped = outer.clone();
    for param in params {
        if let TypeExpr::Fn(_, _, Some(annotation)) = &param.ty {
            scoped.insert(param.name.clone(), annotation.to_effect());
        }
    }
    scoped
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

pub fn expr_has_required_confirmation(expr: &Expr, funcs: &FunctionTable, closures: &FunctionTable, fn_param_effects: &HashMap<String, Effect>, resolving: &mut ResolvingSet) -> bool {
    match expr {
        Expr::SimulatePool { .. } => true,
        Expr::SynthesizePool { confirmed, .. } | Expr::SequencePool { confirmed, .. } => *confirmed,
        Expr::FunctionCall { name, .. } => function_call_effect(name, funcs, closures, fn_param_effects, resolving).1,
        Expr::Variable(_) | Expr::StringLiteral(_) | Expr::Number(_) => true,
        // Same reasoning as `expr_effect`'s `Closure` arm: defining one is
        // always inert.
        Expr::Closure { .. } => true,
        Expr::BinaryOp { left, right, .. } => {
            expr_has_required_confirmation(left, funcs, closures, fn_param_effects, resolving) && expr_has_required_confirmation(right, funcs, closures, fn_param_effects, resolving)
        }
        Expr::Not(inner) => expr_has_required_confirmation(inner, funcs, closures, fn_param_effects, resolving),
        Expr::Try(inner) => expr_has_required_confirmation(inner, funcs, closures, fn_param_effects, resolving),
        Expr::Ok(inner) => expr_has_required_confirmation(inner, funcs, closures, fn_param_effects, resolving),
        Expr::Err(inner) => expr_has_required_confirmation(inner, funcs, closures, fn_param_effects, resolving),
        Expr::EnumConstruct { payload, .. } => payload.as_deref().map_or(true, |inner| expr_has_required_confirmation(inner, funcs, closures, fn_param_effects, resolving)),
        // Store's "confirmed" is always true today (see decl_effect_info's
        // Operation::Store arm below) -- store never hard-blocks on
        // confirmation the way Delete/Synthesize/Sequence do, it only
        // drives a separate warning. Retrieve is always Pure, so it's
        // trivially "confirmed". Delete mirrors the statement form exactly.
        Expr::StoreExpr(_) => true,
        Expr::RetrieveExpr(_) => true,
        Expr::DeleteExpr(op) => op.confirmed,
        // The scrutinee and every arm's body must already be confirmed
        // (Step 14 generalizes this from a fixed two-arm AND to N arms)
        // -- same conservative "every declaration in this join counts"
        // reasoning as `expr_effect`'s `Match` arm above.
        Expr::Match { scrutinee, arms } => {
            expr_has_required_confirmation(scrutinee, funcs, closures, fn_param_effects, resolving)
                && arms.iter().all(|arm| expr_has_required_confirmation(&arm.body, funcs, closures, fn_param_effects, resolving))
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
///
/// `pub` (not private) so `typeck::TypeChecker::check_fn_effect_
/// compatibility` can resolve a named reference to a let-bound closure
/// the identical way a real call to it already would, when validating a
/// concrete closure bound into an effect-annotated `Fn(...)`-typed slot.
pub fn function_call_effect(name: &str, funcs: &FunctionTable, closures: &FunctionTable, fn_param_effects: &HashMap<String, Effect>, resolving: &mut ResolvingSet) -> (Effect, bool) {
    // Global functions take priority (matches typeck's own closures-
    // *before*-global lookup order for resolving *what* gets called --
    // but here it doesn't actually matter which table wins first, since
    // a name can only ever be in one of the two by the time typeck has
    // validated the program: `self.closures`' own duplicate-binding
    // check already rejects a closure shadowing anything, so this `or`
    // is never ambiguous in practice).
    let Some(func) = funcs.get(name).or_else(|| closures.get(name)) else {
        // Undeclared function, or an *unannotated* `Fn(...)`-typed
        // parameter's call (its real body isn't knowable here, only at
        // runtime): typeck's `infer_expr` already reports the former as
        // its own error. An *annotated* one resolves via
        // `fn_param_effects` below instead of reaching here at all; this
        // fallback is now specifically the still-accepted "didn't opt
        // in" case, not the only option -- see `expr_effect`'s doc
        // comment.
        return match fn_param_effects.get(name) {
            Some(&effect) => (effect, true),
            None => (Effect::Pure, true),
        };
    };
    if !resolving.insert(name.to_string()) {
        return (Effect::Destructive, false);
    }
    let scoped = scoped_fn_param_effects(&func.params, fn_param_effects);
    let (joint_effect, confirmed) = body_effect(&func.body, funcs, &scoped, resolving);
    resolving.remove(name);
    (joint_effect, confirmed)
}

/// The effect and confirmation state of a callable's own body -- joining
/// every declaration in it, exactly as `function_call_effect` already
/// did for a *named* callable's body. Extracted so a closure *literal*'s
/// body (which has no name of its own to resolve a call against) can be
/// effect-computed the identical way, from `typeck::TypeChecker::
/// check_fn_effect_compatibility` -- the one place this analysis needs
/// to inspect a concrete closure's real effect without first resolving
/// it by name. No `closures` parameter -- `decl_effect_info`'s own
/// `Declaration::Let` arm always resolves a nested closure call with an
/// empty table regardless (a real, pre-existing, separate limitation of
/// this reporting path, not something this function could change even
/// if it had one to pass through).
pub fn body_effect(body: &[Declaration], funcs: &FunctionTable, fn_param_effects: &HashMap<String, Effect>, resolving: &mut ResolvingSet) -> (Effect, bool) {
    let mut joint_effect = Effect::Pure;
    let mut confirmed = true;
    for inner in body {
        let info = decl_effect_info(inner, funcs, fn_param_effects, resolving);
        joint_effect = join_effects(joint_effect, info.effect);
        if info.confirmation_required && !info.confirmed {
            confirmed = false;
        }
    }
    (joint_effect, confirmed)
}

pub fn decl_effect_info(decl: &Declaration, funcs: &FunctionTable, fn_param_effects: &HashMap<String, Effect>, resolving: &mut ResolvingSet) -> DeclEffect {
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
            // `closures` itself is still hardcoded empty here (unlike
            // `typeck::TypeChecker::check_let`, which passes its real,
            // current `self.closure_decls`) -- a call to a `let`-bound
            // closure declared *inside* this same body is still treated
            // as inert by this pass alone, a real, separate, pre-existing
            // limitation of `nucle explain`'s reporting-only walk, not
            // something this change fixes. `fn_param_effects`, by
            // contrast, *is* the real one passed in -- it's what makes a
            // call to an effect-annotated `Fn(...)`-typed parameter
            // resolve correctly even through this path (see
            // `function_call_effect`'s own `scoped_fn_param_effects`
            // call, which is what populates it correctly for whichever
            // callable's body is currently being walked).
            let empty_closures = FunctionTable::new();
            let eff = expr_effect(&binding.expr, funcs, &empty_closures, fn_param_effects, resolving);
            let req = eff == Effect::Synthesis || eff == Effect::Sequencing || eff == Effect::Destructive;
            let conf = expr_has_required_confirmation(&binding.expr, funcs, &empty_closures, fn_param_effects, resolving);
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
            //
            // `func`'s own annotated `Fn(...)`-typed parameters (if any)
            // are scoped in here via `scoped_fn_param_effects` -- the
            // same call `function_call_effect` makes for a real call --
            // so this reporting-only walk resolves an effect-annotated
            // parameter's call just as accurately as the real
            // confirmation gate does, not just as blindly as it used to.
            let scoped = scoped_fn_param_effects(&func.params, fn_param_effects);
            let (joint_effect, conf) = body_effect(&func.body, funcs, &scoped, resolving);
            let req = joint_effect == Effect::Synthesis || joint_effect == Effect::Sequencing || joint_effect == Effect::Destructive;
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
                let info = decl_effect_info(inner, funcs, fn_param_effects, resolving);
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
                let info = decl_effect_info(inner, funcs, fn_param_effects, resolving);
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
                let info = decl_effect_info(inner, funcs, fn_param_effects, resolving);
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
        Declaration::Enum(decl) => DeclEffect {
            name: decl.name.clone(),
            kind: "enum".into(),
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
        declarations.push(decl_effect_info(decl, &funcs, &HashMap::new(), &mut ResolvingSet::new()));
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
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![], explicit_type_args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Destructive);
    }

    #[test]
    fn unconfirmed_destructive_call_is_not_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", false));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![], explicit_type_args: vec![] };
        assert!(!expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn confirmed_destructive_call_is_confirmed_at_call_site() {
        let mut funcs = FunctionTable::new();
        funcs.insert("wipe".into(), destructive_delete_fn("wipe", true));
        let call = Expr::FunctionCall { name: "wipe".into(), args: vec![], explicit_type_args: vec![] };
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn pure_function_call_needs_no_confirmation() {
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "noop".into(),
            FunctionDecl { name: "noop".into(), type_params: vec![], params: vec![], return_type: TypeExpr::Void, body: vec![], span: Span::default(), doc: None },
        );
        let call = Expr::FunctionCall { name: "noop".into(), args: vec![], explicit_type_args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn undeclared_function_call_does_not_panic() {
        let funcs = FunctionTable::new();
        let call = Expr::FunctionCall { name: "missing".into(), args: vec![], explicit_type_args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
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
                    expr: Expr::FunctionCall { name: "loop_fn".into(), args: vec![], explicit_type_args: vec![] },
                    span: Span::default(),
                })],
                span: Span::default(),
                doc: None,
            },
        );
        let call = Expr::FunctionCall { name: "loop_fn".into(), args: vec![], explicit_type_args: vec![] };
        // Must terminate (no stack overflow) and must not report Pure/confirmed.
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Destructive);
        assert!(!expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
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

    // -------------------------------------------------------------
    // fn_param_effects -- the actual proof the closure-call gap is
    // closed, at the level this analysis operates: a populated
    // `fn_param_effects` table makes a call to an otherwise-
    // unresolvable name (exactly the shape a `Fn(...)`-typed
    // parameter's call has) resolve to the declared ceiling instead
    // of silently falling back to Pure.
    // -------------------------------------------------------------

    #[test]
    fn an_unresolvable_call_with_no_fn_param_effects_entry_is_still_pure() {
        let funcs = FunctionTable::new();
        let call = Expr::FunctionCall { name: "attempt_fn".into(), args: vec![], explicit_type_args: vec![] };
        // The pre-existing, unchanged, still-accepted fallback for an
        // *unannotated* `Fn(...)`-typed parameter's call.
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
    }

    #[test]
    fn an_unresolvable_call_with_a_declared_fn_param_effect_resolves_to_it() {
        let funcs = FunctionTable::new();
        let mut fn_param_effects = HashMap::new();
        fn_param_effects.insert("attempt_fn".to_string(), Effect::Destructive);
        let call = Expr::FunctionCall { name: "attempt_fn".into(), args: vec![], explicit_type_args: vec![] };
        // The actual fix: an effect-annotated `Fn(...)`-typed
        // parameter's call resolves to its declared ceiling, and is
        // trusted as already-confirmed (soundly, since every concrete
        // closure ever bound into that slot was checked against the
        // ceiling at its own binding site -- see
        // `typeck::TypeChecker::check_fn_effect_compatibility`).
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &fn_param_effects, &mut ResolvingSet::new()), Effect::Destructive);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &fn_param_effects, &mut ResolvingSet::new()));
    }

    #[test]
    fn a_named_functions_own_declared_fn_param_effect_propagates_to_its_caller() {
        // The end-to-end demonstration: a plain named function whose
        // body calls its OWN Fn(...)-typed parameter now correctly
        // reports the parameter's declared effect as its own -- before
        // this fix, `run_with_confirm`'s effect would always resolve to
        // Pure regardless of what the parameter's own annotation
        // promised, since nothing populated `fn_param_effects` for it at
        // all.
        let mut funcs = FunctionTable::new();
        funcs.insert(
            "run_with_confirm".into(),
            FunctionDecl {
                name: "run_with_confirm".into(),
                type_params: vec![],
                params: vec![FnParam {
                    name: "op".into(),
                    ty: TypeExpr::Fn(vec![], Box::new(TypeExpr::Void), Some(FnEffectAnnotation::PhysicalKey)),
                }],
                return_type: TypeExpr::Void,
                body: vec![Declaration::Let(LetDecl {
                    name: "x".into(),
                    annotation: TypeExpr::Void,
                    expr: Expr::FunctionCall { name: "op".into(), args: vec![], explicit_type_args: vec![] },
                    span: Span::default(),
                })],
                span: Span::default(),
                doc: None,
            },
        );
        let call = Expr::FunctionCall { name: "run_with_confirm".into(), args: vec![], explicit_type_args: vec![] };
        assert_eq!(expr_effect(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()), Effect::Destructive);
        assert!(expr_has_required_confirmation(&call, &funcs, &FunctionTable::new(), &HashMap::new(), &mut ResolvingSet::new()));
    }
}
