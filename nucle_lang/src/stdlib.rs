//! NucleScript's built-in functions, defined as ordinary `FunctionDecl`s
//! rather than hardcoded AST variants or parser/typeck special cases --
//! the same pattern real language implementations use for intrinsics
//! (`size_of::<T>()`, `std::mem::swap`, etc.): the function *exists* as a
//! real, resolvable entry a caller can look up, get arity-checked
//! against, and see effect-propagate through, exactly like a user's own
//! `fn`; only the small amount of behavior that's genuinely
//! value-dependent (not expressible by a fixed, declared signature) gets
//! a narrow, explicit intrinsic-recognition branch in `typeck`/`middle`
//! keyed off the function's *name*.
//!
//! `consensus_vote` and `protect` are the two expression-position
//! keywords that were nothing more than "a function that happens to be
//! built in" -- `simulate`/`synthesize`/`sequence` stay as dedicated
//! grammar forms since their effect-confirmation semantics
//! (`confirm hardware`) are load-bearing enough to want a real grammar
//! form rather than being just sugar over a call.
//!
//! The parser still accepts `consensus_vote(...)`'s and `protect ...
//! for ...`'s friendly surface syntax (see `parser::parse_primary_expr`)
//! -- both desugar directly to `Expr::FunctionCall` at parse time, so
//! every consumer downstream (`typeck`, `effects`, `middle`) only ever
//! has one representation to handle, not two.

use crate::ast::{FnParam, FunctionDecl, Span, TypeExpr};
use crate::effects::FunctionTable;

/// The `FunctionTable` entry for every built-in function, keyed by name --
/// merged into a program's own function table (see
/// `effects::function_table` and `typeck::TypeChecker`) so a call to
/// `consensus_vote`/`protect` resolves through the exact same lookup a
/// call to a user-defined function does.
///
/// Parameter types here are deliberately loose (`TypeExpr::Void`, which
/// the argument-type-checking loop in `typeck::infer_expr` simply skips):
/// `consensus_vote`'s source parameter accepts a probabilistic pool
/// binding in *any* state/profile, and NucleScript's type system has no
/// "any pool" or plain-number parameter type to express that precisely
/// today. Getting that fully sound would mean extending `TypeExpr` with a
/// generic/numeric parameter form -- a real type-system extension, out of
/// scope for "stop hardcoding capabilities as syntax." Arity is still
/// checked for real (an empty `params` list would silently accept any
/// argument count), and each function's actual return behavior is
/// computed by the intrinsic-recognition branches in
/// `typeck::TypeChecker::infer_consensus_vote` and
/// `middle::infer_binding`, not by these placeholder types.
pub fn builtin_functions() -> FunctionTable {
    let mut table = FunctionTable::new();
    table.insert("consensus_vote".to_string(), consensus_vote_decl());
    table.insert("protect".to_string(), protect_decl());
    table
}

fn consensus_vote_decl() -> FunctionDecl {
    FunctionDecl {
        name: "consensus_vote".to_string(),
        params: vec![
            FnParam { name: "source".to_string(), ty: TypeExpr::Void },
            FnParam { name: "coverage".to_string(), ty: TypeExpr::Void },
        ],
        // Declared as `Recovery` (a fixed, checkable shape) purely so a
        // caller with no special-case knowledge of this function still
        // sees *something* sane; the real, value-dependent
        // `Pool<Recovered, X%>` result comes from
        // `TypeChecker::infer_consensus_vote`, which never consults this
        // field.
        return_type: TypeExpr::Recovery,
        body: Vec::new(),
        span: Span::default(),
    }
}

fn protect_decl() -> FunctionDecl {
    FunctionDecl {
        name: "protect".to_string(),
        params: vec![
            FnParam { name: "data".to_string(), ty: TypeExpr::Void },
            FnParam { name: "guarantee".to_string(), ty: TypeExpr::Void },
        ],
        return_type: TypeExpr::Void,
        body: Vec::new(),
        span: Span::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_functions_are_pure_and_confirmed_via_the_shared_effect_path() {
        use crate::effects::{expr_effect, expr_has_required_confirmation, ResolvingSet};
        use crate::ast::Expr;

        let funcs = builtin_functions();
        let call = Expr::FunctionCall {
            name: "consensus_vote".to_string(),
            args: vec![Expr::Variable("noisy".to_string()), Expr::Number(10.0)],
        };
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), crate::ast::Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));

        let call = Expr::FunctionCall {
            name: "protect".to_string(),
            args: vec![Expr::Variable("data".to_string()), Expr::Variable("guarantee".to_string())],
        };
        assert_eq!(expr_effect(&call, &funcs, &mut ResolvingSet::new()), crate::ast::Effect::Pure);
        assert!(expr_has_required_confirmation(&call, &funcs, &mut ResolvingSet::new()));
    }
}
