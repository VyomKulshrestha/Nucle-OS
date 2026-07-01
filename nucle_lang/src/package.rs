//! Built-in package registry primitives for NucleScript imports.

use serde::{Deserialize, Serialize};

pub const PRESETS_PACKAGE: &str = "nuclescript/presets";
pub const PRESETS_PACKAGE_NAME: &str = "@nuclescript/presets";
pub const PRESETS_PACKAGE_VERSION: &str = "0.1.0";
const PRESETS_MANIFEST_JSON: &str = include_str!("../../packages/nuclescript-presets/package.json");
const PRESETS_SOURCE_NSL: &str = include_str!("../../packages/nuclescript-presets/src/presets.nsl");

const PROFILES_MANIFEST_JSON: &str = include_str!("../../packages/nuclescript-profiles/package.json");
const PROFILES_SOURCE_NSL: &str = include_str!("../../packages/nuclescript-profiles/src/profiles.nsl");

const BENCHMARKS_MANIFEST_JSON: &str = include_str!("../../packages/nuclescript-benchmarks/package.json");
const BENCHMARKS_SOURCE_NSL: &str = include_str!("../../packages/nuclescript-benchmarks/src/benchmarks.nsl");

const RECOVERY_MANIFEST_JSON: &str = include_str!("../../packages/nuclescript-recovery/package.json");
const RECOVERY_SOURCE_NSL: &str = include_str!("../../packages/nuclescript-recovery/src/recovery.nsl");

/// `packages/registry.json` — the index of every package the CLI knows
/// about. This is genuinely parsed at startup (not just present on disk):
/// [`get_registry`] resolves each listed entry's manifest and seeds the
/// in-memory registry from it, so adding a package here is enough for
/// `nucle package inspect/install/verify` to see it.
const REGISTRY_INDEX_JSON: &str = include_str!("../../packages/registry.json");
pub const REGISTRY_INDEX_PATH: &str = "packages/registry.json";

/// One entry in `packages/registry.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub import: String,
    pub version: String,
    pub description: String,
    pub manifest: String,
}

/// The parsed contents of `packages/registry.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub schema: String,
    pub packages: Vec<RegistryEntry>,
}

/// Parse `packages/registry.json`. This is the same source `get_registry()`
/// bootstraps from — exposed separately so callers (e.g. `nucle doctor`,
/// `nucle package install <name>`) can inspect the index without needing
/// the manifests resolved.
pub fn registry_index() -> RegistryIndex {
    serde_json::from_str(REGISTRY_INDEX_JSON)
        .expect("packages/registry.json must be valid JSON matching the registry schema")
}

/// Manifest JSON for every package a `RegistryEntry.manifest` path can name.
/// `include_str!` requires compile-time-known paths, so each bundled
/// package's manifest must be embedded here explicitly; `get_registry()`
/// looks entries up by the path recorded in `packages/registry.json`.
fn embedded_manifest_for_path(path: &str) -> Option<&'static str> {
    match path {
        "packages/nuclescript-presets/package.json" => Some(PRESETS_MANIFEST_JSON),
        "packages/nuclescript-profiles/package.json" => Some(PROFILES_MANIFEST_JSON),
        "packages/nuclescript-benchmarks/package.json" => Some(BENCHMARKS_MANIFEST_JSON),
        "packages/nuclescript-recovery/package.json" => Some(RECOVERY_MANIFEST_JSON),
        _ => None,
    }
}

/// Source files embedded for lock-checksum purposes, keyed by the same
/// `RegistryEntry.manifest` path convention as [`embedded_manifest_for_path`]
/// — a package's checksum covers its manifest *and* its `.nsl` source, so
/// editing the source (not just the manifest) is enough to trip a mismatch.
fn embedded_sources_for_manifest(manifest_path: &str) -> Vec<&'static str> {
    match manifest_path {
        "packages/nuclescript-presets/package.json" => vec![PRESETS_SOURCE_NSL],
        "packages/nuclescript-profiles/package.json" => vec![PROFILES_SOURCE_NSL],
        "packages/nuclescript-benchmarks/package.json" => vec![BENCHMARKS_SOURCE_NSL],
        "packages/nuclescript-recovery/package.json" => vec![RECOVERY_SOURCE_NSL],
        _ => vec![],
    }
}

/// The source file contents that make up a package's lock checksum,
/// resolved by name or import source against `packages/registry.json`.
pub fn checksum_sources(name_or_import: &str) -> Vec<&'static str> {
    let Some(entry) = registry_index()
        .packages
        .into_iter()
        .find(|e| e.name == name_or_import || e.import == name_or_import)
    else {
        return Vec::new();
    };
    embedded_sources_for_manifest(&entry.manifest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetKind {
    PoolSchema,
    Pipeline,
    RecoveryProfile,
    Function,
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
        for entry in registry_index().packages {
            let Some(manifest_json) = embedded_manifest_for_path(&entry.manifest) else {
                log::warn!(
                    "registry.json lists '{}' with manifest '{}', but no manifest is embedded for that path; skipping",
                    entry.name, entry.manifest
                );
                continue;
            };
            match serde_json::from_str::<PackageManifest>(manifest_json) {
                Ok(manifest) => {
                    map.insert(manifest.import_source.clone(), manifest);
                }
                Err(e) => {
                    log::warn!("manifest for '{}' is not valid JSON: {}", entry.name, e);
                }
            }
        }
        std::sync::Mutex::new(map)
    })
}

/// Resolve a package by either its `@scope/name` display name or its
/// `import` source (e.g. `nuclescript/presets`), looking it up against the
/// registry seeded from `packages/registry.json`.
pub fn find_package(name_or_import: &str) -> Option<PackageManifest> {
    let registry = get_registry().lock().unwrap();
    registry
        .values()
        .find(|m| m.name == name_or_import || m.import_source == name_or_import)
        .cloned()
}

/// The raw manifest JSON for a package, as embedded in the binary — needed
/// to compute/verify a lock file checksum against exactly what's installed.
pub fn find_package_manifest_json(name_or_import: &str) -> Option<&'static str> {
    let entry = registry_index()
        .packages
        .into_iter()
        .find(|e| e.name == name_or_import || e.import == name_or_import)?;
    embedded_manifest_for_path(&entry.manifest)
}

pub fn resolve_import(source: &str, item: &str) -> Option<Preset> {
    let registry = get_registry().lock().unwrap();
    let manifest = registry.get(source)?;
    let export = manifest.exports.iter().find(|e| e.name == item)?;
    let kind = match export.kind.as_str() {
        "PoolSchema" | "pool_schema" => PresetKind::PoolSchema,
        "Pipeline" | "pipeline" => PresetKind::Pipeline,
        "RecoveryProfile" | "recovery_profile" => PresetKind::RecoveryProfile,
        "Function" | "function" | "fn" => PresetKind::Function,
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

pub fn validate_manifest(manifest: &PackageManifest) -> Vec<String> {
    let mut errors = Vec::new();
    if manifest.name.is_empty() {
        errors.push("Package name is empty".to_string());
    }
    if manifest.import_source.is_empty() {
        errors.push("Package import source is empty".to_string());
    }
    if manifest.version.is_empty() {
        errors.push("Package version is empty".to_string());
    }
    if manifest.exports.is_empty() {
        errors.push("Package exports list is empty".to_string());
    }
    
    // Temporarily register to check resolve_import
    let original = {
        let registry = get_registry().lock().unwrap();
        registry.get(&manifest.import_source).cloned()
    };
    register_package(manifest.clone());

    for (i, export) in manifest.exports.iter().enumerate() {
        if export.name.is_empty() {
            errors.push(format!("Export #{} name is empty", i));
            continue;
        }
        if export.description.is_empty() {
            errors.push(format!("Export '{}' description is empty", export.name));
        }
        if resolve_import(&manifest.import_source, &export.name).is_none() {
            errors.push(format!("Export '{}' fails to resolve via resolve_import", export.name));
        }
        let kind_valid = match export.kind.as_str() {
            "PoolSchema" | "pool_schema" | "Pipeline" | "pipeline" | "RecoveryProfile" | "recovery_profile" | "Function" | "function" | "fn" => true,
            _ => false,
        };
        if !kind_valid {
            errors.push(format!("Export '{}' has invalid kind '{}'. Must be: pool_schema, pipeline, recovery_profile, function", export.name, export.kind));
        }
    }

    // Restore registry
    {
        let mut registry = get_registry().lock().unwrap();
        if let Some(orig) = original {
            registry.insert(manifest.import_source.clone(), orig);
        } else {
            registry.remove(&manifest.import_source);
        }
    }

    errors
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
    fn registry_is_seeded_from_registry_json() {
        let index = registry_index();
        assert!(!index.packages.is_empty(), "packages/registry.json should list at least one package");
        let entry = index.packages.iter().find(|e| e.import == PRESETS_PACKAGE)
            .expect("registry.json should list the presets package");
        assert_eq!(entry.name, PRESETS_PACKAGE_NAME);
    }

    #[test]
    fn every_registered_package_resolves_and_validates() {
        // Every package.json listed in packages/registry.json — not just
        // presets — must have an embedded manifest, a valid, non-empty
        // manifest, and every export must actually resolve. This is what
        // would catch a package added to the registry but never wired into
        // embedded_manifest_for_path/embedded_sources_for_manifest.
        let index = registry_index();
        assert!(index.packages.len() >= 4, "expected presets, profiles, benchmarks, and recovery to be registered");
        for entry in &index.packages {
            let manifest_json = find_package_manifest_json(&entry.name)
                .unwrap_or_else(|| panic!("no embedded manifest for '{}' — check embedded_manifest_for_path", entry.name));
            let manifest: PackageManifest = serde_json::from_str(manifest_json)
                .unwrap_or_else(|e| panic!("manifest for '{}' is not valid JSON: {}", entry.name, e));
            let errors = validate_manifest(&manifest);
            assert!(errors.is_empty(), "manifest for '{}' failed validation: {:?}", entry.name, errors);

            let sources = checksum_sources(&entry.name);
            assert!(!sources.is_empty(), "no embedded source for '{}' — check embedded_sources_for_manifest", entry.name);
        }
    }

    #[test]
    fn find_package_resolves_by_name_or_import() {
        assert!(find_package(PRESETS_PACKAGE_NAME).is_some());
        assert!(find_package(PRESETS_PACKAGE).is_some());
        assert!(find_package("@nuclescript/does-not-exist").is_none());
    }

    #[test]
    fn find_package_manifest_json_matches_embedded_source() {
        let json = find_package_manifest_json(PRESETS_PACKAGE_NAME).unwrap();
        assert_eq!(json, PRESETS_MANIFEST_JSON);
        assert!(find_package_manifest_json("@nuclescript/does-not-exist").is_none());
    }

    #[test]
    fn presets_package_exports_a_function() {
        // Closes a real gap: PresetKind::Function existed with nothing to
        // back it — zero packages exported one, so it was untested end to
        // end. `archive_with_guarantee` is a real `fn` in presets.nsl.
        let preset = resolve_import(PRESETS_PACKAGE, "archive_with_guarantee")
            .expect("presets package should export archive_with_guarantee");
        assert_eq!(preset.kind, PresetKind::Function);
    }

    #[test]
    fn importing_the_exported_function_type_checks() {
        // The language validates import existence at compile time but does
        // not yet bind imported symbols into scope for use (see
        // docs/examples/preset_imports.nsl) — so this checks the same thing
        // that example does: the import itself compiles cleanly, for a
        // Function-kind export specifically rather than only pool/pipeline
        // ones.
        let src = r#"
            import { archive_with_guarantee } from "nuclescript/presets"
        "#;
        let tokens = crate::Lexer::new(src).tokenize().unwrap();
        let program = crate::Parser::new(tokens).parse_program().unwrap();
        let report = crate::typeck::check_program(&program);
        assert!(!report.has_errors(), "expected import to type-check, got: {:?}", report.diagnostics);
    }

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
        let errors = validate_manifest(&manifest);
        assert!(errors.is_empty(), "Builtin manifest should have no validation errors, got: {:?}", errors);
    }

    #[test]
    fn validate_manifest_catches_invalid_fields() {
        let mut manifest = presets_manifest();
        manifest.name = "".to_string();
        manifest.version = "".to_string();
        manifest.exports[0].description = "".to_string();
        manifest.exports[0].kind = "invalid_kind".to_string();

        let errors = validate_manifest(&manifest);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("name is empty")));
        assert!(errors.iter().any(|e| e.contains("version is empty")));
        assert!(errors.iter().any(|e| e.contains("description is empty")));
        assert!(errors.iter().any(|e| e.contains("invalid kind")));
    }
}
