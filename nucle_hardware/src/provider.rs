//! Provider trait: the sole execution-side interface for submitting
//! hardware requests. See `nucle_lang::hardware` for the request types.

use nucle_lang::hardware::HardwareRequest;
use std::time::Duration;

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

    /// Submit a batch of hardware requests, returning immediately with a
    /// handle to poll or wait on rather than blocking until it finishes.
    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle>;

    /// Submit a batch and block until it finishes — the common case when
    /// running several submissions concurrently isn't needed.
    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String> {
        self.submit(batch).wait()
    }
}
