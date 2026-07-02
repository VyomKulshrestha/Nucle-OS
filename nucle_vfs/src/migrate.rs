//! # DNA File Migration
//!
//! Provides tools to re-encode stored DNA files with different parameters
//! (such as redundancy) while maintaining manifest history for audit.

use crate::syscall::{CodecKind, NucleOS};
use crate::file::StorageManifest;

/// Codecs `dna_write`/`dna_read` implement end-to-end today.
/// `migrate_object`'s `new_codec` parameter is checked against these so a
/// migration request never silently no-ops on an unsupported target.
pub const SUPPORTED_CODECS: &[&str] = &["ternary-rotating-cipher", "yin-yang"];

/// Kept for compatibility with callers checking against a single name;
/// prefer [`SUPPORTED_CODECS`] for the full list.
pub const SUPPORTED_CODEC: &str = "ternary-rotating-cipher";

/// Re-encodes a stored file under new parameters.
///
/// Decodes the file, deletes the old strands/catalog entries, writes it again,
/// and appends the old manifest to the file's manifest history.
///
/// `new_codec`, if given, must name a codec NucleOS's storage pipeline can
/// actually produce ([`SUPPORTED_CODECS`]) — migrating to anything else is
/// rejected with a clear error rather than silently ignored. When omitted,
/// the file keeps the codec it was already stored with.
pub fn migrate_object(
    os: &mut NucleOS,
    filename: &str,
    new_redundancy: Option<usize>,
    new_codec: Option<&str>,
) -> Result<StorageManifest, String> {
    // 1. Read old file data and metadata
    let old_file = os.catalog.get_by_name(filename)
        .ok_or_else(|| format!("file '{}' not found", filename))?.clone();

    let codec_kind = match new_codec {
        Some(name) => CodecKind::from_codec_name(name).ok_or_else(|| {
            format!(
                "codec migration to '{}' is not supported: NucleOS's storage \
                 pipeline only implements {:?} end-to-end",
                name, SUPPORTED_CODECS
            )
        })?,
        None => CodecKind::from_codec_name(&old_file.codec)
            .ok_or_else(|| format!("file's stored codec '{}' is not recognized", old_file.codec))?,
    };

    let data = os.dna_read(filename)?;

    // 2. Extract old manifest and history
    let mut history = old_file.manifest_history.clone();
    if let Some(ref m) = old_file.manifest {
        history.push(m.clone());
    }

    // 3. Delete old file
    os.dna_delete(filename)?;

    // 4. Determine new redundancy (per-stripe RS parity count, not the
    // possibly-larger total parity strand count on multi-stripe files)
    let redundancy_val = new_redundancy.unwrap_or(old_file.rs_parity_per_stripe);

    // 5. Write new file
    let _write_result = os.dna_write_with_codec(filename, &data, redundancy_val, codec_kind)?;

    // 6. Update the new catalog entry with the old history
    let new_manifest = if let Some(new_file) = os.catalog.get_by_name_mut(filename) {
        new_file.manifest_history = history;
        new_file.manifest.clone()
    } else {
        None
    };

    new_manifest.ok_or_else(|| "Failed to retrieve new storage manifest after migration".to_string())
}
