//! A mock provider that simulates real hardware latency on a background
//! `std::thread` — proves `submit()` genuinely returns before the job
//! finishes, and that multiple submissions run concurrently rather than one
//! blocking the next. `MockProvider` stays instant/simple; this is a
//! separate type so its ergonomics aren't disturbed.

use crate::provider::{JobHandle, JobStatus, Provider};
use nucle_lang::hardware::HardwareRequest;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A mock hardware provider that simulates a real submission taking
/// wall-clock time, via a background thread instead of executing instantly.
pub struct DelayedMockProvider {
    delay: Duration,
}

impl DelayedMockProvider {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}

struct ThreadedJobHandle {
    state: Arc<Mutex<JobStatus>>,
}

impl JobHandle for ThreadedJobHandle {
    fn status(&self) -> JobStatus {
        self.state.lock().unwrap().clone()
    }
}

impl Provider for DelayedMockProvider {
    fn name(&self) -> &str {
        "mock-delayed"
    }

    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
        let count = batch.len();
        let delay = self.delay;
        let state = Arc::new(Mutex::new(JobStatus::Pending));
        let state_for_thread = Arc::clone(&state);

        std::thread::spawn(move || {
            *state_for_thread.lock().unwrap() = JobStatus::Running;
            // A panic inside the simulated job becomes a clean Failed(...)
            // instead of poisoning the mutex and propagating a panic out of
            // a later wait() call on a different thread.
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                std::thread::sleep(delay);
                format!("Mock provider (delayed) successfully simulated {} hardware requests.", count)
            }));
            let final_status = match outcome {
                Ok(msg) => JobStatus::Complete(msg),
                Err(_) => JobStatus::Failed("mock-delayed provider panicked during simulated execution".to_string()),
            };
            *state_for_thread.lock().unwrap() = final_status;
        });

        Box::new(ThreadedJobHandle { state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::synthesis_request;
    use std::time::Instant;

    #[test]
    fn submit_returns_immediately_even_with_a_real_delay() {
        let provider = DelayedMockProvider::new(Duration::from_millis(200));
        let start = Instant::now();
        let _handle = provider.submit(&[synthesis_request("a.bin")]);
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "submit() should return well before the simulated delay elapses"
        );
    }

    #[test]
    fn multiple_submissions_run_concurrently_not_sequentially() {
        let provider = DelayedMockProvider::new(Duration::from_millis(100));
        let start = Instant::now();

        let h1 = provider.submit(&[synthesis_request("a.bin")]);
        let h2 = provider.submit(&[synthesis_request("b.bin")]);
        let h3 = provider.submit(&[synthesis_request("c.bin")]);

        h1.wait().unwrap();
        h2.wait().unwrap();
        h3.wait().unwrap();

        let elapsed = start.elapsed();
        // Sequential execution would take ~300ms; concurrent execution
        // should stay well under that even with generous CI scheduling slack.
        assert!(
            elapsed < Duration::from_millis(250),
            "expected concurrent execution (~100ms), took {:?}",
            elapsed
        );
    }

    #[test]
    fn status_passes_through_pending_or_running_before_complete() {
        let provider = DelayedMockProvider::new(Duration::from_millis(80));
        let handle = provider.submit(&[synthesis_request("a.bin")]);

        assert!(matches!(handle.status(), JobStatus::Pending | JobStatus::Running));

        let result = handle.wait().unwrap();
        assert!(result.contains('1'));
        assert_eq!(handle.status(), JobStatus::Complete(result));
    }

    #[test]
    fn name_is_mock_delayed() {
        assert_eq!(DelayedMockProvider::new(Duration::from_millis(1)).name(), "mock-delayed");
    }
}
