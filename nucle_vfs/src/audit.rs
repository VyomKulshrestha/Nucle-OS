//! # System-wide audit log
//!
//! An append-only, system-wide "who did what, when" trail — `pool_dir/
//! audit.log`, one JSON line per `dna_write`/`dna_read`/`dna_delete`
//! event, including failed ones. This is a different durability shape
//! from `syscall::NucleOS::persist`'s atomic rewrite of `state.json`:
//! there's no "current state" here to atomically replace, only a
//! sequence of things that happened, so the log is opened in append mode
//! and never rewritten — a crash immediately after a write can at worst
//! leave one trailing malformed line, never corrupt an earlier entry.
//!
//! A migration (`migrate::migrate_object`) doesn't get its own event
//! kind: it's implemented as a `dna_read` + `dna_delete` + `dna_write`
//! against the same filename, and each of those already appends its own
//! event, so a migration shows up as that same three-event trail with no
//! extra bookkeeping needed here.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const AUDIT_LOG_NAME: &str = "audit.log";

/// One recorded event: an operation attempted against a named file,
/// whether it succeeded, and a short human-readable detail (the success
/// summary or the error message).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: u64,
    pub operation: String,
    pub filename: String,
    pub archive_id: Option<String>,
    pub success: bool,
    pub detail: String,
}

impl AuditEvent {
    pub fn new(operation: &str, filename: &str, archive_id: Option<String>, success: bool, detail: String) -> Self {
        Self {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            operation: operation.to_string(),
            filename: filename.to_string(),
            archive_id,
            success,
            detail,
        }
    }
}

/// Appends one event to `pool_dir/audit.log`, creating the pool directory
/// and/or the log itself if either doesn't exist yet.
pub fn append(pool_dir: &Path, event: &AuditEvent) -> Result<(), String> {
    std::fs::create_dir_all(pool_dir)
        .map_err(|e| format!("failed to create pool directory '{}': {}", pool_dir.display(), e))?;

    let line = serde_json::to_string(event)
        .map_err(|e| format!("failed to serialize audit event: {}", e))?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(pool_dir.join(AUDIT_LOG_NAME))
        .map_err(|e| format!("failed to open audit log at '{}': {}", pool_dir.display(), e))?;

    writeln!(file, "{}", line)
        .map_err(|e| format!("failed to append to audit log at '{}': {}", pool_dir.display(), e))
}

/// Reads every event in `pool_dir/audit.log`, oldest first, or an empty
/// list if the log doesn't exist yet (a pool that's never had a mutating
/// operation run against it). A line that fails to parse as JSON is
/// skipped rather than failing the whole read — the one shape of
/// corruption an append-only log can suffer is a truncated trailing line
/// from a crash mid-write, and that shouldn't hide every entry before it.
pub fn read_events(pool_dir: &Path) -> Result<Vec<AuditEvent>, String> {
    let path = pool_dir.join(AUDIT_LOG_NAME);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("failed to open audit log at '{}': {}", path.display(), e))?;

    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| format!("failed to read audit log at '{}': {}", path.display(), e))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AuditEvent>(&line) {
            events.push(event);
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nucle_vfs_audit_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn reading_a_log_that_does_not_exist_yet_returns_empty_not_an_error() {
        let dir = scratch_dir("missing");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read_events(&dir).unwrap().len(), 0);
    }

    #[test]
    fn appended_events_are_read_back_in_order() {
        let dir = scratch_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, &AuditEvent::new("write", "a.txt", Some("archive-1".into()), true, "ok".into())).unwrap();
        append(&dir, &AuditEvent::new("delete", "a.txt", Some("archive-1".into()), true, "removed".into())).unwrap();
        append(&dir, &AuditEvent::new("read", "b.txt", None, false, "file 'b.txt' not found".into())).unwrap();

        let events = read_events(&dir).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].operation, "write");
        assert_eq!(events[1].operation, "delete");
        assert_eq!(events[2].operation, "read");
        assert!(!events[2].success);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_trailing_malformed_line_is_skipped_not_fatal() {
        let dir = scratch_dir("malformed");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, &AuditEvent::new("write", "a.txt", Some("archive-1".into()), true, "ok".into())).unwrap();

        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(AUDIT_LOG_NAME))
            .unwrap();
        writeln!(file, "{{\"timestamp\":1,\"operation\":\"write\"").unwrap(); // truncated JSON

        let events = read_events(&dir).unwrap();
        assert_eq!(events.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
