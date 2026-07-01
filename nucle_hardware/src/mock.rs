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

    #[test]
    fn mock_provider_reports_batch_size() {
        let provider = MockProvider;
        let msg = provider.execute_batch(&[]).unwrap();
        assert!(msg.contains('0'));
    }
}
