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
//! - **Metadata similarity search** — ranks files by structural
//!   resemblance (filename, size, type, content hash), not full-text
//!   meaning — see `vector_index`'s own doc comment for why "semantic"
//!   would oversell what this actually does

pub mod primer;
pub mod crispr_sim;
pub mod vector_index;
pub mod search;
