//! Provider trait: the sole execution-side interface for submitting
//! hardware requests. See `nucle_lang::hardware` for the request types.

use nucle_lang::hardware::{HardwareRequest, RequestType};
use std::time::Duration;

/// Stable request categories advertised by a hardware provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Synthesis,
    Sequencing,
    Destructive,
    Qc,
    Recovery,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Synthesis => "synthesis",
            Self::Sequencing => "sequencing",
            Self::Destructive => "destructive",
            Self::Qc => "qc",
            Self::Recovery => "recovery",
        }
    }

    fn from_request(request: &RequestType) -> Self {
        match request {
            RequestType::Synthesis { .. } => Self::Synthesis,
            RequestType::Sequencing { .. } => Self::Sequencing,
            RequestType::Destructive { .. } => Self::Destructive,
            RequestType::Qc { .. } => Self::Qc,
            RequestType::Recovery { .. } => Self::Recovery,
        }
    }
}

/// Static capabilities that callers can inspect before submitting work.
///
/// This is an advertised contract, not a claim that a remote instrument is
/// currently online. `profiles` is empty for generic local providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub name: &'static str,
    pub request_kinds: &'static [RequestKind],
    pub profiles: &'static [&'static str],
    pub requires_nonempty: bool,
    pub resumable: bool,
    pub networked: bool,
}

const ALL_REQUEST_KINDS: &[RequestKind] = &[
    RequestKind::Synthesis,
    RequestKind::Sequencing,
    RequestKind::Destructive,
    RequestKind::Qc,
    RequestKind::Recovery,
];
const SYNTHESIS_ONLY: &[RequestKind] = &[RequestKind::Synthesis];
const SEQUENCING_ONLY: &[RequestKind] = &[RequestKind::Sequencing];

pub const FILE_EXPORT_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "file-export",
    request_kinds: ALL_REQUEST_KINDS,
    profiles: &[],
    requires_nonempty: false,
    resumable: false,
    networked: false,
};
pub const MOCK_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "mock",
    request_kinds: ALL_REQUEST_KINDS,
    profiles: &[],
    requires_nonempty: false,
    resumable: false,
    networked: false,
};
pub const DELAYED_MOCK_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "mock-delayed",
    request_kinds: ALL_REQUEST_KINDS,
    profiles: &[],
    requires_nonempty: false,
    resumable: false,
    networked: false,
};
pub const TWIST_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "twist",
    request_kinds: SYNTHESIS_ONLY,
    profiles: &["Twist"],
    requires_nonempty: true,
    resumable: true,
    networked: true,
};
pub const IDT_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "idt",
    request_kinds: SYNTHESIS_ONLY,
    profiles: &["IDT"],
    requires_nonempty: true,
    resumable: true,
    networked: true,
};
pub const ILLUMINA_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "illumina",
    request_kinds: SEQUENCING_ONLY,
    profiles: &["Illumina"],
    requires_nonempty: true,
    resumable: true,
    networked: true,
};
pub const NANOPORE_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    name: "nanopore",
    request_kinds: SEQUENCING_ONLY,
    profiles: &["Nanopore"],
    requires_nonempty: true,
    resumable: true,
    networked: true,
};
pub const VENDOR_CAPABILITIES: &[ProviderCapabilities] = &[
    TWIST_CAPABILITIES,
    IDT_CAPABILITIES,
    ILLUMINA_CAPABILITIES,
    NANOPORE_CAPABILITIES,
];

pub fn vendor_capabilities(name: &str) -> Option<ProviderCapabilities> {
    VENDOR_CAPABILITIES.iter().copied().find(|item| item.name == name)
}

/// Lifecycle state of a batch submitted to a `Provider`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Complete(String),
    Failed(String),
}

/// A handle to a batch submitted for execution — returned immediately by
/// `Provider::submit`, polled or blocked on later. Submitting doesn't imply
/// the job is done; only `status()`/`wait()` reveal that.
pub trait JobHandle: Send {
    /// Current lifecycle state, without blocking.
    fn status(&self) -> JobStatus;

    /// Opaque, non-secret token that a provider can use to reconstruct this
    /// handle in a later process. Immediate/local providers do not have one.
    fn resume_token(&self) -> Option<String> {
        None
    }

    /// Blocks until the job reaches a terminal state.
    fn wait(&self) -> Result<String, String> {
        let mut backoff = Duration::from_millis(1);
        let cap = Duration::from_millis(20);
        loop {
            match self.status() {
                JobStatus::Complete(msg) => return Ok(msg),
                JobStatus::Failed(msg) => return Err(msg),
                JobStatus::Pending | JobStatus::Running => {
                    std::thread::sleep(backoff);
                    backoff = std::cmp::min(backoff * 2, cap);
                }
            }
        }
    }
}

/// A job handle for a batch that's already finished the instant it was
/// submitted — the common case for providers with no real hardware delay.
pub struct ImmediateJobHandle(Result<String, String>);

impl ImmediateJobHandle {
    pub fn new(result: Result<String, String>) -> Self {
        Self(result)
    }
}

impl JobHandle for ImmediateJobHandle {
    fn status(&self) -> JobStatus {
        match &self.0 {
            Ok(msg) => JobStatus::Complete(msg.clone()),
            Err(msg) => JobStatus::Failed(msg.clone()),
        }
    }
}

/// Common interface for physical DNA synthesis/sequencing hardware adapters.
pub trait Provider {
    /// Friendly name of the provider.
    fn name(&self) -> &str;

    /// Request and profile contract advertised by this provider.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Reject an incompatible batch before any network or filesystem side
    /// effect occurs.
    fn validate_batch(&self, batch: &[HardwareRequest]) -> Result<(), String> {
        let capabilities = self.capabilities();
        if capabilities.requires_nonempty && batch.is_empty() {
            return Err(format!(
                "provider '{}' requires at least one compatible hardware request",
                capabilities.name
            ));
        }
        for request in batch {
            let kind = RequestKind::from_request(&request.detail);
            if !capabilities.request_kinds.contains(&kind) {
                return Err(format!(
                    "provider '{}' cannot safely execute request type '{}'; supported request types: {}",
                    capabilities.name,
                    kind.as_str(),
                    capabilities.request_kinds.iter().map(|kind| kind.as_str()).collect::<Vec<_>>().join(", ")
                ));
            }
            if !capabilities.profiles.is_empty() {
                let detail_profile = match &request.detail {
                    RequestType::Synthesis { profile, .. }
                    | RequestType::Sequencing { profile, .. } => Some(profile.as_str()),
                    _ => None,
                };
                if request.profile.as_deref() != detail_profile {
                    return Err(format!(
                        "provider '{}' received inconsistent request profile metadata",
                        capabilities.name
                    ));
                }
                let profile = detail_profile.ok_or_else(|| format!(
                    "provider '{}' requires a hardware profile for request type '{}'",
                    capabilities.name,
                    kind.as_str()
                ))?;
                if !capabilities.profiles.contains(&profile) {
                    return Err(format!(
                        "provider '{}' does not accept profile '{}'; supported profiles: {}",
                        capabilities.name,
                        profile,
                        capabilities.profiles.join(", ")
                    ));
                }
            }
        }
        Ok(())
    }

    /// Submit a batch of hardware requests, returning immediately with a
    /// handle to poll or wait on rather than blocking until it finishes.
    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle>;

    /// Submit a batch and block until it finishes — the common case when
    /// running several submissions concurrently isn't needed.
    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String> {
        self.validate_batch(batch)?;
        self.submit(batch).wait()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::synthesis_request;

    struct CapabilityProvider(ProviderCapabilities);

    impl Provider for CapabilityProvider {
        fn name(&self) -> &str { self.0.name }
        fn capabilities(&self) -> ProviderCapabilities { self.0 }
        fn submit(&self, _batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
            Box::new(ImmediateJobHandle::new(Ok("submitted".to_string())))
        }
    }

    #[test]
    fn vendor_capabilities_are_unique_and_profile_scoped() {
        let names: Vec<_> = VENDOR_CAPABILITIES.iter().map(|item| item.name).collect();
        assert_eq!(names, ["twist", "idt", "illumina", "nanopore"]);
        assert_eq!(vendor_capabilities("idt"), Some(IDT_CAPABILITIES));
        assert_eq!(vendor_capabilities("unknown"), None);
        assert!(VENDOR_CAPABILITIES.iter().all(|item| {
            item.requires_nonempty && item.resumable && item.networked
                && item.profiles.len() == 1 && item.request_kinds.len() == 1
        }));
    }

    #[test]
    fn idt_rejects_twist_profile_before_submit() {
        let provider = CapabilityProvider(IDT_CAPABILITIES);
        let error = provider.validate_batch(&[synthesis_request("archive.bin")]).unwrap_err();
        assert!(error.contains("does not accept profile 'Twist'"), "{error}");
    }

    #[test]
    fn profile_metadata_must_be_consistent() {
        let provider = CapabilityProvider(TWIST_CAPABILITIES);
        let mut request = synthesis_request("archive.bin");
        request.profile = Some("IDT".to_string());
        let error = provider.validate_batch(&[request]).unwrap_err();
        assert!(error.contains("inconsistent request profile metadata"), "{error}");
    }

    #[test]
    fn generic_local_provider_may_accept_an_empty_batch() {
        let provider = CapabilityProvider(MOCK_CAPABILITIES);
        assert!(provider.validate_batch(&[]).is_ok());
    }
}
