//! # File Catalog
//!
//! Manages the mapping: filename → file metadata → primer pair → strand addresses.
//! The catalog is the "filesystem table" of DNA storage.
//!
//! `DnaFile` metadata is defined in the `file` module.

use crate::file::DnaFile;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// File catalog: maps filenames to metadata and DNA addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    /// Map from file_id to file metadata -- *current* versions only. A
    /// filename's prior versions live in `version_history` instead, so
    /// `list()`/`list_prefixed()` (and everything built on them --
    /// `dna_list`, `dna_stat`) only ever see one entry per filename, not
    /// one per historical write.
    files: HashMap<String, DnaFile>,
    /// Map from filename to file_id (for lookup by name).
    name_index: HashMap<String, String>,
    /// Map from primer_id to file_id.
    primer_index: HashMap<String, String>,
    /// Map from filename to its superseded versions, oldest first (the
    /// current version is never in here -- it's in `files`/`name_index`).
    /// Rewriting an existing filename moves what *was* current into this
    /// list rather than deleting it; nothing here ever loses pool strands
    /// until an explicit `remove`/`remove_by_name` (a real `dna_delete`).
    #[serde(default)]
    version_history: HashMap<String, Vec<DnaFile>>,
}

impl Catalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            name_index: HashMap::new(),
            primer_index: HashMap::new(),
            version_history: HashMap::new(),
        }
    }

    /// Register a file in the catalog. If `file.filename` already has a
    /// current version, that version is archived into `version_history`
    /// (not deleted -- its pool strands and primer stay physically
    /// retrievable, see `NucleOS::dna_read_version`/`dna_history`) rather
    /// than being silently replaced.
    pub fn register(&mut self, file: DnaFile) {
        if let Some(old_id) = self.name_index.get(&file.filename).cloned() {
            if let Some(old_file) = self.files.remove(&old_id) {
                self.version_history.entry(file.filename.clone()).or_default().push(old_file);
            }
        }
        self.name_index.insert(file.filename.clone(), file.file_id.clone());
        self.primer_index.insert(file.primer_id.clone(), file.file_id.clone());
        self.files.insert(file.file_id.clone(), file);
    }

    /// Every version of `filename` that isn't current, oldest first.
    /// Empty if the file doesn't exist or has only ever been written once.
    pub fn history(&self, filename: &str) -> &[DnaFile] {
        self.version_history.get(filename).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Look up a specific version of `filename` -- the current one or any
    /// prior one -- by its 1-indexed version number.
    pub fn get_version(&self, filename: &str, version: u32) -> Option<&DnaFile> {
        if let Some(current) = self.get_by_name(filename) {
            if current.version == version {
                return Some(current);
            }
        }
        self.history(filename).iter().find(|f| f.version == version)
    }

    /// Like `get_version`, but mutable -- for persisting a recovery
    /// manifest back onto whichever version (current or historical) was
    /// actually just read, matching `dna_read`'s existing behavior for
    /// the current version.
    pub fn get_version_mut(&mut self, filename: &str, version: u32) -> Option<&mut DnaFile> {
        let is_current = self.get_by_name(filename).is_some_and(|f| f.version == version);
        if is_current {
            return self.get_by_name_mut(filename);
        }
        self.version_history.get_mut(filename)?.iter_mut().find(|f| f.version == version)
    }

    /// Get file metadata by file ID.
    pub fn get(&self, file_id: &str) -> Option<&DnaFile> {
        self.files.get(file_id)
    }

    /// Get file metadata by filename.
    pub fn get_by_name(&self, filename: &str) -> Option<&DnaFile> {
        self.name_index.get(filename)
            .and_then(|id| self.files.get(id))
    }

    /// Get file metadata mutably by filename.
    pub fn get_by_name_mut(&mut self, filename: &str) -> Option<&mut DnaFile> {
        let file_id = self.name_index.get(filename)?.clone();
        self.files.get_mut(&file_id)
    }

    /// Get file metadata by primer ID.
    pub fn get_by_primer(&self, primer_id: &str) -> Option<&DnaFile> {
        self.primer_index.get(primer_id)
            .and_then(|id| self.files.get(id))
    }

    /// Remove a file from the catalog.
    pub fn remove(&mut self, file_id: &str) -> Option<DnaFile> {
        let file = self.files.remove(file_id)?;
        self.name_index.remove(&file.filename);
        self.primer_index.remove(&file.primer_id);
        Some(file)
    }

    /// Removes the current version *and every historical version* of
    /// `filename`, returning all of them (current first, then oldest to
    /// newest). A real delete removes the whole version chain rather than
    /// leaving orphaned history entries with no current version to point
    /// back to -- see `NucleOS::dna_delete`, which uses this (not
    /// `remove_by_name`) so it can also clear each returned version's
    /// pool strands and search index entry.
    pub fn take_all_versions(&mut self, filename: &str) -> Vec<DnaFile> {
        let mut all = Vec::new();
        if let Some(current) = self.remove_by_name(filename) {
            all.push(current);
        }
        if let Some(history) = self.version_history.remove(filename) {
            all.extend(history);
        }
        all
    }

    /// Remove a file by filename.
    pub fn remove_by_name(&mut self, filename: &str) -> Option<DnaFile> {
        let file_id = self.name_index.get(filename)?.clone();
        self.remove(&file_id)
    }

    /// List all files.
    pub fn list(&self) -> Vec<&DnaFile> {
        self.files.values().collect()
    }

    /// Lists files whose name starts with `prefix` (an empty prefix lists
    /// everything). The catalog is still just a flat string → `DnaFile`
    /// map -- names like `"docs/report.txt"` are ordinary keys, not a real
    /// directory tree -- so this is prefix filtering, not tree traversal;
    /// it's what makes that flat namespace still feel like directories
    /// from the CLI (`nucle list docs/`).
    pub fn list_prefixed(&self, prefix: &str) -> Vec<&DnaFile> {
        self.files.values().filter(|f| f.filename.starts_with(prefix)).collect()
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Check if a filename exists.
    pub fn contains_name(&self, filename: &str) -> bool {
        self.name_index.contains_key(filename)
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Catalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "DNA File Catalog ({} files):", self.len())?;
        for file in self.files.values() {
            writeln!(f, "  {}", file)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(id: &str, name: &str, primer: &str) -> DnaFile {
        make_file_versioned(id, name, primer, 1)
    }

    fn make_file_versioned(id: &str, name: &str, primer: &str, version: u32) -> DnaFile {
        DnaFile {
            file_id: id.into(),
            filename: name.into(),
            size: 1024,
            content_hash: vec![0; 8],
            created_at: 1700000000,
            primer_id: primer.into(),
            data_strand_count: 10,
            parity_strand_count: 4,
            rs_parity_per_stripe: 4,
            codec: "ternary".into(),
            redundancy: 1.4,
            manifest: None,
            manifest_history: Vec::new(),
            version,
        }
    }

    #[test]
    fn test_catalog_register_and_get() {
        let mut catalog = Catalog::new();
        let file = make_file("f1", "readme.txt", "P0000");
        catalog.register(file);

        assert_eq!(catalog.len(), 1);
        assert!(catalog.get("f1").is_some());
        assert!(catalog.get_by_name("readme.txt").is_some());
        assert!(catalog.get_by_primer("P0000").is_some());
    }

    #[test]
    fn test_catalog_remove() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "test.txt", "P0000"));
        catalog.register(make_file("f2", "data.bin", "P0001"));

        let removed = catalog.remove_by_name("test.txt");
        assert!(removed.is_some());
        assert_eq!(catalog.len(), 1);
        assert!(!catalog.contains_name("test.txt"));
        assert!(catalog.contains_name("data.bin"));
    }

    #[test]
    fn test_catalog_persistence() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "a.txt", "P0000"));
        catalog.register(make_file("f2", "b.bin", "P0001"));

        let json = catalog.to_json().unwrap();
        let restored = Catalog::from_json(&json).unwrap();

        assert_eq!(restored.len(), 2);
        assert!(restored.get_by_name("a.txt").is_some());
    }

    #[test]
    fn test_file_total_strands() {
        let file = make_file("f1", "test.txt", "P0000");
        assert_eq!(file.total_strands(), 14); // 10 data + 4 parity
    }

    #[test]
    fn test_list_prefixed_filters_by_name_prefix() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "docs/readme.txt", "P0000"));
        catalog.register(make_file("f2", "downloads/readme.txt", "P0001"));
        catalog.register(make_file("f3", "docs/notes.txt", "P0002"));

        let docs = catalog.list_prefixed("docs/");
        assert_eq!(docs.len(), 2);
        assert!(docs.iter().all(|f| f.filename.starts_with("docs/")));

        assert_eq!(catalog.list_prefixed("").len(), 3, "an empty prefix should list everything");
        assert_eq!(catalog.list_prefixed("nonexistent/").len(), 0);
    }

    #[test]
    fn test_same_leaf_name_in_different_prefixes_does_not_collide() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "docs/readme.txt", "P0000"));
        catalog.register(make_file("f2", "downloads/readme.txt", "P0001"));

        assert_eq!(catalog.len(), 2);
        assert!(catalog.get_by_name("docs/readme.txt").is_some());
        assert!(catalog.get_by_name("downloads/readme.txt").is_some());
    }

    #[test]
    fn test_catalog_display() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "test.txt", "P0000"));
        let display = format!("{}", catalog);
        assert!(display.contains("test.txt"));
    }

    #[test]
    fn registering_the_same_filename_twice_archives_the_old_version_instead_of_deleting_it() {
        let mut catalog = Catalog::new();
        catalog.register(make_file_versioned("f1", "notes.txt", "P0000", 1));
        catalog.register(make_file_versioned("f2", "notes.txt", "P0001", 2));

        // Only one entry appears as "the file" -- list()/get_by_name see
        // the current version, not a duplicate per historical write.
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.get_by_name("notes.txt").unwrap().file_id, "f2");
        assert_eq!(catalog.get_by_name("notes.txt").unwrap().version, 2);

        // But the old version is still findable, not deleted.
        let history = catalog.history("notes.txt");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].file_id, "f1");
        assert_eq!(history[0].version, 1);
    }

    #[test]
    fn get_version_resolves_both_current_and_historical_versions() {
        let mut catalog = Catalog::new();
        catalog.register(make_file_versioned("f1", "notes.txt", "P0000", 1));
        catalog.register(make_file_versioned("f2", "notes.txt", "P0001", 2));
        catalog.register(make_file_versioned("f3", "notes.txt", "P0002", 3));

        assert_eq!(catalog.get_version("notes.txt", 1).unwrap().file_id, "f1");
        assert_eq!(catalog.get_version("notes.txt", 2).unwrap().file_id, "f2");
        assert_eq!(catalog.get_version("notes.txt", 3).unwrap().file_id, "f3");
        assert!(catalog.get_version("notes.txt", 4).is_none());
        assert!(catalog.get_version("nonexistent.txt", 1).is_none());
    }

    #[test]
    fn get_version_mut_can_update_a_historical_version_in_place() {
        let mut catalog = Catalog::new();
        catalog.register(make_file_versioned("f1", "notes.txt", "P0000", 1));
        catalog.register(make_file_versioned("f2", "notes.txt", "P0001", 2));

        catalog.get_version_mut("notes.txt", 1).unwrap().size = 42;
        assert_eq!(catalog.history("notes.txt")[0].size, 42);
        // The current version is untouched.
        assert_eq!(catalog.get_by_name("notes.txt").unwrap().size, 1024);
    }

    #[test]
    fn take_all_versions_removes_the_current_version_and_the_entire_history() {
        let mut catalog = Catalog::new();
        catalog.register(make_file_versioned("f1", "notes.txt", "P0000", 1));
        catalog.register(make_file_versioned("f2", "notes.txt", "P0001", 2));
        catalog.register(make_file_versioned("f3", "notes.txt", "P0002", 3));

        let removed = catalog.take_all_versions("notes.txt");
        assert_eq!(removed.len(), 3);
        assert_eq!(removed[0].file_id, "f3", "current version should come first");

        assert!(catalog.get_by_name("notes.txt").is_none());
        assert!(catalog.history("notes.txt").is_empty());
        assert_eq!(catalog.len(), 0);
    }

    #[test]
    fn take_all_versions_on_a_never_written_filename_returns_empty() {
        let mut catalog = Catalog::new();
        assert!(catalog.take_all_versions("nonexistent.txt").is_empty());
    }

    #[test]
    fn history_is_empty_for_a_file_written_only_once() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "notes.txt", "P0000"));
        assert!(catalog.history("notes.txt").is_empty());
    }
}
