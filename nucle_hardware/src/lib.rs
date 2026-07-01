//! # Hardware Provider Adapters
//!
//! Provides mock and file-export implementations of the physical hardware bridge.

use nucle_lang::hardware::HardwareRequest;
use std::path::PathBuf;

/// Common interface for physical DNA synthesis/sequencing hardware adapters.
pub trait Provider {
    /// Friendly name of the provider.
    fn name(&self) -> &str;

    /// Execute a batch of hardware requests (synthesis/sequencing).
    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String>;
}

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

/// A provider that serializes batches to a local JSON file for laboratory submission.
pub struct FileExportProvider {
    pub export_path: PathBuf,
}

impl FileExportProvider {
    /// Initialize a new file export provider.
    pub fn new(export_path: PathBuf) -> Self {
        Self { export_path }
    }
}

impl Provider for FileExportProvider {
    fn name(&self) -> &str {
        "file-export"
    }

    fn execute_batch(&self, batch: &[HardwareRequest]) -> Result<String, String> {
        let json_str = serde_json::to_string_pretty(batch)
            .map_err(|e| format!("Serialization error: {}", e))?;
        if let Some(parent) = self.export_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directories: {}", e))?;
        }
        std::fs::write(&self.export_path, &json_str)
            .map_err(|e| format!("Failed to write batch file: {}", e))?;
        Ok(format!("Successfully exported batch of {} requests to '{}'", batch.len(), self.export_path.display()))
    }
}
