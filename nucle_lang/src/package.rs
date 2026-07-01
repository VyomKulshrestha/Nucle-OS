//! Built-in package registry primitives for NucleScript imports.

use serde::{Deserialize, Serialize};

pub const PRESETS_PACKAGE: &str = "nuclescript/presets";
pub const PRESETS_PACKAGE_NAME: &str = "@nuclescript/presets";
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


use std::collections::HashMap;
use std::sync::OnceLock;

static REGISTRY: OnceLock<std::sync::Mutex<HashMap<String, PackageManifest>>> = OnceLock::new();

pub fn get_registry() -> &'static std::sync::Mutex<HashMap<String, PackageManifest>> {
    REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        let manifest: PackageManifest = serde_json::from_str(PRESETS_MANIFEST_JSON)
            .expect("@nuclescript/presets manifest must be valid JSON");
        map.insert(manifest.import_source.clone(), manifest);
        std::sync::Mutex::new(map)
    })
}

pub fn resolve_import(source: &str, item: &str) -> Option<Preset> {
    let registry = get_registry().lock().unwrap();
    let manifest = registry.get(source)?;
    let export = manifest.exports.iter().find(|e| e.name == item)?;
    let kind = match export.kind.as_str() {
        "PoolSchema" | "pool_schema" => PresetKind::PoolSchema,
        "Pipeline" | "pipeline" => PresetKind::Pipeline,
        "RecoveryProfile" | "recovery_profile" => PresetKind::RecoveryProfile,
        _ => PresetKind::PoolSchema,
    };
    Some(Preset {
        name: Box::leak(export.name.clone().into_boxed_str()),
        kind,
        description: Box::leak(export.description.clone().into_boxed_str()),
    })
}

pub fn package_exists(source: &str) -> bool {
    get_registry().lock().unwrap().contains_key(source)
}

pub fn register_package(manifest: PackageManifest) {
    get_registry().lock().unwrap().insert(manifest.import_source.clone(), manifest);
}

pub fn list_packages() -> Vec<PackageManifest> {
    get_registry().lock().unwrap().values().cloned().collect()
}

pub fn presets_manifest() -> PackageManifest {
    serde_json::from_str(PRESETS_MANIFEST_JSON)
        .expect("@nuclescript/presets manifest must be valid JSON")
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
