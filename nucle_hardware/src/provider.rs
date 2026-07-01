//! Provider trait: the sole execution-side interface for submitting
//! hardware requests. See `nucle_lang::hardware` for the request types.

use nucle_lang::hardware::HardwareRequest;

/// Common interface for physical DNA synthesis/sequencing hardware adapters.
pub trait Provider {
    /// Friendly name of the provider.
    fn name(&self) -> &str;

    /// Execute a batch of hardware requests (synthesis/sequencing).
    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String>;
}
