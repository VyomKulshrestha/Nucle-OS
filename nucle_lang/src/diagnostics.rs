use crate::effects::EffectSummary;
use crate::ast::Effect;

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
