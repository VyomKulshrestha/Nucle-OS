//! # nucle_vfs — Virtual File System for DNA Storage
//!
//! Abstracts the entire DNA storage stack behind clean syscall-style
//! interfaces. DNA storage needs a proper ABI — this layer provides it.
//!
//! ## Core Operations
//!
//! - `dna_write()` — encode → ECC → tag → store
//! - `dna_read()` — search → retrieve → decode → return
//! - `dna_stat()` — pool statistics, health metrics
//! - `dna_delete()` — mark strands for removal

pub mod pool;
pub mod file;
pub mod catalog;
pub mod syscall;
pub mod migrate;
