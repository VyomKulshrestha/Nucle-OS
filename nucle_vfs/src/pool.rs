//! # DNA Storage Pool
//!
//! In-memory store of all DNA strands in the storage system.
//! The pool is the physical layer abstraction — all encoded,
//! ECC-protected, primer-tagged strands live here.
//!
//! Provides pool-level statistics, strand management, and
//! serialization for persistence.

use nucle_codec::base::DnaStrand;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Pool Entry
// ---------------------------------------------------------------------------

/// A single strand entry in the pool with its metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolEntry {
    /// The DNA strand data.
    pub strand: DnaStrand,
    /// File ID this strand belongs to.
    pub file_id: String,
    /// Strand index within the file.
    pub strand_index: usize,
    /// Which logical strand (0..data_strand_count for data, 0..parity_count
    /// for parity) this entry is a read of. Equal to `strand_index` for a
    /// plain write; when synthesis noise simulation produced multiple
    /// coverage copies of the same logical strand, every copy shares the
    /// same `source_index` so `dna_read` can regroup and consensus-vote
    /// them before Reed-Solomon ever sees them.
    pub source_index: usize,
    /// Whether this is a parity (ECC) strand.
    pub is_parity: bool,
}

// ---------------------------------------------------------------------------
// DNA Pool
// ---------------------------------------------------------------------------

/// In-memory DNA storage pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnaPool {
    /// All strands in the pool, keyed by a unique strand ID.
    strands: HashMap<u64, PoolEntry>,
    /// Next strand ID to assign.
    next_id: u64,
    /// Index: file_id → list of strand IDs belonging to that file.
    file_index: HashMap<String, Vec<u64>>,
}

impl DnaPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            strands: HashMap::new(),
            next_id: 0,
            file_index: HashMap::new(),
        }
    }

    /// Add a strand to the pool. Returns the assigned strand ID.
    pub fn add_strand(&mut self, entry: PoolEntry) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        self.file_index
            .entry(entry.file_id.clone())
            .or_default()
            .push(id);

        self.strands.insert(id, entry);
        id
    }

    /// Add multiple strands for a file. Returns the assigned strand IDs.
    pub fn add_file_strands(
        &mut self,
        file_id: &str,
        strands: Vec<DnaStrand>,
        parity_strands: Vec<DnaStrand>,
    ) -> Vec<u64> {
        let mut ids = Vec::new();

        for (i, strand) in strands.into_iter().enumerate() {
            let id = self.add_strand(PoolEntry {
                strand,
                file_id: file_id.to_string(),
                strand_index: i,
                source_index: i,
                is_parity: false,
            });
            ids.push(id);
        }

        for (i, strand) in parity_strands.into_iter().enumerate() {
            let id = self.add_strand(PoolEntry {
                strand,
                file_id: file_id.to_string(),
                strand_index: strands_count_placeholder(i, ids.len()),
                source_index: i,
                is_parity: true,
            });
            ids.push(id);
        }

        ids
    }

    /// Get a strand by ID.
    pub fn get_strand(&self, id: u64) -> Option<&PoolEntry> {
        self.strands.get(&id)
    }

    /// Get all strands belonging to a file.
    pub fn get_file_strands(&self, file_id: &str) -> Vec<&PoolEntry> {
        self.file_index
            .get(file_id)
            .map(|ids| ids.iter().filter_map(|id| self.strands.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get only data (non-parity) strands for a file.
    pub fn get_data_strands(&self, file_id: &str) -> Vec<&DnaStrand> {
        self.get_file_strands(file_id)
            .into_iter()
            .filter(|e| !e.is_parity)
            .map(|e| &e.strand)
            .collect()
    }

    /// Get only parity strands for a file.
    pub fn get_parity_strands(&self, file_id: &str) -> Vec<&DnaStrand> {
        self.get_file_strands(file_id)
            .into_iter()
            .filter(|e| e.is_parity)
            .map(|e| &e.strand)
            .collect()
    }

    /// Remove all strands belonging to a file.
    pub fn remove_file(&mut self, file_id: &str) -> usize {
        let ids = match self.file_index.remove(file_id) {
            Some(ids) => ids,
            None => return 0,
        };
        let count = ids.len();
        for id in ids {
            self.strands.remove(&id);
        }
        count
    }

    /// Get all strands in the pool as a flat list (for CRISPR retrieval).
    pub fn all_strands(&self) -> Vec<&DnaStrand> {
        self.strands.values().map(|e| &e.strand).collect()
    }

    /// List all file IDs in the pool.
    pub fn file_ids(&self) -> Vec<&str> {
        self.file_index.keys().map(|s| s.as_str()).collect()
    }

    // -----------------------------------------------------------------------
    // Statistics
    // -----------------------------------------------------------------------

    /// Total number of strands in the pool.
    pub fn total_strands(&self) -> usize {
        self.strands.len()
    }

    /// Number of files in the pool.
    pub fn file_count(&self) -> usize {
        self.file_index.len()
    }

    /// Total nucleotides across all strands.
    pub fn total_nucleotides(&self) -> usize {
        self.strands.values().map(|e| e.strand.len()).sum()
    }

    /// Total data strands (non-parity).
    pub fn total_data_strands(&self) -> usize {
        self.strands.values().filter(|e| !e.is_parity).count()
    }

    /// Total parity strands.
    pub fn total_parity_strands(&self) -> usize {
        self.strands.values().filter(|e| e.is_parity).count()
    }

    /// Redundancy ratio: total strands / data strands.
    pub fn redundancy_ratio(&self) -> f64 {
        let data = self.total_data_strands();
        if data == 0 {
            return 0.0;
        }
        self.total_strands() as f64 / data as f64
    }

    /// Average strand length.
    pub fn avg_strand_length(&self) -> f64 {
        if self.strands.is_empty() {
            return 0.0;
        }
        self.total_nucleotides() as f64 / self.total_strands() as f64
    }

    /// Pool is empty.
    pub fn is_empty(&self) -> bool {
        self.strands.is_empty()
    }

    /// Serialize pool to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize pool from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// Helper to compute parity strand index.
fn strands_count_placeholder(parity_idx: usize, data_count: usize) -> usize {
    data_count + parity_idx
}

impl Default for DnaPool {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DnaPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "┌─ DNA Storage Pool ──────────────────")?;
        writeln!(f, "│ Files:          {}", self.file_count())?;
        writeln!(f, "│ Total strands:  {}", self.total_strands())?;
        writeln!(f, "│ Data strands:   {}", self.total_data_strands())?;
        writeln!(f, "│ Parity strands: {}", self.total_parity_strands())?;
        writeln!(f, "│ Nucleotides:    {}", self.total_nucleotides())?;
        writeln!(f, "│ Avg strand len: {:.0} nt", self.avg_strand_length())?;
        writeln!(f, "│ Redundancy:     {:.2}×", self.redundancy_ratio())?;
        write!(f, "└──────────────────────────────────────")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_add_and_get() {
        let mut pool = DnaPool::new();

        let strand = DnaStrand::from_str("ATCGATCG").unwrap();
        let id = pool.add_strand(PoolEntry {
            strand: strand.clone(),
            file_id: "file1".into(),
            strand_index: 0,
            source_index: 0,
            is_parity: false,
        });

        assert_eq!(pool.total_strands(), 1);
        let entry = pool.get_strand(id).unwrap();
        assert_eq!(entry.strand, strand);
        assert_eq!(entry.file_id, "file1");
    }

    #[test]
    fn test_file_strands() {
        let mut pool = DnaPool::new();

        for i in 0..5 {
            pool.add_strand(PoolEntry {
                strand: DnaStrand::from_str("ATCGATCG").unwrap(),
                file_id: "file1".into(),
                strand_index: i,
                source_index: i,
                is_parity: i >= 3, // Last 2 are parity
            });
        }

        for i in 0..3 {
            pool.add_strand(PoolEntry {
                strand: DnaStrand::from_str("GCTAGCTA").unwrap(),
                file_id: "file2".into(),
                strand_index: i,
                source_index: i,
                is_parity: false,
            });
        }

        assert_eq!(pool.file_count(), 2);
        assert_eq!(pool.get_file_strands("file1").len(), 5);
        assert_eq!(pool.get_data_strands("file1").len(), 3);
        assert_eq!(pool.get_parity_strands("file1").len(), 2);
        assert_eq!(pool.get_file_strands("file2").len(), 3);
    }

    #[test]
    fn test_remove_file() {
        let mut pool = DnaPool::new();

        for i in 0..3 {
            pool.add_strand(PoolEntry {
                strand: DnaStrand::from_str("ATCG").unwrap(),
                file_id: "file1".into(),
                strand_index: i,
                source_index: i,
                is_parity: false,
            });
        }

        assert_eq!(pool.total_strands(), 3);
        let removed = pool.remove_file("file1");
        assert_eq!(removed, 3);
        assert_eq!(pool.total_strands(), 0);
        assert_eq!(pool.file_count(), 0);
    }

    #[test]
    fn test_pool_statistics() {
        let mut pool = DnaPool::new();

        for i in 0..4 {
            pool.add_strand(PoolEntry {
                strand: DnaStrand::from_str("ATCGATCG").unwrap(), // 8 nt
                file_id: "f1".into(),
                strand_index: i,
                source_index: i,
                is_parity: i == 3,
            });
        }

        assert_eq!(pool.total_strands(), 4);
        assert_eq!(pool.total_data_strands(), 3);
        assert_eq!(pool.total_parity_strands(), 1);
        assert_eq!(pool.total_nucleotides(), 32);
        assert!((pool.avg_strand_length() - 8.0).abs() < 0.01);
        assert!((pool.redundancy_ratio() - 4.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_pool_persistence() {
        let mut pool = DnaPool::new();
        pool.add_strand(PoolEntry {
            strand: DnaStrand::from_str("ATCG").unwrap(),
            file_id: "f1".into(),
            strand_index: 0,
            source_index: 0,
            is_parity: false,
        });

        let json = pool.to_json().unwrap();
        let restored = DnaPool::from_json(&json).unwrap();
        assert_eq!(restored.total_strands(), 1);
    }

    #[test]
    fn test_pool_display() {
        let pool = DnaPool::new();
        let display = format!("{}", pool);
        assert!(display.contains("DNA Storage Pool"));
    }

    #[test]
    fn test_all_strands() {
        let mut pool = DnaPool::new();
        pool.add_strand(PoolEntry {
            strand: DnaStrand::from_str("AAAA").unwrap(),
            file_id: "f1".into(),
            strand_index: 0,
            source_index: 0,
            is_parity: false,
        });
        pool.add_strand(PoolEntry {
            strand: DnaStrand::from_str("TTTT").unwrap(),
            file_id: "f2".into(),
            strand_index: 0,
            source_index: 0,
            is_parity: false,
        });

        assert_eq!(pool.all_strands().len(), 2);
    }
}
