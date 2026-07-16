//! # Hardware Provider Adapters
//!
//! Provides mock, file-export, and real vendor implementations of the
//! physical hardware bridge (see `provider.rs` for the `Provider` trait
//! they all implement). `twist`/`idt`/`illumina` are real HTTP clients
//! against each vendor's public API; `nanopore` is a real gRPC client
//! against Oxford Nanopore's public MinKNOW API (a local instrument
//! protocol, not a cloud REST API like the other three — see its module
//! doc comment). None of the four has live vendor credentials in this
//! project's development environment, so none has been exercised against
//! a real vendor end-to-end — see each module's doc comment for exactly
//! what is and isn't independently confirmed about its wire format.

pub mod confirm;
pub mod delayed_mock;
pub mod file_export;
pub mod mock;
mod http_client;
pub mod idt;
pub mod illumina;
pub mod nanopore;
pub mod provider;
pub mod twist;

pub use confirm::{count_effectful, is_effectful, submit_with_confirmation, submit_with_confirmation_async};
pub use delayed_mock::DelayedMockProvider;
pub use file_export::FileExportProvider;
pub use idt::{IdtConfig, IdtProvider};
pub use illumina::{IlluminaConfig, IlluminaProvider};
pub use mock::MockProvider;
pub use nanopore::{NanoporeAuth, NanoporeConfig, NanoporeProvider};
pub use provider::{ImmediateJobHandle, JobHandle, JobStatus, Provider};
pub use twist::{TwistConfig, TwistProvider};

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

    /// Shaped exactly like what `collect_hardware_requests` really produces
    /// for a `pipeline { ..., verify roundtrip }` stage -- `RequestType::Qc`,
    /// `Effect::Pure`. Unlike `pure_request`, this proves the actual
    /// Qc/Recovery design decision (read-only, no confirmation required),
    /// not just that gating logic ignores `Pure` in the abstract.
    pub fn qc_request(file: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Pure,
            target: file.to_string(),
            profile: None,
            confirmation: String::new(),
            detail: RequestType::Qc { file_name: file.to_string(), checks: vec!["roundtrip".to_string()] },
        }
    }

    /// Shaped exactly like what `collect_hardware_requests` really produces
    /// for a `consensus_vote(...)` call -- `RequestType::Recovery`,
    /// `Effect::Pure`.
    pub fn recovery_request(binding_name: &str) -> HardwareRequest {
        HardwareRequest {
            effect: Effect::Pure,
            target: binding_name.to_string(),
            profile: None,
            confirmation: String::new(),
            detail: RequestType::Recovery {
                binding_name: binding_name.to_string(),
                consensus_method: "majority-vote".to_string(),
            },
        }
    }
}
