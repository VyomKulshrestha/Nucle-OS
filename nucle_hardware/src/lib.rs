//! # Hardware Provider Adapters
//!
//! Provides mock and file-export implementations of the physical hardware
//! bridge. Real vendor adapters (Twist, IDT, Illumina, Oxford Nanopore)
//! belong here too, once the request model in `nucle_lang::hardware` has
//! been exercised long enough to be considered stable — see `provider.rs`
//! for the trait they'd implement.

pub mod file_export;
pub mod mock;
pub mod provider;

pub use file_export::FileExportProvider;
pub use mock::MockProvider;
pub use provider::Provider;
