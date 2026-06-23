//! # nucle_synth — DNA Synthesis Simulator
//!
//! Models the exact error distributions of real DNA synthesizers
//! and sequencing platforms. This is the "noisy channel" that
//! all higher layers must survive.
//!
//! ## Hardware Profiles
//!
//! - **Illumina** — low error, substitution-dominant
//! - **Oxford Nanopore** — higher error, indel-heavy (especially homopolymers)
//! - **Twist Bioscience** — synthesis errors, deletion-dominant
//! - **Custom** — user-defined error parameters
//!
//! ## Error Types
//!
//! - Substitutions (base → wrong base)
//! - Insertions (extra base inserted)
//! - Deletions (base dropped)
//! - Strand dropout (entire strand lost)
//! - Strand truncation (incomplete synthesis)

pub mod strand;
pub mod errors;
pub mod profiles;
pub mod noise;
