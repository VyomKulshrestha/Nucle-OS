//! Import expansion: `import { name as alias } from "nuclescript/pkg"`
//! brings the package's real declarations into the program.
//!
//! Before type checking, every imported item that is a pool, `let` binding,
//! function, enum, strand or sequence in the package's embedded `.nsl`
//! source is spliced into the program right after its `import`, together
//! with every other declaration from that package it refers to (an imported
//! function brings the pools and enums its body names). An `as` alias
//! renames the imported item; its dependencies keep their package names.
//!
//! Pipeline exports stay validate-only: declaring a pipeline runs it, and
//! importing a template should never store files as a side effect.
//!
//! Spliced declarations carry the `import` statement's source span, so a
//! diagnostic about imported code points at the line that imported it. A
//! package source that fails to parse contributes nothing; the import is
//! still checked by name, as before.

use crate::ast::{Declaration, ImportDecl, Program, Span};
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};

/// The program with each import's declarations spliced in after it.
pub fn expand_imports(program: &Program) -> Program {
    if !program.declarations.iter().any(|d| matches!(d, Declaration::Import(_))) {
        return program.clone();
    }
    let mut out = Vec::with_capacity(program.declarations.len());
    let mut spliced: HashSet<(String, String)> = HashSet::new();
    for decl in &program.declarations {
        out.push(decl.clone());
        if let Declaration::Import(import) = decl {
            out.extend(expand_one(import, &mut spliced));
        }
    }
    Program { declarations: out }
}

fn expand_one(import: &ImportDecl, spliced: &mut HashSet<(String, String)>) -> Vec<Declaration> {
    let package: Vec<Declaration> = crate::package::checksum_sources(&import.source)
        .into_iter()
        .filter_map(|src| {
            let tokens = Lexer::new(src).tokenize().ok()?;
            Parser::new(tokens).parse_program().ok()
        })
        .flat_map(|p| p.declarations)
        .collect();
    let by_name: HashMap<String, &Declaration> =
        package.iter().filter_map(|d| importable_name(d).map(|n| (n.to_string(), d))).collect();

    let mut result = Vec::new();
    for item in &import.items {
        let Some(&root) = by_name.get(&item.name) else { continue };
        // Dependencies first, so each declaration is checked after what it uses.
        let mut order = Vec::new();
        collect_dependencies(root, &by_name, &mut HashSet::from([item.name.clone()]), &mut order);
        for dep in order {
            let key = (import.source.clone(), dep.clone());
            if spliced.insert(key) {
                result.push(respan(by_name[&dep], import.span));
            }
        }
        let alias = item.alias.clone().unwrap_or_else(|| item.name.clone());
        if spliced.insert((import.source.clone(), alias.clone())) {
            let mut decl = respan(root, import.span);
            rename(&mut decl, &alias);
            result.push(decl);
        }
    }
    result
}

/// Declarations an import can bring in, by name. Pipelines are excluded on
/// purpose (see the module docs); imports, operations, tests and control
/// flow have no importable name.
fn importable_name(decl: &Declaration) -> Option<&str> {
    match decl {
        Declaration::Pool(d) => Some(&d.name),
        Declaration::Let(d) => Some(&d.name),
        Declaration::Function(d) => Some(&d.name),
        Declaration::Enum(d) => Some(&d.name),
        Declaration::Strand(d) => Some(&d.name),
        Declaration::Sequence(d) => Some(&d.name),
        _ => None,
    }
}

/// Every package declaration `decl` names, transitively, in dependency
/// order (a declaration's own dependencies before it).
fn collect_dependencies(
    decl: &Declaration,
    by_name: &HashMap<String, &Declaration>,
    visiting: &mut HashSet<String>,
    order: &mut Vec<String>,
) {
    let value = serde_json::to_value(decl).unwrap_or(serde_json::Value::Null);
    let mut names = Vec::new();
    strings_in(&value, &mut names);
    for name in names {
        if by_name.contains_key(&name) && visiting.insert(name.clone()) {
            collect_dependencies(by_name[&name], by_name, visiting, order);
            order.push(name);
        }
    }
}

fn strings_in(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings_in(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| strings_in(v, out)),
        _ => {}
    }
}

/// A copy of `decl` with every source span replaced by `span`.
fn respan(decl: &Declaration, span: Span) -> Declaration {
    let mut value = serde_json::to_value(decl).expect("AST serializes");
    let span_value = serde_json::to_value(span).expect("Span serializes");
    replace_spans(&mut value, &span_value);
    serde_json::from_value(value).expect("AST round-trips through serde")
}

fn replace_spans(value: &mut serde_json::Value, span: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if key == "span" && v.is_object() {
                    *v = span.clone();
                } else {
                    replace_spans(v, span);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| replace_spans(v, span)),
        _ => {}
    }
}

fn rename(decl: &mut Declaration, name: &str) {
    match decl {
        Declaration::Pool(d) => d.name = name.to_string(),
        Declaration::Let(d) => d.name = name.to_string(),
        Declaration::Function(d) => d.name = name.to_string(),
        Declaration::Enum(d) => d.name = name.to_string(),
        Declaration::Strand(d) => d.name = name.to_string(),
        Declaration::Sequence(d) => d.name = name.to_string(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Program {
        Parser::new(Lexer::new(src).tokenize().unwrap()).parse_program().unwrap()
    }

    fn names(p: &Program) -> Vec<String> {
        p.declarations.iter().filter_map(|d| importable_name(d).map(str::to_string)).collect()
    }

    #[test]
    fn an_imported_function_brings_the_package_items_it_uses() {
        let p = expand_imports(&parse("import { recover_with_consensus } from \"nuclescript/recovery\""));
        let n = names(&p);
        assert!(n.contains(&"recover_with_consensus".to_string()), "{n:?}");
    }

    #[test]
    fn an_imported_let_brings_its_pool_and_inputs_in_dependency_order() {
        let p = expand_imports(&parse("import { illumina_recovery } from \"nuclescript/presets\""));
        let n = names(&p);
        let pos = |x: &str| n.iter().position(|y| y == x).unwrap_or_else(|| panic!("{x} missing from {n:?}"));
        assert!(pos("medical_archive") < pos("illumina_noisy"));
        assert!(pos("illumina_noisy") < pos("illumina_recovery"));
    }

    #[test]
    fn alias_renames_only_the_imported_item_and_pipelines_are_not_spliced() {
        let p = expand_imports(&parse(
            "import { medical_archive as vault, reliable_store } from \"nuclescript/presets\"",
        ));
        let n = names(&p);
        assert!(n.contains(&"vault".to_string()) && !n.contains(&"medical_archive".to_string()), "{n:?}");
        assert!(!p.declarations.iter().any(|d| matches!(d, Declaration::Pipeline(_))));
    }

    #[test]
    fn spliced_code_points_at_the_import_line() {
        let p = expand_imports(&parse("\n\nimport { twist_archive } from \"nuclescript/presets\""));
        let Declaration::Pool(pool) = &p.declarations[1] else { panic!("expected the spliced pool") };
        assert_eq!(pool.span.line, 3);
    }

    #[test]
    fn programs_without_imports_are_untouched() {
        let src = parse("pool a: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }");
        assert_eq!(expand_imports(&src), src);
    }
}
