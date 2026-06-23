//! # nucle_index — DNA Retrieval & Indexing
//!
//! Solves the hardest unsolved software problem in DNA storage:
//! retrieving a single file from millions of pooled strands without
//! reading everything.
//!
//! ## Components
//!
//! - **Primer addressing** — unique PCR primer pairs per file
//! - **CRISPR random access** — simulated selective amplification
//! - **Vector index** — content-addressable similarity lookup
//! - **Semantic search** — query by content, not just filename

pub mod primer;
pub mod crispr_sim;
pub mod vector_index;
pub mod search;
