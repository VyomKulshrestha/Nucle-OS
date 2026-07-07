use crate::effects::EffectSummary;
use crate::ast::Effect;
use crate::typeck::Diagnostic;

/// Render a diagnostic the way rustc/cargo do: `file:line:column: level
/// [code]: message`, followed by the offending source line and a `^^^`
/// underline under the diagnostic's exact span. Pure string formatting
/// over what Step 0 (spans) and Step 1 (codes) already compute -- no new
/// analysis, just making the existing data visible.
pub fn render_snippet(path: &str, source: &str, diagnostic: &Diagnostic) -> String {
    let span = diagnostic.span;
    if span.line == 0 {
        // A synthetic/spanless diagnostic (shouldn't happen from real
        // source, but hand-built programs in tests can produce one) --
        // fall back to a plain one-liner rather than underlining nothing.
        return format!("{}: {} [{}]: {}", path, diagnostic.level, diagnostic.code, diagnostic.message);
    }

    let line_text = source.lines().nth(span.line - 1).unwrap_or("");
    let gutter = format!("{}", span.line);
    let pad = " ".repeat(gutter.len());

    // Underline from the span's start column to its end column on the
    // same line; a span that ends on a later line (rare, only for
    // multi-line constructs) just underlines to the end of the first
    // line rather than trying to render a multi-line caret block.
    let underline_start = span.column.saturating_sub(1);
    let underline_len = if span.end_line == span.line && span.end_column > span.column {
        span.end_column - span.column
    } else {
        1
    };
    let underline = format!(
        "{}{}",
        " ".repeat(underline_start),
        "^".repeat(underline_len.max(1))
    );

    format!(
        "{path}:{line}:{column}: {level} [{code}]: {message}\n{pad} |\n{gutter} | {line_text}\n{pad} | {underline}",
        path = path,
        line = span.line,
        column = span.column,
        level = diagnostic.level,
        code = diagnostic.code,
        message = diagnostic.message,
        pad = pad,
        gutter = gutter,
        line_text = line_text,
        underline = underline,
    )
}

pub fn generate_explanation(notes: &[String], summary: &EffectSummary) -> String {
    let mut explanation = String::new();

    explanation.push_str("--- Execution & Safety Explanation ---\n\n");

    if !notes.is_empty() {
        explanation.push_str("### Optimization Decisions:\n");
        for note in notes {
            if note.contains("raised redundancy") {
                explanation.push_str(&format!("- {}. Redundancy was increased to satisfy statistical recovery guarantees under this profile's specific error profile.\n", note));
            } else if note.contains("still carries") {
                explanation.push_str(&format!("- {}. The consensus voting strategy was not able to fully eliminate errors due to limited coverage.\n", note));
            } else {
                explanation.push_str(&format!("- {}\n", note));
            }
        }
        explanation.push_str("\n");
    }

    explanation.push_str("### Safety & Confirmation Summary:\n");
    for decl in &summary.declarations {
        let status = if decl.confirmation_required {
            if decl.confirmed {
                "CONFIRMED"
            } else {
                "REQUIRES CONFIRMATION"
            }
        } else {
            "SAFE (Pure)"
        };
        explanation.push_str(&format!(
            "- {} '{}' ({}): {} effect. [{}]\n",
            decl.kind, decl.name, decl.effect, decl.effect, status
        ));
        if decl.confirmation_required && !decl.confirmed {
            match decl.effect {
                Effect::Synthesis => {
                    explanation.push_str("  -> WARNING: This operation performs DNA synthesis on hardware. Explicit confirmation ('confirm hardware') is required to proceed.\n");
                }
                Effect::Sequencing => {
                    explanation.push_str("  -> WARNING: This operation performs DNA sequencing on hardware. Explicit confirmation ('confirm hardware') is required to proceed.\n");
                }
                Effect::Destructive => {
                    explanation.push_str("  -> WARNING: This operation permanently destroys physical DNA tubes. Explicit physical key confirmation ('confirm physical_key') is required to proceed.\n");
                }
                _ => {}
            }
        }
    }

    explanation
}
