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
//! - Storage decay (Arrhenius strand breakage over time, see [`decay`])
//!
//! [`planning`] turns what a channel does to strands into the Reed-Solomon
//! parity a file needs to come back through it.

pub mod strand;
pub mod errors;
pub mod profiles;
pub mod noise;
pub mod decay;
pub mod cost;
pub mod procedural;
pub mod planning;
