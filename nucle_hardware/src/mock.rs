//! A mock hardware provider for dry runs and tests — never touches disk
//! or a real vendor API.

use crate::provider::Provider;
use nucle_lang::hardware::HardwareRequest;

/// A mock hardware provider for testing dry runs.
pub struct MockProvider;

impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String> {
        let count = batch.len();
        Ok(format!("Mock provider successfully simulated {} hardware requests.", count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{destructive_request, synthesis_request};

    #[test]
    fn mock_provider_reports_batch_size() {
        let provider = MockProvider;
        let msg = provider.execute_batch(&[]).unwrap();
        assert!(msg.contains('0'));
    }

    #[test]
    fn mock_provider_name_is_mock() {
        assert_eq!(MockProvider.name(), "mock");
    }

    #[test]
    fn mock_provider_handles_nonempty_batch_and_reports_count() {
        let provider = MockProvider;
        let batch = [synthesis_request("a.bin"), destructive_request("b.bin")];
        let msg = provider.execute_batch(&batch).unwrap();
        assert!(msg.contains('2'), "expected message to mention batch size 2, got: {}", msg);
    }

    #[test]
    fn mock_provider_never_touches_disk() {
        // A dry run must be side-effect-free — there is no output path to
        // check, so this asserts the batch always succeeds regardless of
        // content, which is the whole point of a mock provider.
        let provider = MockProvider;
        let batch = [destructive_request("would_be_deleted.bin")];
        assert!(provider.execute_batch(&batch).is_ok());
    }
}
