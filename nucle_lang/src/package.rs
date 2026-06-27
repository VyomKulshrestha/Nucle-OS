//! Built-in package registry primitives for NucleScript imports.

use serde::{Deserialize, Serialize};

pub const PRESETS_PACKAGE: &str = "nuclescript/presets";
pub const PRESETS_PACKAGE_NAME: &str = "@vyomkulshrestha/nuclescript-presets";
pub const PRESETS_PACKAGE_VERSION: &str = "0.1.0";
const PRESETS_MANIFEST_JSON: &str = include_str!("../../packages/nuclescript-presets/package.json");

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    #[serde(rename = "import")]
    pub import_source: String,
    pub version: String,
    pub license: String,
    pub description: String,
    pub repository: PackageRepository,
    pub exports: Vec<PackageExport>,
    pub source: String,
    pub readme: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageExport {
    pub name: String,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRepository {
    #[serde(rename = "type")]
    pub repository_type: String,
    pub url: String,
    pub directory: Option<String>,
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

pub fn presets_manifest() -> PackageManifest {
    serde_json::from_str(PRESETS_MANIFEST_JSON)
        .expect("@vyomkulshrestha/nuclescript-presets manifest must be valid JSON")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manifest_matches_builtin_resolver() {
        let manifest = presets_manifest();
        assert_eq!(manifest.name, PRESETS_PACKAGE_NAME);
        assert_eq!(manifest.import_source, PRESETS_PACKAGE);
        assert_eq!(manifest.version, PRESETS_PACKAGE_VERSION);
        for export in &manifest.exports {
            assert!(
                resolve_import(&manifest.import_source, &export.name).is_some(),
                "manifest export '{}' must resolve",
                export.name
            );
        }
    }
}
