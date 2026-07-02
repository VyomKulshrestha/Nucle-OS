//! # DNA File Handle
//!
//! The `DnaFile` struct represents a file stored in DNA,
//! carrying all metadata needed for retrieval, verification,
//! and management.

use nucle_ecc::pipeline::RecoveryManifest;
use serde::{Serialize, Deserialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimulationAssumptions {
    pub seed: u64,
    pub coverage_depth: u32,
    pub synthesis_profile: String,
    pub sequencing_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageManifest {
    pub archive_id: String,
    pub codec: String,
    pub profile: String,
    pub redundancy: usize,
    pub primer_set: String,
    pub index_strategy: String,
    pub simulation_assumptions: Option<SimulationAssumptions>,
    pub created_at: i64,
    /// Manifest from the most recent successful `dna_read` of this object,
    /// stored per-object (keyed by this file's archive_id) rather than as
    /// session-global state, so retrieving a different file never overwrites it.
    #[serde(default)]
    pub recovery_manifest: Option<RecoveryManifest>,
}

/// Metadata for a single file stored in DNA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnaFile {
    /// Unique file identifier.
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// Original file size in bytes.
    pub size: usize,
    /// SHA-256 hash of the original content (first 8 bytes).
    pub content_hash: Vec<u8>,
    /// Timestamp when the file was stored (Unix epoch seconds).
    pub created_at: u64,
    /// Primer pair ID used for this file.
    pub primer_id: String,
    /// Number of data strands.
    pub data_strand_count: usize,
    /// Number of parity strands.
    pub parity_strand_count: usize,
    /// The RS parity count each stripe was encoded with (the `redundancy`
    /// argument passed to `dna_write`), distinct from `parity_strand_count`
    /// once large files span multiple GF(256) stripes and accumulate more
    /// total parity strands than any single stripe's parity count. `dna_read`
    /// needs this exact value to reconstruct a matching `RsConfig` for decode.
    #[serde(default)]
    pub rs_parity_per_stripe: usize,
    /// Codec used for encoding.
    pub codec: String,
    /// Redundancy level (e.g., 2.0 = 2× parity).
    pub redundancy: f64,
    /// Storage manifest containing biological assumptions.
    #[serde(default)]
    pub manifest: Option<StorageManifest>,
    /// History of manifests for audit log.
    #[serde(default)]
    pub manifest_history: Vec<StorageManifest>,
}

impl DnaFile {
    /// Total strands (data + parity).
    pub fn total_strands(&self) -> usize {
        self.data_strand_count + self.parity_strand_count
    }

    /// Whether this file has error correction parity.
    pub fn has_ecc(&self) -> bool {
        self.parity_strand_count > 0
    }

    /// Estimated storage efficiency: original bytes per strand.
    pub fn bytes_per_strand(&self) -> f64 {
        if self.data_strand_count == 0 {
            return 0.0;
        }
        self.size as f64 / self.data_strand_count as f64
    }
}

impl fmt::Display for DnaFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} bytes, {} strands, primer={})",
            self.filename, self.size, self.total_strands(), self.primer_id
        )
    }
}

/// A file handle for open operations (future extension).
#[derive(Debug, Clone)]
pub struct FileHandle {
    /// The file this handle refers to.
    pub file: DnaFile,
    /// Current read cursor position.
    pub cursor: usize,
}

impl FileHandle {
    /// Open a file handle.
    pub fn open(file: DnaFile) -> Self {
        Self { file, cursor: 0 }
    }

    /// Reset cursor to beginning.
    pub fn rewind(&mut self) {
        self.cursor = 0;
    }

    /// Seek to a position.
    pub fn seek(&mut self, pos: usize) {
        self.cursor = pos.min(self.file.size);
    }

    /// Remaining bytes from cursor.
    pub fn remaining(&self) -> usize {
        self.file.size.saturating_sub(self.cursor)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file() -> DnaFile {
        DnaFile {
            file_id: "f1".into(),
            filename: "readme.txt".into(),
            size: 1024,
            content_hash: vec![0xAB; 8],
            created_at: 1700000000,
            primer_id: "P0000".into(),
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
    fn test_total_strands() {
        let file = make_file();
        assert_eq!(file.total_strands(), 14);
    }

    #[test]
    fn test_has_ecc() {
        let file = make_file();
        assert!(file.has_ecc());

        let mut no_ecc = make_file();
        no_ecc.parity_strand_count = 0;
        assert!(!no_ecc.has_ecc());
    }

    #[test]
    fn test_bytes_per_strand() {
        let file = make_file();
        assert!((file.bytes_per_strand() - 102.4).abs() < 0.01);
    }

    #[test]
    fn test_display() {
        let file = make_file();
        let s = format!("{}", file);
        assert!(s.contains("readme.txt"));
        assert!(s.contains("1024 bytes"));
    }

    #[test]
    fn test_file_handle() {
        let file = make_file();
        let mut handle = FileHandle::open(file);

        assert_eq!(handle.cursor, 0);
        assert_eq!(handle.remaining(), 1024);

        handle.seek(500);
        assert_eq!(handle.cursor, 500);
        assert_eq!(handle.remaining(), 524);

        handle.rewind();
        assert_eq!(handle.cursor, 0);
    }
}
