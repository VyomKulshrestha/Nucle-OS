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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{destructive_request, sequencing_request, synthesis_request};

    /// Unique-per-test scratch path so parallel test threads never collide.
    fn scratch_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nucle_hardware_test_{}_{}.json", name, std::process::id()))
    }

    #[test]
    fn file_export_name_is_file_export() {
        assert_eq!(FileExportProvider::new(scratch_path("name")).name(), "file-export");
    }

    #[test]
    fn file_export_writes_valid_json_matching_batch() {
        let path = scratch_path("roundtrip");
        let provider = FileExportProvider::new(path.clone());
        let batch = vec![synthesis_request("a.bin"), destructive_request("b.bin")];

        provider.execute_batch(&batch).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let restored: Vec<HardwareRequest> = serde_json::from_str(&written).unwrap();
        assert_eq!(restored.len(), batch.len());
        assert_eq!(restored, batch);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_export_batch_serialization_preserves_request_fields() {
        let path = scratch_path("fields");
        let provider = FileExportProvider::new(path.clone());
        let batch = vec![sequencing_request("scan.bin")];

        provider.execute_batch(&batch).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let restored: Vec<HardwareRequest> = serde_json::from_str(&written).unwrap();
        assert_eq!(restored[0].target, "scan.bin");
        assert_eq!(restored[0].profile.as_deref(), Some("Illumina"));
        assert_eq!(restored[0].confirmation, "hardware");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_export_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!("nucle_hardware_test_nested_{}", std::process::id()));
        let path = dir.join("batch.json");
        let _ = std::fs::remove_dir_all(&dir); // in case a previous run left it behind

        let provider = FileExportProvider::new(path.clone());
        let result = provider.execute_batch(&[synthesis_request("a.bin")]);
        assert!(result.is_ok());
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_export_reports_count_and_path_in_message() {
        let path = scratch_path("message");
        let provider = FileExportProvider::new(path.clone());
        let batch = vec![synthesis_request("a.bin"), synthesis_request("b.bin"), synthesis_request("c.bin")];

        let msg = provider.execute_batch(&batch).unwrap();
        assert!(msg.contains('3'));
        assert!(msg.contains(&path.display().to_string()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_export_empty_batch_writes_empty_json_array() {
        let path = scratch_path("empty");
        let provider = FileExportProvider::new(path.clone());

        provider.execute_batch(&[]).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let restored: Vec<HardwareRequest> = serde_json::from_str(&written).unwrap();
        assert!(restored.is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
