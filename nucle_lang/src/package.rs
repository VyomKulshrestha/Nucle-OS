//! Built-in package registry primitives for NucleScript imports.

use serde::{Deserialize, Serialize};

pub const PRESETS_PACKAGE: &str = "nuclescript/presets";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetKind {
    PoolSchema,
    Pipeline,
    RecoveryProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: &'static str,
    pub kind: PresetKind,
    pub description: &'static str,
}

pub fn resolve_import(source: &str, item: &str) -> Option<Preset> {
    if source != PRESETS_PACKAGE {
        return None;
    }
    presets().into_iter().find(|preset| preset.name == item)
}

pub fn package_exists(source: &str) -> bool {
    source == PRESETS_PACKAGE
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "medical_archive",
            kind: PresetKind::PoolSchema,
            description: "Ternary Illumina archive defaults with conservative redundancy.",
        },
        Preset {
            name: "twist_archive",
            kind: PresetKind::PoolSchema,
            description: "Twist synthesis defaults for low-error archival pools.",
        },
        Preset {
            name: "reliable_store",
            kind: PresetKind::Pipeline,
            description: "Encode, protect, store, and verify with optimizer-visible redundancy.",
        },
        Preset {
            name: "illumina_recovery",
            kind: PresetKind::RecoveryProfile,
            description: "Illumina consensus defaults for simulation and planning.",
        },
    ]
}

