//! # DNA File Migration
//!
//! Provides tools to re-encode stored DNA files with different parameters
//! (such as redundancy) while maintaining manifest history for audit.

use crate::syscall::NucleOS;
use crate::file::StorageManifest;

/// The only codec `dna_write`/`dna_read` actually implement end-to-end.
/// `migrate_object`'s `new_codec` parameter is checked against this so a
/// migration request never silently no-ops on an unsupported target.
pub const SUPPORTED_CODEC: &str = "ternary-rotating-cipher";

/// Re-encodes a stored file under new parameters.
///
/// Decodes the file, deletes the old strands/catalog entries, writes it again,
/// and appends the old manifest to the file's manifest history.
///
/// `new_codec`, if given, must name a codec NucleOS's storage pipeline can
/// actually produce ([`SUPPORTED_CODEC`]) — the pipeline hardcodes a single
/// codec end-to-end today, so migrating *to* a different one is rejected
/// with a clear error rather than silently ignored.
pub fn migrate_object(
    os: &mut NucleOS,
    filename: &str,
    new_redundancy: Option<usize>,
    new_codec: Option<&str>,
) -> Result<StorageManifest, String> {
    if let Some(codec) = new_codec {
        if codec != SUPPORTED_CODEC {
            return Err(format!(
                "codec migration to '{}' is not supported: NucleOS's storage \
                 pipeline only implements '{}' end-to-end",
                codec, SUPPORTED_CODEC
            ));
        }
    }

    // 1. Read old file data and metadata
    let old_file = os.catalog.get_by_name(filename)
        .ok_or_else(|| format!("file '{}' not found", filename))?.clone();
    let data = os.dna_read(filename)?;

    // 2. Extract old manifest and history
    let mut history = old_file.manifest_history.clone();
    if let Some(ref m) = old_file.manifest {
        history.push(m.clone());
    }

    // 3. Delete old file
    os.dna_delete(filename)?;

    // 4. Determine new redundancy
    let redundancy_val = new_redundancy.unwrap_or(old_file.parity_strand_count);

    // 5. Write new file
    let _write_result = os.dna_write(filename, &data, redundancy_val)?;

    // 6. Update the new catalog entry with the old history
    let new_manifest = if let Some(new_file) = os.catalog.get_by_name_mut(filename) {
        new_file.manifest_history = history;
        new_file.manifest.clone()
    } else {
        None
    };

    new_manifest.ok_or_else(|| "Failed to retrieve new storage manifest after migration".to_string())
}
