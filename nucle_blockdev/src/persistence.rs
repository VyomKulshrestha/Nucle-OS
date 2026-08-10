//! Shared atomic JSON persistence helpers for simulated block devices.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create device directory '{}': {e}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("failed to serialize block-device image: {e}"))?;
    let tmp = path.with_extension(format!(
        "{}tmp",
        path.extension().and_then(|ext| ext.to_str()).map(|ext| format!("{ext}.")).unwrap_or_default()
    ));
    std::fs::write(&tmp, bytes)
        .map_err(|e| format!("failed to write temporary device image '{}': {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("failed to finalize device image '{}': {e}", path.display()))
}

pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read device image '{}': {e}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("failed to parse device image '{}': {e}", path.display()))
}
