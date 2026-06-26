//! Playground-facing compiler API.

use crate::codegen;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::sim_backend;
use crate::typeck::{self, DiagnosticLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaygroundReport {
    pub ok: bool,
    pub diagnostics: Vec<PlaygroundDiagnostic>,
    pub simulation_steps: Vec<String>,
    pub optimiser_notes: Vec<String>,
    pub vfs_call_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaygroundDiagnostic {
    pub level: String,
    pub message: String,
}

pub fn analyze_source(source: &str) -> PlaygroundReport {
    let tokens = match Lexer::new(source).tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            return PlaygroundReport {
                ok: false,
                diagnostics: vec![PlaygroundDiagnostic {
                    level: "error".into(),
                    message: format!("lex error: {}", err),
                }],
                simulation_steps: Vec::new(),
                optimiser_notes: Vec::new(),
                vfs_call_count: 0,
            };
        }
    };

    let program = match Parser::new(tokens).parse_program() {
        Ok(program) => program,
        Err(err) => {
            return PlaygroundReport {
                ok: false,
                diagnostics: vec![PlaygroundDiagnostic {
                    level: "error".into(),
                    message: format!("parse error: {}", err),
                }],
                simulation_steps: Vec::new(),
                optimiser_notes: Vec::new(),
                vfs_call_count: 0,
            };
        }
    };

    let type_report = typeck::check_program(&program);
    let diagnostics = type_report
        .diagnostics
        .iter()
        .map(|diagnostic| PlaygroundDiagnostic {
            level: match diagnostic.level {
                DiagnosticLevel::Error => "error".into(),
                DiagnosticLevel::Warning => "warning".into(),
            },
            message: diagnostic.message.clone(),
        })
        .collect::<Vec<_>>();

    if type_report.has_errors() {
        return PlaygroundReport {
            ok: false,
            diagnostics,
            simulation_steps: Vec::new(),
            optimiser_notes: Vec::new(),
            vfs_call_count: 0,
        };
    }

    let simulation = sim_backend::compile_simulation(program.clone(), type_report.clone());
    let compiled = codegen::compile_program(program, type_report);
    PlaygroundReport {
        ok: true,
        diagnostics,
        simulation_steps: simulation.steps.iter().map(ToString::to_string).collect(),
        optimiser_notes: simulation.optimiser_notes,
        vfs_call_count: compiled.calls.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playground_report_contains_simulation_steps() {
        let source = r#"
            import { medical_archive } from "nuclescript/presets"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
        "#;
        let report = analyze_source(source);
        assert!(report.ok);
        assert_eq!(report.vfs_call_count, 0);
        assert_eq!(report.simulation_steps.len(), 2);
    }

    #[test]
    fn playground_report_rejects_unknown_import() {
        let source = r#"import { missing } from "nuclescript/presets""#;
        let report = analyze_source(source);
        assert!(!report.ok);
        assert_eq!(report.diagnostics[0].level, "error");
    }
}
