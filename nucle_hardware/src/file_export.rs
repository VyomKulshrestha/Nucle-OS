//! A provider that serializes batches to a local JSON file for laboratory
//! submission — the only provider today that actually persists anything.

use crate::provider::Provider;
use nucle_lang::hardware::HardwareRequest;
use std::path::PathBuf;

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
