//! # Vector Similarity Index
//!
//! A simple vector embedding store for file metadata, enabling
//! semantic-style search over the DNA storage pool.
//!
//! Each file is represented as a fixed-length embedding vector
//! derived from its metadata (name, size, type, content hash).
//! Queries compute cosine similarity to find the most relevant files.
//!
//! This is a lightweight in-memory index — no external ML dependencies.

use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fmt;

/// Dimensionality of embedding vectors.
pub const EMBED_DIM: usize = 32;

// ---------------------------------------------------------------------------
// Embedding Vector
// ---------------------------------------------------------------------------

/// A fixed-length embedding vector for similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingVec {
    pub values: Vec<f64>,
}

impl EmbeddingVec {
    /// Create a new embedding from a slice.
    pub fn new(values: &[f64]) -> Self {
        Self {
            values: values.to_vec(),
        }
    }

    /// Create a zero vector of the given dimension.
    pub fn zeros(dim: usize) -> Self {
        Self {
            values: vec![0.0; dim],
        }
    }

    /// Dimensionality of this vector.
    pub fn dim(&self) -> usize {
        self.values.len()
    }

    /// L2 norm (magnitude) of the vector.
    pub fn norm(&self) -> f64 {
        self.values.iter().map(|v| v * v).sum::<f64>().sqrt()
    }

    /// Normalize to unit length.
    pub fn normalize(&self) -> Self {
        let n = self.norm();
        if n == 0.0 {
            return self.clone();
        }
        Self {
            values: self.values.iter().map(|v| v / n).collect(),
        }
    }

    /// Cosine similarity between two vectors (-1.0 to 1.0).
    pub fn cosine_similarity(&self, other: &Self) -> f64 {
        let dot: f64 = self.values.iter()
            .zip(other.values.iter())
            .map(|(a, b)| a * b)
            .sum();
        let norm_a = self.norm();
        let norm_b = other.norm();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        dot / (norm_a * norm_b)
    }

    /// Euclidean distance between two vectors.
    pub fn euclidean_distance(&self, other: &Self) -> f64 {
        self.values.iter()
            .zip(other.values.iter())
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f64>()
            .sqrt()
    }
}

// ---------------------------------------------------------------------------
// File Embedding — derive a vector from file metadata
// ---------------------------------------------------------------------------

/// Generate an embedding vector from file metadata.
///
/// Uses a deterministic hash-based approach (no ML model needed).
/// The embedding captures:
/// - Filename character n-grams (positions 0–15)
/// - File size features (positions 16–19)
/// - Content hash features (positions 20–27)
/// - File type features (positions 28–31)
pub fn embed_file_metadata(
    filename: &str,
    file_size: usize,
    content_hash: &[u8],
) -> EmbeddingVec {
    let mut vec = vec![0.0f64; EMBED_DIM];

    // Filename character features (positions 0–15)
    let name_bytes = filename.as_bytes();
    for (i, &b) in name_bytes.iter().enumerate() {
        let pos = i % 16;
        vec[pos] += (b as f64) / 255.0;
    }
    // Normalize by name length
    let name_len = name_bytes.len().max(1) as f64;
    for v in vec[..16].iter_mut() {
        *v /= name_len;
    }

    // File size features (positions 16–19)
    let size_f = file_size as f64;
    vec[16] = (size_f.ln().max(0.0)) / 30.0; // Log scale
    vec[17] = (size_f / 1_000_000.0).min(1.0); // MB scale
    vec[18] = if file_size < 1024 { 1.0 } else { 0.0 }; // Small file indicator
    vec[19] = if file_size > 1_000_000 { 1.0 } else { 0.0 }; // Large file indicator

    // Content hash features (positions 20–27)
    for (i, &b) in content_hash.iter().take(8).enumerate() {
        vec[20 + i] = b as f64 / 255.0;
    }

    // File extension features (positions 28–31)
    let ext = filename.rsplit('.').next().unwrap_or("");
    let ext_hash = simple_hash(ext.as_bytes());
    vec[28] = ((ext_hash & 0xFF) as f64) / 255.0;
    vec[29] = (((ext_hash >> 8) & 0xFF) as f64) / 255.0;
    vec[30] = if matches!(ext, "txt" | "md" | "log" | "csv") { 1.0 } else { 0.0 }; // Text
    vec[31] = if matches!(ext, "jpg" | "png" | "gif" | "bmp") { 1.0 } else { 0.0 }; // Image

    EmbeddingVec::new(&vec).normalize()
}

/// Generate a query embedding from a search string.
///
/// Simulates what an embedding model would produce for a query.
pub fn embed_query(query: &str) -> EmbeddingVec {
    let mut vec = vec![0.0f64; EMBED_DIM];

    // Query character features (same space as filename)
    let query_bytes = query.as_bytes();
    for (i, &b) in query_bytes.iter().enumerate() {
        let pos = i % 16;
        vec[pos] += (b as f64) / 255.0;
    }
    let len = query_bytes.len().max(1) as f64;
    for v in vec[..16].iter_mut() {
        *v /= len;
    }

    // Check for size-related keywords
    let lower = query.to_lowercase();
    if lower.contains("small") || lower.contains("tiny") {
        vec[18] = 1.0;
    }
    if lower.contains("large") || lower.contains("big") {
        vec[19] = 1.0;
    }
    if lower.contains("text") || lower.contains("document") {
        vec[30] = 1.0;
    }
    if lower.contains("image") || lower.contains("photo") {
        vec[31] = 1.0;
    }

    EmbeddingVec::new(&vec).normalize()
}

/// Simple deterministic hash for strings.
fn simple_hash(data: &[u8]) -> u32 {
    let mut hash: u32 = 5381;
    for &b in data {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    hash
}

// ---------------------------------------------------------------------------
// Vector Index
// ---------------------------------------------------------------------------

/// A search result with relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// File identifier.
    pub file_id: String,
    /// Cosine similarity score (0.0 to 1.0 for normalized vectors).
    pub score: f64,
}

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}  {}", self.score, self.file_id)
    }
}

/// In-memory vector similarity index.
///
/// Stores embeddings for all indexed files and supports brute-force
/// nearest neighbor search via cosine similarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndex {
    /// Map from file ID to embedding vector.
    embeddings: HashMap<String, EmbeddingVec>,
}

impl VectorIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            embeddings: HashMap::new(),
        }
    }

    /// Number of indexed files.
    pub fn len(&self) -> usize {
        self.embeddings.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.embeddings.is_empty()
    }

    /// Add a file embedding to the index.
    pub fn insert(&mut self, file_id: &str, embedding: EmbeddingVec) {
        self.embeddings.insert(file_id.to_string(), embedding);
    }

    /// Remove a file from the index.
    pub fn remove(&mut self, file_id: &str) -> bool {
        self.embeddings.remove(file_id).is_some()
    }

    /// Get the embedding for a file.
    pub fn get(&self, file_id: &str) -> Option<&EmbeddingVec> {
        self.embeddings.get(file_id)
    }

    /// Search for the top-k most similar files to a query vector.
    ///
    /// Returns results sorted by descending similarity score.
    pub fn search(&self, query: &EmbeddingVec, top_k: usize) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self.embeddings.iter()
            .map(|(id, emb)| SearchResult {
                file_id: id.clone(),
                score: query.cosine_similarity(emb),
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results.truncate(top_k);
        results
    }

    /// Search by filename query string.
    pub fn search_by_query(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let query_vec = embed_query(query);
        self.search(&query_vec, top_k)
    }

    /// Serialize the index to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize the index from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl Default for VectorIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_vec_basics() {
        let v = EmbeddingVec::new(&[3.0, 4.0]);
        assert_eq!(v.dim(), 2);
        assert!((v.norm() - 5.0).abs() < 1e-10);

        let normalized = v.normalize();
        assert!((normalized.norm() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = EmbeddingVec::new(&[1.0, 0.0]);
        let b = EmbeddingVec::new(&[0.0, 1.0]);
        let c = EmbeddingVec::new(&[1.0, 0.0]);

        // Orthogonal vectors
        assert!((a.cosine_similarity(&b) - 0.0).abs() < 1e-10);
        // Identical vectors
        assert!((a.cosine_similarity(&c) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = EmbeddingVec::new(&[0.0, 0.0]);
        let b = EmbeddingVec::new(&[3.0, 4.0]);
        assert!((a.euclidean_distance(&b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_embed_file_metadata() {
        let emb = embed_file_metadata("test.txt", 1024, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(emb.dim(), EMBED_DIM);
        // Should be normalized
        assert!((emb.norm() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_similar_files_score_higher() {
        let emb1 = embed_file_metadata("readme.txt", 500, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let emb2 = embed_file_metadata("readme.md", 600, &[1, 2, 3, 4, 5, 6, 7, 9]);
        let emb3 = embed_file_metadata("photo.jpg", 5_000_000, &[99, 88, 77, 66, 55, 44, 33, 22]);

        // readme.txt should be more similar to readme.md than to photo.jpg
        let sim_similar = emb1.cosine_similarity(&emb2);
        let sim_different = emb1.cosine_similarity(&emb3);

        assert!(
            sim_similar > sim_different,
            "similar files ({:.3}) should score higher than different ({:.3})",
            sim_similar, sim_different
        );
    }

    #[test]
    fn test_vector_index_search() {
        let mut index = VectorIndex::new();

        index.insert("readme.txt", embed_file_metadata("readme.txt", 500, &[1; 8]));
        index.insert("readme.md", embed_file_metadata("readme.md", 600, &[2; 8]));
        index.insert("photo.jpg", embed_file_metadata("photo.jpg", 5_000_000, &[99; 8]));

        let results = index.search_by_query("readme", 2);
        assert_eq!(results.len(), 2);

        // Top results should be the readme files
        let top_ids: Vec<&str> = results.iter().map(|r| r.file_id.as_str()).collect();
        assert!(
            top_ids.contains(&"readme.txt") || top_ids.contains(&"readme.md"),
            "expected readme files in top results, got {:?}",
            top_ids
        );
    }

    #[test]
    fn test_index_persistence() {
        let mut index = VectorIndex::new();
        index.insert("file1.txt", embed_file_metadata("file1.txt", 100, &[0; 8]));
        index.insert("file2.bin", embed_file_metadata("file2.bin", 200, &[1; 8]));

        let json = index.to_json().unwrap();
        let restored = VectorIndex::from_json(&json).unwrap();

        assert_eq!(restored.len(), 2);
        assert!(restored.get("file1.txt").is_some());
        assert!(restored.get("file2.bin").is_some());
    }

    #[test]
    fn test_index_remove() {
        let mut index = VectorIndex::new();
        index.insert("a.txt", EmbeddingVec::zeros(EMBED_DIM));
        assert_eq!(index.len(), 1);

        assert!(index.remove("a.txt"));
        assert_eq!(index.len(), 0);
        assert!(!index.remove("nonexistent"));
    }

    #[test]
    fn test_zero_vector_similarity() {
        let zero = EmbeddingVec::zeros(4);
        let nonzero = EmbeddingVec::new(&[1.0, 2.0, 3.0, 4.0]);
        // Similarity with zero vector should be 0
        assert_eq!(zero.cosine_similarity(&nonzero), 0.0);
    }
}
