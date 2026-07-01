//! # Hardware Provider Adapters
//!
//! Provides mock and file-export implementations of the physical hardware
//! bridge. Real vendor adapters (Twist, IDT, Illumina, Oxford Nanopore)
//! belong here too, once the request model in `nucle_lang::hardware` has
//! been exercised long enough to be considered stable — see `provider.rs`
//! for the trait they'd implement.

pub mod confirm;
pub mod file_export;
pub mod mock;
pub mod provider;

pub use confirm::{count_effectful, is_effectful, submit_with_confirmation};
pub use file_export::FileExportProvider;
pub use mock::MockProvider;
pub use provider::Provider;

/// Shared test fixtures for building `HardwareRequest`s without depending
/// on a real `.nsl` compile — used across confirm/mock/file_export tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use nucle_lang::hardware::{HardwareRequest, RequestType};
    use nucle_lang::Effect;

    pub fn synthesis_request(file: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Synthesis,
            target: file.to_string(),
            profile: Some("Twist".to_string()),
            confirmation: "hardware".to_string(),
            detail: RequestType::Synthesis { file_name: file.to_string(), profile: "Twist".to_string() },
        }
    }

    pub fn sequencing_request(file: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Sequencing,
            target: file.to_string(),
            profile: Some("Illumina".to_string()),
            confirmation: "hardware".to_string(),
            detail: RequestType::Sequencing { file_name: file.to_string(), profile: "Illumina".to_string() },
        }
    }

    pub fn destructive_request(file: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Destructive,
            target: file.to_string(),
            profile: None,
            confirmation: "physical_key".to_string(),
            detail: RequestType::Destructive { file_name: file.to_string() },
        }
    }

    /// A synthetic Pure-effect request. `collect_hardware_requests` never
    /// actually produces one (it only ever collects effectful operations),
    /// but the type doesn't forbid it, so this is a legitimate way to unit
    /// test that gating logic ignores Pure entries specifically.
    pub fn pure_request(file: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Pure,
            target: file.to_string(),
            profile: None,
            confirmation: String::new(),
            detail: RequestType::Synthesis { file_name: file.to_string(), profile: "n/a".to_string() },
        }
    }
}
