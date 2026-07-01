//! Package lock file: records a checksum of exactly what a package resolved
//! to — its manifest *and* its `.nsl` source — so `nucle package verify`
//! (and, on a warn-only basis, `nucle run`/`nucle check`) can detect drift
//! between what's locked and what's actually installed.
//!
//! Format: JSON (`nucle.lock`), matching the rest of the CLI's structured
//! output (`--json` everywhere already uses `serde_json`) rather than TOML —
//! there is no other consumer here that would benefit from TOML's comments
//! or Cargo-specific tooling, so JSON avoids adding a second serialization
//! format to the workspace for no gain.

use crate::package::PackageManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LOCK_FILE_NAME: &str = "nucle.lock";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub import_source: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LockFile {
    pub packages: Vec<LockedPackage>,
}

/// SHA-256 checksum of a package's manifest JSON plus its concatenated
/// source files, in order — so editing the manifest *or* the source trips
/// a mismatch, not just the manifest.
pub fn compute_checksum(manifest_json: &str, sources: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest_json.as_bytes());
    for source in sources {
        hasher.update(source.as_bytes());
    }
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// Build a [`LockedPackage`] entry for `manifest`, checksummed against its
/// own JSON plus `sources` (e.g. the package's `.nsl` source files).
pub fn generate(manifest: &PackageManifest, manifest_json: &str, sources: &[&str]) -> LockedPackage {
    LockedPackage {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        import_source: manifest.import_source.clone(),
        checksum: compute_checksum(manifest_json, sources),
    }
}

impl LockFile {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn find(&self, import_source: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.import_source == import_source)
    }

    /// Insert or replace the locked entry for a package.
    pub fn upsert(&mut self, entry: LockedPackage) {
        if let Some(existing) = self.packages.iter_mut().find(|p| p.import_source == entry.import_source) {
            *existing = entry;
        } else {
            self.packages.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_deterministic() {
        let a = compute_checksum("{\"name\":\"x\"}", &["source"]);
        let b = compute_checksum("{\"name\":\"x\"}", &["source"]);
        assert_eq!(a, b);
    }

    #[test]
    fn checksum_changes_with_manifest_content() {
        let a = compute_checksum("{\"name\":\"x\"}", &["source"]);
        let b = compute_checksum("{\"name\":\"y\"}", &["source"]);
        assert_ne!(a, b);
    }

    #[test]
    fn checksum_changes_with_source_content() {
        // The whole point of including sources: editing a package's .nsl
        // source (not just its manifest) must change the checksum.
        let a = compute_checksum("{\"name\":\"x\"}", &["fn a() returns Void {}"]);
        let b = compute_checksum("{\"name\":\"x\"}", &["fn a() returns Void { delete \"x\" from y }"]);
        assert_ne!(a, b);
    }

    #[test]
    fn checksum_with_no_sources_matches_manifest_only() {
        let a = compute_checksum("{\"name\":\"x\"}", &[]);
        let b = compute_checksum("{\"name\":\"x\"}", &[]);
        assert_eq!(a, b);
    }

    #[test]
    fn lock_file_roundtrips_through_json() {
        let mut lock = LockFile::default();
        lock.upsert(LockedPackage {
            name: "@nuclescript/presets".into(),
            version: "0.1.0".into(),
            import_source: "nuclescript/presets".into(),
            checksum: "abc123".into(),
        });
        let json = lock.to_json().unwrap();
        let restored = LockFile::from_json(&json).unwrap();
        assert_eq!(restored.find("nuclescript/presets").unwrap().checksum, "abc123");
    }

    #[test]
    fn generate_produces_a_checksummed_locked_package() {
        use crate::package::{PackageExport, PackageRepository};

        let manifest = PackageManifest {
            name: "@nuclescript/presets".into(),
            import_source: "nuclescript/presets".into(),
            version: "0.1.0".into(),
            license: "MIT".into(),
            description: "test".into(),
            repository: PackageRepository {
                repository_type: "git".into(),
                url: "https://example.com".into(),
                directory: None,
            },
            exports: vec![PackageExport { name: "x".into(), kind: "pool_schema".into(), description: "x".into() }],
            source: "src".into(),
            readme: "readme".into(),
        };
        let manifest_json = serde_json::to_string(&manifest).unwrap();
        let locked = generate(&manifest, &manifest_json, &["fn x() returns Void {}"]);
        assert_eq!(locked.name, "@nuclescript/presets");
        assert_eq!(locked.import_source, "nuclescript/presets");
        assert_eq!(locked.checksum, compute_checksum(&manifest_json, &["fn x() returns Void {}"]));
    }

    #[test]
    fn upsert_replaces_existing_entry() {
        let mut lock = LockFile::default();
        lock.upsert(LockedPackage {
            name: "a".into(),
            version: "0.1.0".into(),
            import_source: "a/pkg".into(),
            checksum: "old".into(),
        });
        lock.upsert(LockedPackage {
            name: "a".into(),
            version: "0.2.0".into(),
            import_source: "a/pkg".into(),
            checksum: "new".into(),
        });
        assert_eq!(lock.packages.len(), 1);
        assert_eq!(lock.find("a/pkg").unwrap().checksum, "new");
    }
}
