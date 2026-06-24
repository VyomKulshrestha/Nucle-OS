//! # nucle_codec — DNA Encoding/Decoding Engine
//!
//! Converts arbitrary binary data into valid DNA sequences and back,
//! enforcing hard biological constraints:
//!
//! - **GC content**: 40–60% per strand
//! - **Homopolymer runs**: max 3 consecutive identical bases
//! - **Secondary structure**: no palindromic sequences > 6 nt
//!
//! ## Codec Strategies
//!
//! - **Ternary Rotating Cipher** (Goldman et al.) — ~1.58 bits/nt
//! - **DNA Fountain** (Erlich & Zielinski) — ~1.57 bits/nt, rateless
//! - **Yin-Yang** (Ping et al.) — ~2.0 bits/nt, GC-balanced by construction
//!
//! ## Example
//!
//! ```rust
//! use nucle_codec::base::{Nucleotide, DnaStrand};
//!
//! let strand = DnaStrand::from_str("ATCGATCG").unwrap();
//! assert_eq!(strand.len(), 8);
//! ```

pub mod base;
pub mod constraints;
pub mod ternary;
pub mod fountain;
pub mod yinyang;
pub mod benchmark;
