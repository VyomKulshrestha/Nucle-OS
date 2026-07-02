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
    /// Map from file_id to file metadata.
    files: HashMap<String, DnaFile>,
    /// Map from filename to file_id (for lookup by name).
    name_index: HashMap<String, String>,
    /// Map from primer_id to file_id.
    primer_index: HashMap<String, String>,
}

impl Catalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            name_index: HashMap::new(),
            primer_index: HashMap::new(),
        }
    }

    /// Register a file in the catalog.
    pub fn register(&mut self, file: DnaFile) {
        self.name_index.insert(file.filename.clone(), file.file_id.clone());
        self.primer_index.insert(file.primer_id.clone(), file.file_id.clone());
        self.files.insert(file.file_id.clone(), file);
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

    /// Remove a file by filename.
    pub fn remove_by_name(&mut self, filename: &str) -> Option<DnaFile> {
        let file_id = self.name_index.get(filename)?.clone();
        self.remove(&file_id)
    }

    /// List all files.
    pub fn list(&self) -> Vec<&DnaFile> {
        self.files.values().collect()
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
    fn test_catalog_display() {
        let mut catalog = Catalog::new();
        catalog.register(make_file("f1", "test.txt", "P0000"));
        let display = format!("{}", catalog);
        assert!(display.contains("test.txt"));
    }
}
