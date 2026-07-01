//! # Semantic Search Interface
//!
//! Unified search pipeline that combines:
//! - **Exact match**: filename and metadata lookups
//! - **Vector similarity**: semantic search via embeddings
//! - **Primer resolution**: translates search results to DNA addresses
//!
//! Query flow: query → parse → vector lookup → primer resolution → ranked results

use crate::primer::{PrimerPair, PrimerLibrary};
use crate::vector_index::{VectorIndex, SearchResult as VectorResult, embed_file_metadata, embed_query};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Search Query
// ---------------------------------------------------------------------------

/// A parsed search query with optional filters.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Raw query string.
    pub raw: String,
    /// Extracted filename pattern (if any).
    pub filename_pattern: Option<String>,
    /// Minimum file size filter.
    pub min_size: Option<usize>,
    /// Maximum file size filter.
    pub max_size: Option<usize>,
    /// File type filter (extension).
    pub file_type: Option<String>,
}

impl SearchQuery {
    /// Parse a raw query string into a structured query.
    ///
    /// Supports simple syntax:
    /// - `name:readme` — filter by filename pattern
    /// - `type:txt` — filter by file extension
    /// - `size:>1000` — minimum size filter
    /// - `size:<1000000` — maximum size filter
    /// - Everything else is treated as a semantic search term
    pub fn parse(raw: &str) -> Self {
        let mut query = Self {
            raw: raw.to_string(),
            filename_pattern: None,
            min_size: None,
            max_size: None,
            file_type: None,
        };

        for token in raw.split_whitespace() {
            if let Some(name) = token.strip_prefix("name:") {
                query.filename_pattern = Some(name.to_lowercase());
            } else if let Some(ext) = token.strip_prefix("type:") {
                query.file_type = Some(ext.to_lowercase());
            } else if let Some(size_str) = token.strip_prefix("size:>") {
                if let Ok(size) = size_str.parse::<usize>() {
                    query.min_size = Some(size);
                }
            } else if let Some(size_str) = token.strip_prefix("size:<") {
                if let Ok(size) = size_str.parse::<usize>() {
                    query.max_size = Some(size);
                }
            }
        }

        query
    }

    /// Get the semantic portion of the query (everything that isn't a filter).
    pub fn semantic_terms(&self) -> String {
        self.raw
            .split_whitespace()
            .filter(|t| {
                !t.starts_with("name:")
                    && !t.starts_with("type:")
                    && !t.starts_with("size:")
            })
            .collect::<Vec<&str>>()
            .join(" ")
    }
}

// ---------------------------------------------------------------------------
// File Metadata Registry
// ---------------------------------------------------------------------------

/// Metadata about a stored file, used for filtering and display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// File identifier (unique name).
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// File size in bytes.
    pub size: usize,
    /// Content hash (first 8 bytes of SHA-256 or similar).
    pub content_hash: Vec<u8>,
    /// Assigned primer pair ID.
    pub primer_id: String,
    /// Number of DNA strands used.
    pub strand_count: usize,
}

/// Registry of all stored files and their metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRegistry {
    files: HashMap<String, FileMeta>,
}

impl FileRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Register a file.
    pub fn insert(&mut self, meta: FileMeta) {
        self.files.insert(meta.file_id.clone(), meta);
    }

    /// Get file metadata by ID.
    pub fn get(&self, file_id: &str) -> Option<&FileMeta> {
        self.files.get(file_id)
    }

    /// Remove a file.
    pub fn remove(&mut self, file_id: &str) -> Option<FileMeta> {
        self.files.remove(file_id)
    }

    /// List all files.
    pub fn list(&self) -> Vec<&FileMeta> {
        self.files.values().collect()
    }

    /// Number of registered files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Check if a file matches the query filters.
    fn matches_filters(&self, file_id: &str, query: &SearchQuery) -> bool {
        let meta = match self.files.get(file_id) {
            Some(m) => m,
            None => return false,
        };

        if let Some(ref pattern) = query.filename_pattern {
            if !meta.filename.to_lowercase().contains(pattern) {
                return false;
            }
        }

        if let Some(ref ext) = query.file_type {
            let file_ext = meta.filename.rsplit('.').next().unwrap_or("").to_lowercase();
            if file_ext != *ext {
                return false;
            }
        }

        if let Some(min) = query.min_size {
            if meta.size < min {
                return false;
            }
        }

        if let Some(max) = query.max_size {
            if meta.size > max {
                return false;
            }
        }

        true
    }
}

impl Default for FileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Search Result
// ---------------------------------------------------------------------------

/// A unified search result combining similarity score and file metadata.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    /// File identifier.
    pub file_id: String,
    /// Relevance score (0.0 to 1.0).
    pub score: f64,
    /// File metadata.
    pub meta: Option<FileMeta>,
    /// Primer pair ID for retrieval.
    pub primer_id: Option<String>,
}

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.meta.as_ref()
            .map(|m| m.filename.as_str())
            .unwrap_or(&self.file_id);
        let size = self.meta.as_ref()
            .map(|m| format!("{} bytes", m.size))
            .unwrap_or_default();
        write!(f, "{:.3}  {}  {}", self.score, name, size)
    }
}

// ---------------------------------------------------------------------------
// Search Engine
// ---------------------------------------------------------------------------

/// The unified search engine combining vector search with metadata filtering.
pub struct SearchEngine {
    /// Vector similarity index.
    pub index: VectorIndex,
    /// File metadata registry.
    pub registry: FileRegistry,
    /// Primer library for address resolution.
    pub primers: PrimerLibrary,
}

impl SearchEngine {
    /// Create a new search engine.
    pub fn new(primers: PrimerLibrary) -> Self {
        Self {
            index: VectorIndex::new(),
            registry: FileRegistry::new(),
            primers,
        }
    }

    /// Register a file in the search engine.
    ///
    /// Computes the embedding and stores metadata.
    pub fn register_file(&mut self, meta: FileMeta) {
        // Compute and store embedding
        let embedding = embed_file_metadata(
            &meta.filename,
            meta.size,
            &meta.content_hash,
        );
        self.index.insert(&meta.file_id, embedding);
        self.registry.insert(meta);
    }

    /// Remove a file from the search engine.
    pub fn remove_file(&mut self, file_id: &str) -> Option<FileMeta> {
        self.index.remove(file_id);
        self.registry.remove(file_id)
    }

    /// Search for files matching a query string.
    ///
    /// Combines vector similarity with metadata filtering.
    pub fn search(&self, query_str: &str, top_k: usize) -> Vec<SearchResult> {
        let query = SearchQuery::parse(query_str);

        // Get semantic terms for vector search
        let semantic = query.semantic_terms();
        let use_vector = !semantic.is_empty();

        // Vector search
        let vector_results: Vec<VectorResult> = if use_vector {
            self.index.search_by_query(&semantic, top_k * 2)
        } else {
            // No semantic query — rank all files equally
            self.registry.list().iter().map(|m| VectorResult {
                file_id: m.file_id.clone(),
                score: 1.0,
            }).collect()
        };

        // Apply filters and resolve primers
        let mut results: Vec<SearchResult> = vector_results
            .into_iter()
            .filter(|r| self.registry.matches_filters(&r.file_id, &query))
            .map(|r| {
                let meta = self.registry.get(&r.file_id).cloned();
                let primer_id = meta.as_ref().map(|m| m.primer_id.clone());
                SearchResult {
                    file_id: r.file_id,
                    score: r.score,
                    meta,
                    primer_id,
                }
            })
            .collect();

        results.truncate(top_k);
        results
    }

    /// Resolve a search result to its primer pair for physical retrieval.
    pub fn resolve_primer(&self, result: &SearchResult) -> Option<&PrimerPair> {
        result.primer_id.as_ref()
            .and_then(|id| self.primers.get(id))
    }

    /// Number of indexed files.
    pub fn file_count(&self) -> usize {
        self.registry.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> SearchEngine {
        let primers = PrimerLibrary::generate(5, 20, 42);
        let mut engine = SearchEngine::new(primers);

        engine.register_file(FileMeta {
            file_id: "f1".into(),
            filename: "readme.txt".into(),
            size: 500,
            content_hash: vec![1; 8],
            primer_id: "P0000".into(),
            strand_count: 10,
        });

        engine.register_file(FileMeta {
            file_id: "f2".into(),
            filename: "photo.jpg".into(),
            size: 5_000_000,
            content_hash: vec![2; 8],
            primer_id: "P0001".into(),
            strand_count: 100,
        });

        engine.register_file(FileMeta {
            file_id: "f3".into(),
            filename: "notes.txt".into(),
            size: 200,
            content_hash: vec![3; 8],
            primer_id: "P0002".into(),
            strand_count: 5,
        });

        engine
    }

    #[test]
    fn test_query_parsing() {
        let q = SearchQuery::parse("name:readme type:txt size:>100 hello world");
        assert_eq!(q.filename_pattern, Some("readme".into()));
        assert_eq!(q.file_type, Some("txt".into()));
        assert_eq!(q.min_size, Some(100));
        assert_eq!(q.semantic_terms(), "hello world");
    }

    #[test]
    fn test_search_by_name() {
        let engine = make_engine();
        let results = engine.search("readme", 5);

        assert!(!results.is_empty());
        // readme.txt should rank highly
        let has_readme = results.iter().any(|r| r.file_id == "f1");
        assert!(has_readme, "readme.txt should appear in results");
    }

    #[test]
    fn test_search_with_type_filter() {
        let engine = make_engine();
        let results = engine.search("type:txt", 5);

        // Should only return .txt files
        for r in &results {
            let meta = r.meta.as_ref().unwrap();
            assert!(
                meta.filename.ends_with(".txt"),
                "expected .txt files, got {}",
                meta.filename
            );
        }
    }

    #[test]
    fn test_search_with_size_filter() {
        let engine = make_engine();
        let results = engine.search("size:<1000", 5);

        for r in &results {
            let meta = r.meta.as_ref().unwrap();
            assert!(meta.size < 1000, "expected small files, got size {}", meta.size);
        }
    }

    #[test]
    fn test_primer_resolution() {
        let engine = make_engine();
        let results = engine.search("readme", 1);

        if let Some(result) = results.first() {
            if result.file_id == "f1" {
                let primer = engine.resolve_primer(result);
                assert!(primer.is_some());
                assert_eq!(primer.unwrap().id, "P0000");
            }
        }
    }

    #[test]
    fn test_register_and_remove() {
        let mut engine = make_engine();
        assert_eq!(engine.file_count(), 3);

        engine.remove_file("f2");
        assert_eq!(engine.file_count(), 2);
        assert!(engine.registry.get("f2").is_none());
    }

    #[test]
    fn test_search_result_display() {
        let result = SearchResult {
            file_id: "f1".into(),
            score: 0.95,
            meta: Some(FileMeta {
                file_id: "f1".into(),
                filename: "test.txt".into(),
                size: 1024,
                content_hash: vec![0; 8],
                primer_id: "P0000".into(),
                strand_count: 5,
            }),
            primer_id: Some("P0000".into()),
        };
        let display = format!("{}", result);
        assert!(display.contains("0.950"));
        assert!(display.contains("test.txt"));
    }
}
