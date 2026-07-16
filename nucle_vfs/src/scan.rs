//! # Proactive integrity scanning
//!
//! Corruption in this project has always been discovered *passively* --
//! only when a `dna_read` actually happens to run into it. This module
//! is what lets an operator ask "is anything already broken?" without
//! waiting for a real retrieve to stumble onto it, and without having to
//! manually retrieve every file one at a time (`nucle scan` does that
//! for the whole pool in one pass).
//!
//! There's no cheaper check available than actually attempting the real
//! decode: no per-strand checksum exists anywhere in this codebase today
//! (only `DnaFile::content_hash`, a whole-file hash) -- so this module
//! defines just the report shape, and reuses `NucleOS::dna_read`'s own
//! decode/consensus/hash-check pipeline (see `NucleOS::dna_scan`) rather
//! than inventing a second, parallel one. That reuse is deliberate, not
//! a shortcut: a scan therefore appends the same audit-log event and
//! updates the same recovery manifest a real retrieve would, for every
//! file scanned -- it just never prints or returns the actual content.

use serde::{Deserialize, Serialize};

/// One file's result from a pool-wide scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileScanResult {
    pub filename: String,
    pub file_id: String,
    /// `DnaFile::total_strands()` -- how many strands this file's own
    /// metadata says it should have, independent of what's actually in
    /// the pool right now.
    pub expected_strands: usize,
    /// How many strands tagged with this file's `file_id` are actually
    /// present in the pool. Less than `expected_strands` means strands
    /// vanished outside the normal `dna_delete` path (e.g. a corrupted
    /// or hand-edited `state.json`) -- informational context alongside
    /// `recoverable`, not a gate on attempting the real decode: ECC/
    /// consensus might still recover the file despite some being gone.
    pub present_strands: usize,
    /// Whether a real decode attempt (the same pipeline `dna_read` uses)
    /// actually succeeded.
    pub recoverable: bool,
    /// A short human-readable detail: the success message, or the
    /// decode failure's own error text.
    pub detail: String,
}

/// The aggregate result of scanning every file in a pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub total_files: usize,
    pub healthy: usize,
    pub corrupted: usize,
    pub results: Vec<FileScanResult>,
}

impl std::fmt::Display for ScanReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Pool scan: {} file(s), {} healthy, {} corrupted", self.total_files, self.healthy, self.corrupted)?;
        for r in &self.results {
            let status = if r.recoverable { "OK" } else { "CORRUPTED" };
            writeln!(
                f,
                "  [{:<9}] {} ({}/{} strands present) -- {}",
                status, r.filename, r.present_strands, r.expected_strands, r.detail
            )?;
        }
        Ok(())
    }
}
