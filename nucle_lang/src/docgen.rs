//! `nucle doc`: renders a program's `///`-documented declarations
//! (`pool`, `strand`, `seq`, `fn`, `pipeline`) to Markdown.
//!
//! Deliberately narrow in scope: `let`/`if`/`for`/`test`/operations don't
//! carry a `doc` field at all (see `ast::PoolDecl::doc`'s doc comment) --
//! those are statement-like, not the kind of named, referenceable
//! declaration a reader would look up in generated docs. A future built-in
//! whose behavior is fully expressible as an ordinary function (see
//! `stdlib.rs`'s own doc comment on the same theme) could ship as a real
//! package instead of needing this file to know about it specially; this
//! only ever walks whatever `Program` it's given.

use crate::ast::{Declaration, PipelineStep, Program, TypeExpr};
use crate::effects::{decl_effect_info, function_table, ResolvingSet};

/// Renders every documented (and undocumented -- see below) top-level
/// `pool`/`strand`/`seq`/`fn`/`pipeline` declaration in `program` as one
/// Markdown document, grouped by kind. A declaration with no `///` comment
/// still gets an entry (so the output is a complete reference, not just
/// the subset someone remembered to document), just without a
/// description paragraph.
pub fn generate_docs(program: &Program) -> String {
    let funcs = function_table(program);
    let mut out = String::from("# NucleScript Documentation\n");

    render_section(&mut out, "Pools", program, &funcs, |decl| match decl {
        Declaration::Pool(d) => Some(DocEntry {
            name: &d.name,
            doc: d.doc.as_deref(),
            signature: format!(
                "pool {}: DnaPool {{ codec: {}, redundancy: {}x, profile: {} }}",
                d.name, d.codec, d.redundancy, d.profile
            ),
        }),
        _ => None,
    });

    render_section(&mut out, "Strands", program, &funcs, |decl| match decl {
        Declaration::Strand(d) => {
            Some(DocEntry { name: &d.name, doc: d.doc.as_deref(), signature: format!("strand {}: Strand = \"{}\"", d.name, d.sequence) })
        }
        _ => None,
    });

    render_section(&mut out, "Sequences", program, &funcs, |decl| match decl {
        Declaration::Sequence(d) => {
            Some(DocEntry { name: &d.name, doc: d.doc.as_deref(), signature: format!("seq {}: Sequence = \"{}\"", d.name, d.sequence) })
        }
        _ => None,
    });

    render_section(&mut out, "Functions", program, &funcs, |decl| match decl {
        Declaration::Function(d) => Some(DocEntry {
            name: &d.name,
            doc: d.doc.as_deref(),
            signature: format!(
                "fn {}{}({}) -> {}",
                d.name,
                render_type_params(&d.type_params),
                d.params.iter().map(|p| format!("{}: {}", p.name, render_type(&p.ty))).collect::<Vec<_>>().join(", "),
                render_type(&d.return_type)
            ),
        }),
        _ => None,
    });

    render_section(&mut out, "Pipelines", program, &funcs, |decl| match decl {
        Declaration::Pipeline(d) => Some(DocEntry {
            name: &d.name,
            doc: d.doc.as_deref(),
            signature: format!("pipeline {} {{\n    {}\n}}", d.name, render_pipeline_steps(&d.steps)),
        }),
        _ => None,
    });

    out
}

struct DocEntry<'a> {
    name: &'a str,
    doc: Option<&'a str>,
    signature: String,
}

fn render_section<'a>(
    out: &mut String,
    heading: &str,
    program: &'a Program,
    funcs: &crate::effects::FunctionTable,
    extract: impl Fn(&'a Declaration) -> Option<DocEntry<'a>>,
) {
    let entries: Vec<(DocEntry<'a>, &'a Declaration)> =
        program.declarations.iter().filter_map(|decl| extract(decl).map(|entry| (entry, decl))).collect();
    if entries.is_empty() {
        return;
    }

    out.push_str(&format!("\n## {}\n", heading));
    for (entry, decl) in entries {
        out.push_str(&format!("\n### `{}`\n", entry.name));
        if let Some(doc) = entry.doc {
            out.push_str(&format!("\n{}\n", doc));
        }
        out.push_str(&format!("\n```nuclescript\n{}\n```\n", entry.signature));

        let info = decl_effect_info(decl, funcs, &mut ResolvingSet::new());
        let confirmation = if !info.confirmation_required {
            String::new()
        } else if info.confirmed {
            " (confirmed)".to_string()
        } else {
            " (**requires confirmation**)".to_string()
        };
        out.push_str(&format!("\n**Effect:** {}{}\n", info.effect, confirmation));
    }
}

/// Also reused by `typeck.rs` for Result/`?` diagnostic messages that
/// need to name a type -- one renderer, not a duplicate ad hoc one per
/// call site.
pub(crate) fn render_type(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Pool(pool_type) => match pool_type.error_rate_percent {
            Some(rate) => format!("Pool<{}, {}%>", pool_type.state, rate),
            None => format!("Pool<{}>", pool_type.state),
        },
        TypeExpr::Strand => "Strand".to_string(),
        TypeExpr::Sequence => "Sequence".to_string(),
        TypeExpr::File => "File".to_string(),
        TypeExpr::DnaFile => "DnaFile".to_string(),
        TypeExpr::Recovery => "Recovery".to_string(),
        TypeExpr::Void => "Void".to_string(),
        TypeExpr::Result(ok, err) => format!("Result<{}, {}>", render_type(ok), render_type(err)),
        TypeExpr::Str => "Str".to_string(),
        TypeExpr::Fn(params, ret) => format!(
            "Fn({}) -> {}",
            params.iter().map(render_type).collect::<Vec<_>>().join(", "),
            render_type(ret)
        ),
    }
}

/// Renders a function's `<T, U>` type-parameter list for its signature,
/// or an empty string for a non-generic function (the overwhelming
/// majority) -- also reused by `nucle_lsp/src/backend.rs`'s hover text,
/// same reuse pattern as `render_type`.
pub(crate) fn render_type_params(type_params: &[String]) -> String {
    if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    }
}

fn render_pipeline_steps(steps: &[PipelineStep]) -> String {
    steps
        .iter()
        .map(|step| match step {
            PipelineStep::Encode { path, codec } => format!("encode \"{}\" using {}", path, codec),
            PipelineStep::Protect { redundancy } => format!("protect with redundancy {}x", redundancy),
            PipelineStep::Store { pool } => format!("store into {}", pool),
            PipelineStep::VerifyRoundtrip => "verify roundtrip".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",\n    ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn documents_a_function_with_a_doc_comment() {
        let program = Program {
            declarations: vec![Declaration::Function(FunctionDecl {
                name: "archive_it".into(),
                type_params: vec![],
                params: vec![FnParam { name: "data".into(), ty: TypeExpr::File }],
                return_type: TypeExpr::DnaFile,
                body: Vec::new(),
                span: Span::default(),
                doc: Some("Archives a file with the given recovery guarantee.".into()),
            })],
        };
        let docs = generate_docs(&program);
        assert!(docs.contains("## Functions"));
        assert!(docs.contains("### `archive_it`"));
        assert!(docs.contains("Archives a file with the given recovery guarantee."));
        assert!(docs.contains("fn archive_it(data: File) -> DnaFile"));
        assert!(docs.contains("**Effect:** Pure"));
    }

    #[test]
    fn documents_a_pool_without_a_doc_comment_but_omits_the_description() {
        let program = Program {
            declarations: vec![Declaration::Pool(PoolDecl {
                name: "archive".into(),
                codec: Codec::Ternary,
                redundancy: 3,
                profile: Profile::Illumina,
                span: Span::default(),
                doc: None,
            })],
        };
        let docs = generate_docs(&program);
        assert!(docs.contains("### `archive`"));
        assert!(docs.contains("pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }"));
    }

    #[test]
    fn omits_empty_sections() {
        let program = Program { declarations: vec![] };
        let docs = generate_docs(&program);
        assert!(!docs.contains("## Pools"));
        assert!(!docs.contains("## Functions"));
    }

    #[test]
    fn flags_an_unconfirmed_destructive_declaration() {
        // Destructive/synthesis effects only ever appear directly as
        // top-level Operations, which docgen doesn't document (no `doc`
        // field on Operation) -- but a function *wrapping* one should
        // still show the joined effect on the function itself.
        let program = Program {
            declarations: vec![Declaration::Function(FunctionDecl {
                name: "wipe".into(),
                type_params: vec![],
                params: vec![],
                return_type: TypeExpr::Void,
                body: vec![Declaration::Operation(Operation::Delete(DeleteOp {
                    file: "archive.bin".into(),
                    pool: "archive".into(),
                    confirmed: false,
                    span: Span::default(),
                }))],
                span: Span::default(),
                doc: None,
            })],
        };
        let docs = generate_docs(&program);
        assert!(docs.contains("**Effect:** Destructive (**requires confirmation**)"));
    }
}
