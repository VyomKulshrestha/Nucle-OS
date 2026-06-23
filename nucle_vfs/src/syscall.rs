//! # Syscall-Style API — `dna_write()`, `dna_read()`, `dna_stat()`
//!
//! The top-level API that ties every layer together:
//!
//! ```text
//! dna_write(name, data) → Codec → ECC → Primers → Pool
//! dna_read(name)        → Pool → Primers → ECC → Codec → data
//! dna_stat()            → Pool stats, file listing, health
//! dna_delete(name)      → Remove from Pool + Catalog
//! ```

use crate::pool::{DnaPool, PoolEntry};
use crate::file::DnaFile;
use crate::catalog::Catalog;
use nucle_codec::base::{DnaCodec, StrandCollection};
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_index::primer::PrimerLibrary;
use nucle_index::search::{SearchEngine, FileMeta, SearchResult};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// NucleOS — the unified DNA storage OS
// ---------------------------------------------------------------------------

/// The main DNA storage operating system.
///
/// Combines all layers into a single interface:
/// - Codec (encode/decode)
/// - ECC (error correction — not applied in basic write for now)
/// - Index (primers, search)
/// - VFS (pool, catalog)
pub struct NucleOS {
    /// DNA strand storage pool.
    pub pool: DnaPool,
    /// File metadata catalog.
    pub catalog: Catalog,
    /// Primer library for file addressing.
    pub primers: PrimerLibrary,
    /// Search engine.
    pub search: SearchEngine,
    /// Number of primer pairs used so far.
    primers_used: usize,
}

impl NucleOS {
    /// Initialize a new NucleOS instance.
    ///
    /// `max_files`: maximum number of files (determines primer library size).
    pub fn new(max_files: usize) -> Self {
        let primers = PrimerLibrary::generate(max_files.max(10), 20, 42);
        let search = SearchEngine::new(primers.clone());
        Self {
            pool: DnaPool::new(),
            catalog: Catalog::new(),
            primers,
            search,
            primers_used: 0,
        }
    }

    /// Create with default capacity (100 files).
    pub fn default_os() -> Self {
        Self::new(100)
    }

    // -----------------------------------------------------------------------
    // dna_write — store a file into DNA
    // -----------------------------------------------------------------------

    /// Store binary data as a file in DNA storage.
    ///
    /// Pipeline: data → encode → tag with primers → store in pool
    pub fn dna_write(&mut self, filename: &str, data: &[u8]) -> Result<WriteResult, String> {
        // Check if filename already exists
        if self.catalog.contains_name(filename) {
            return Err(format!("file '{}' already exists", filename));
        }

        // Assign a primer pair
        let primer_pair = self.primers
            .assign_next(self.primers_used)
            .ok_or("no primer pairs available")?
            .clone();
        self.primers_used += 1;

        // Encode using ternary codec
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let encoded = codec.encode(data)
            .map_err(|e| format!("encoding failed: {}", e))?;

        // Tag each strand with primers
        let tagged_strands: Vec<_> = encoded.strands.iter()
            .map(|s| primer_pair.tag_strand(s))
            .collect();

        // Compute content hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hasher.finalize();
        let content_hash = hash[..8].to_vec();

        // Get timestamp
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Generate file ID
        let file_id = self.catalog.next_file_id();

        // Store strands in pool
        for (i, strand) in tagged_strands.iter().enumerate() {
            self.pool.add_strand(PoolEntry {
                strand: strand.clone(),
                file_id: file_id.clone(),
                strand_index: i,
                is_parity: false,
            });
        }

        // Register in catalog
        let dna_file = DnaFile {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            size: data.len(),
            content_hash: content_hash.clone(),
            created_at,
            primer_id: primer_pair.id.clone(),
            data_strand_count: tagged_strands.len(),
            parity_strand_count: 0,
            codec: "ternary-rotating-cipher".into(),
            redundancy: 1.0,
        };
        self.catalog.register(dna_file);

        // Register in search engine
        self.search.register_file(FileMeta {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            size: data.len(),
            content_hash,
            primer_id: primer_pair.id.clone(),
            strand_count: tagged_strands.len(),
        });

        Ok(WriteResult {
            file_id,
            filename: filename.to_string(),
            data_size: data.len(),
            strand_count: tagged_strands.len(),
            primer_id: primer_pair.id,
        })
    }

    // -----------------------------------------------------------------------
    // dna_read — retrieve a file from DNA
    // -----------------------------------------------------------------------

    /// Read a file back from DNA storage.
    ///
    /// Pipeline: find primer → get strands → untag → decode
    pub fn dna_read(&self, filename: &str) -> Result<Vec<u8>, String> {
        // Look up the file
        let dna_file = self.catalog.get_by_name(filename)
            .ok_or(format!("file '{}' not found", filename))?;

        // Get the primer pair
        let primer_pair = self.primers.get(&dna_file.primer_id)
            .ok_or(format!("primer '{}' not found", dna_file.primer_id))?;

        // Get strands from pool
        let pool_entries = self.pool.get_file_strands(&dna_file.file_id);
        if pool_entries.is_empty() {
            return Err(format!("no strands found for file '{}'", filename));
        }

        // Untag strands (remove primers)
        let data_strands: Vec<_> = pool_entries.iter()
            .filter(|e| !e.is_parity)
            .filter_map(|e| primer_pair.untag_strand(&e.strand))
            .collect();

        if data_strands.is_empty() {
            return Err("failed to untag strands".into());
        }

        // Decode
        let collection = StrandCollection::from_strands(data_strands, dna_file.size);
        let codec = TernaryCodec::new(TernaryConfig::no_overlap());
        let decoded = codec.decode(&collection)
            .map_err(|e| format!("decoding failed: {}", e))?;

        Ok(decoded)
    }

    // -----------------------------------------------------------------------
    // dna_stat — pool and file statistics
    // -----------------------------------------------------------------------

    /// Get pool statistics.
    pub fn dna_stat(&self) -> PoolStatus {
        PoolStatus {
            file_count: self.catalog.len(),
            total_strands: self.pool.total_strands(),
            data_strands: self.pool.total_data_strands(),
            parity_strands: self.pool.total_parity_strands(),
            total_nucleotides: self.pool.total_nucleotides(),
            avg_strand_length: self.pool.avg_strand_length(),
            redundancy: self.pool.redundancy_ratio(),
            files: self.catalog.list().iter().map(|f| FileInfo {
                filename: f.filename.clone(),
                size: f.size,
                strand_count: f.total_strands(),
                codec: f.codec.clone(),
            }).collect(),
        }
    }

    // -----------------------------------------------------------------------
    // dna_delete — remove a file
    // -----------------------------------------------------------------------

    /// Delete a file from DNA storage.
    pub fn dna_delete(&mut self, filename: &str) -> Result<DeleteResult, String> {
        let dna_file = self.catalog.get_by_name(filename)
            .ok_or(format!("file '{}' not found", filename))?;
        let file_id = dna_file.file_id.clone();
        let strand_count = dna_file.total_strands();

        // Remove from pool
        self.pool.remove_file(&file_id);

        // Remove from catalog
        self.catalog.remove(&file_id);

        // Remove from search
        self.search.remove_file(&file_id);

        Ok(DeleteResult {
            filename: filename.to_string(),
            strands_removed: strand_count,
        })
    }

    // -----------------------------------------------------------------------
    // dna_search — search for files
    // -----------------------------------------------------------------------

    /// Search for files matching a query.
    pub fn dna_search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.search.search(query, top_k)
    }
}

// ---------------------------------------------------------------------------
// Result Types
// ---------------------------------------------------------------------------

/// Result of a dna_write operation.
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub file_id: String,
    pub filename: String,
    pub data_size: usize,
    pub strand_count: usize,
    pub primer_id: String,
}

impl fmt::Display for WriteResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Stored '{}' ({} bytes → {} strands, primer={})",
            self.filename, self.data_size, self.strand_count, self.primer_id
        )
    }
}

/// Result of a dna_delete operation.
#[derive(Debug, Clone)]
pub struct DeleteResult {
    pub filename: String,
    pub strands_removed: usize,
}

/// A file summary in pool status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub filename: String,
    pub size: usize,
    pub strand_count: usize,
    pub codec: String,
}

/// Pool status report.
#[derive(Debug, Clone)]
pub struct PoolStatus {
    pub file_count: usize,
    pub total_strands: usize,
    pub data_strands: usize,
    pub parity_strands: usize,
    pub total_nucleotides: usize,
    pub avg_strand_length: f64,
    pub redundancy: f64,
    pub files: Vec<FileInfo>,
}

impl fmt::Display for PoolStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "╔══════════════════════════════════════╗")?;
        writeln!(f, "║         NucleOS Pool Status          ║")?;
        writeln!(f, "╠══════════════════════════════════════╣")?;
        writeln!(f, "║ Files:          {:>6}               ║", self.file_count)?;
        writeln!(f, "║ Total strands:  {:>6}               ║", self.total_strands)?;
        writeln!(f, "║ Data strands:   {:>6}               ║", self.data_strands)?;
        writeln!(f, "║ Parity strands: {:>6}               ║", self.parity_strands)?;
        writeln!(f, "║ Nucleotides:    {:>6}               ║", self.total_nucleotides)?;
        writeln!(f, "║ Avg strand len: {:>6.0} nt            ║", self.avg_strand_length)?;
        writeln!(f, "║ Redundancy:     {:>5.2}×              ║", self.redundancy)?;
        writeln!(f, "╟──────────────────────────────────────╢")?;
        writeln!(f, "║ Files:                               ║")?;
        for fi in &self.files {
            writeln!(f, "║   {} ({} bytes, {} strands)", fi.filename, fi.size, fi.strand_count)?;
        }
        write!(f, "╚══════════════════════════════════════╝")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read_roundtrip() {
        let mut os = NucleOS::new(10);
        let data = b"Hello, NucleOS! This is a test file stored in DNA.";

        let result = os.dna_write("hello.txt", data).unwrap();
        assert!(!result.file_id.is_empty());
        assert!(result.strand_count > 0);

        let recovered = os.dna_read("hello.txt").unwrap();
        assert_eq!(recovered, data.to_vec());
    }

    #[test]
    fn test_write_binary_data() {
        let mut os = NucleOS::new(10);
        let data: Vec<u8> = (0..=255).collect();

        os.dna_write("binary.bin", &data).unwrap();
        let recovered = os.dna_read("binary.bin").unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_multiple_files() {
        let mut os = NucleOS::new(10);

        os.dna_write("file1.txt", b"First file").unwrap();
        os.dna_write("file2.txt", b"Second file").unwrap();
        os.dna_write("file3.txt", b"Third file").unwrap();

        let status = os.dna_stat();
        assert_eq!(status.file_count, 3);

        // Each file should decode correctly
        assert_eq!(os.dna_read("file1.txt").unwrap(), b"First file");
        assert_eq!(os.dna_read("file2.txt").unwrap(), b"Second file");
        assert_eq!(os.dna_read("file3.txt").unwrap(), b"Third file");
    }

    #[test]
    fn test_duplicate_filename_error() {
        let mut os = NucleOS::new(10);
        os.dna_write("test.txt", b"data").unwrap();
        assert!(os.dna_write("test.txt", b"other").is_err());
    }

    #[test]
    fn test_read_nonexistent_error() {
        let os = NucleOS::new(10);
        assert!(os.dna_read("missing.txt").is_err());
    }

    #[test]
    fn test_delete() {
        let mut os = NucleOS::new(10);
        os.dna_write("temp.txt", b"temporary data").unwrap();

        let status = os.dna_stat();
        assert_eq!(status.file_count, 1);

        let del = os.dna_delete("temp.txt").unwrap();
        assert!(del.strands_removed > 0);

        let status = os.dna_stat();
        assert_eq!(status.file_count, 0);
        assert_eq!(status.total_strands, 0);
    }

    #[test]
    fn test_pool_status_display() {
        let mut os = NucleOS::new(10);
        os.dna_write("test.txt", b"Status test").unwrap();

        let status = os.dna_stat();
        let display = format!("{}", status);
        assert!(display.contains("NucleOS"));
        assert!(display.contains("test.txt"));
    }

    #[test]
    fn test_search() {
        let mut os = NucleOS::new(10);
        os.dna_write("readme.txt", b"read me").unwrap();
        os.dna_write("photo.jpg", b"photo data here").unwrap();

        let results = os.dna_search("readme", 5);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_write_result_display() {
        let result = WriteResult {
            file_id: "f1".into(),
            filename: "test.txt".into(),
            data_size: 100,
            strand_count: 5,
            primer_id: "P0000".into(),
        };
        let display = format!("{}", result);
        assert!(display.contains("test.txt"));
        assert!(display.contains("100 bytes"));
    }
}
