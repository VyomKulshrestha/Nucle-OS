//! # nucle_ecc — Error Correction for DNA Storage
//!
//! DNA is a noisy channel with insertion/deletion-heavy error profiles.
//! This crate provides a multi-layer error correction stack:
//!
//! - **Reed-Solomon** — outer code for strand-level erasure recovery
//! - **Fountain/LT codes** — rateless erasure recovery
//! - **Consensus sequencing** — majority voting across strand copies
//! - **Repair pipeline** — orchestrated multi-stage correction

pub mod reed_solomon;
pub mod fountain_code;
pub mod consensus;
pub mod pipeline;
