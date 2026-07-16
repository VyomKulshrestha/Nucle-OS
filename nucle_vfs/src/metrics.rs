//! # Pool-state metrics, exported in Prometheus text format
//!
//! **What this is scoped to, and why.** NucleOS is a one-shot CLI, not a
//! long-running service -- there's no in-process request traffic to
//! instrument (no latency histograms, no request counters), because
//! `store`/`retrieve`/etc. each run and exit as their own separate
//! process. What *does* exist, and is worth exporting, is the pool's
//! current state: file/strand counts, capacity usage, encryption status,
//! and the audit log's own event tally. `nucle serve` is a small,
//! separate exporter process (like `postgres_exporter`/`redis_exporter`)
//! that re-reads that state fresh on every scrape and renders it as
//! Prometheus text -- it does not turn NucleOS itself into a
//! network-attached storage service. `store`/`retrieve`/etc. still only
//! ever run as direct CLI invocations against the same `pool_dir`; the
//! exporter and the CLI never talk to each other, they just both read
//! (and, for the CLI, write) the same files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Count of audit-log events sharing one `(operation, success)` pair --
/// the natural Prometheus label set for a counter over that log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventCount {
    pub operation: String,
    pub success: bool,
    pub count: usize,
}

/// A snapshot of one pool's current state, ready to render as Prometheus
/// text (`to_prometheus_text`) or serialize as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMetrics {
    pub file_count: usize,
    pub total_strands: usize,
    pub data_strands: usize,
    pub parity_strands: usize,
    pub total_nucleotides: usize,
    pub redundancy_ratio: f64,
    pub encrypted: bool,
    pub max_nucleotides: Option<usize>,
    pub audit_event_counts: Vec<AuditEventCount>,
}

/// Builds a snapshot from `os`'s current in-memory state plus
/// `pool_dir/audit.log` (empty counts if the pool has never had a
/// mutating operation run against it). `audit_event_counts` is sorted by
/// `(operation, success)` so `to_prometheus_text`'s output -- and this
/// module's own tests -- are deterministic, not at the mercy of
/// `HashMap` iteration order.
pub fn collect(os: &crate::syscall::NucleOS, pool_dir: &Path) -> Result<PoolMetrics, String> {
    let status = os.dna_stat();
    let events = crate::audit::read_events(pool_dir)?;

    let mut counts: HashMap<(String, bool), usize> = HashMap::new();
    for e in &events {
        *counts.entry((e.operation.clone(), e.success)).or_insert(0) += 1;
    }
    let mut audit_event_counts: Vec<AuditEventCount> = counts
        .into_iter()
        .map(|((operation, success), count)| AuditEventCount { operation, success, count })
        .collect();
    audit_event_counts.sort_by(|a, b| a.operation.cmp(&b.operation).then(a.success.cmp(&b.success)));

    Ok(PoolMetrics {
        file_count: status.file_count,
        total_strands: status.total_strands,
        data_strands: status.data_strands,
        parity_strands: status.parity_strands,
        total_nucleotides: status.total_nucleotides,
        redundancy_ratio: status.redundancy,
        encrypted: status.encrypted,
        max_nucleotides: os.max_nucleotides(),
        audit_event_counts,
    })
}

impl PoolMetrics {
    /// Renders this snapshot in Prometheus's plain-text exposition
    /// format (one `# HELP`/`# TYPE` pair per metric, then its
    /// value/sample lines) -- hand-rolled rather than pulling in the
    /// `prometheus` crate, since that crate's registry/global-state
    /// machinery is built for instrumenting a long-running request
    /// handler, not rendering one already-computed snapshot; for a
    /// small, fixed metric set, string formatting is both simpler and
    /// one fewer dependency.
    pub fn to_prometheus_text(&self) -> String {
        let mut out = String::new();

        push_gauge(&mut out, "nucleos_pool_files", "Number of files currently stored in the pool.", self.file_count as f64);
        push_gauge(&mut out, "nucleos_pool_strands_total", "Total strands (data + parity) currently stored.", self.total_strands as f64);
        push_gauge(&mut out, "nucleos_pool_data_strands", "Data strands currently stored.", self.data_strands as f64);
        push_gauge(&mut out, "nucleos_pool_parity_strands", "Parity strands currently stored.", self.parity_strands as f64);
        push_gauge(&mut out, "nucleos_pool_nucleotides_total", "Total nucleotides currently stored.", self.total_nucleotides as f64);
        push_gauge(&mut out, "nucleos_pool_redundancy_ratio", "Average redundancy ratio across all stored files.", self.redundancy_ratio);
        push_gauge(&mut out, "nucleos_pool_encrypted", "Whether this pool is encrypted at rest (1) or not (0).", if self.encrypted { 1.0 } else { 0.0 });

        if let Some(max) = self.max_nucleotides {
            push_gauge(&mut out, "nucleos_pool_capacity_max_nucleotides", "Configured capacity limit in nucleotides, if one is set.", max as f64);
        }

        out.push_str("# HELP nucleos_audit_events_total Audit log events, by operation and outcome.\n");
        out.push_str("# TYPE nucleos_audit_events_total counter\n");
        for c in &self.audit_event_counts {
            out.push_str(&format!(
                "nucleos_audit_events_total{{operation=\"{}\",success=\"{}\"}} {}\n",
                c.operation, c.success, c.count
            ));
        }

        out
    }
}

fn push_gauge(out: &mut String, name: &str, help: &str, value: f64) {
    out.push_str(&format!("# HELP {} {}\n", name, help));
    out.push_str(&format!("# TYPE {} gauge\n", name));
    out.push_str(&format!("{} {}\n", name, value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall::NucleOS;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nucle_vfs_metrics_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn an_empty_pool_collects_zeroed_metrics_with_no_audit_events() {
        let dir = scratch_dir("empty");
        let _ = std::fs::remove_dir_all(&dir);
        let os = NucleOS::open(&dir, 10).unwrap();

        let metrics = collect(&os, &dir).unwrap();
        assert_eq!(metrics.file_count, 0);
        assert_eq!(metrics.total_strands, 0);
        assert!(!metrics.encrypted);
        assert_eq!(metrics.max_nucleotides, None);
        assert!(metrics.audit_event_counts.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_file_is_reflected_in_the_snapshot_and_audit_counts() {
        let dir = scratch_dir("stored");
        let _ = std::fs::remove_dir_all(&dir);
        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("a.txt", b"hello metrics", 1).unwrap();

        let metrics = collect(&os, &dir).unwrap();
        assert_eq!(metrics.file_count, 1);
        assert_eq!(metrics.total_strands, 2); // 1 data + 1 parity at redundancy 1
        assert_eq!(metrics.audit_event_counts.len(), 1);
        assert_eq!(metrics.audit_event_counts[0].operation, "write");
        assert!(metrics.audit_event_counts[0].success);
        assert_eq!(metrics.audit_event_counts[0].count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_operation_is_counted_separately_from_successes() {
        let dir = scratch_dir("mixed_outcomes");
        let _ = std::fs::remove_dir_all(&dir);
        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("a.txt", b"data", 0).unwrap();
        assert!(os.dna_read("does_not_exist.txt").is_err());

        let metrics = collect(&os, &dir).unwrap();
        let write_ok = metrics.audit_event_counts.iter().find(|c| c.operation == "write" && c.success).unwrap();
        assert_eq!(write_ok.count, 1);
        let read_failed = metrics.audit_event_counts.iter().find(|c| c.operation == "read" && !c.success).unwrap();
        assert_eq!(read_failed.count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prometheus_text_includes_every_metric_with_help_and_type_lines() {
        let dir = scratch_dir("prom_text");
        let _ = std::fs::remove_dir_all(&dir);
        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("a.txt", b"data", 1).unwrap();
        os.set_max_nucleotides(Some(1_000_000));

        let text = collect(&os, &dir).unwrap().to_prometheus_text();
        for name in [
            "nucleos_pool_files",
            "nucleos_pool_strands_total",
            "nucleos_pool_data_strands",
            "nucleos_pool_parity_strands",
            "nucleos_pool_nucleotides_total",
            "nucleos_pool_redundancy_ratio",
            "nucleos_pool_encrypted",
            "nucleos_pool_capacity_max_nucleotides",
            "nucleos_audit_events_total",
        ] {
            assert!(text.contains(&format!("# HELP {}", name)), "missing HELP for {}", name);
            assert!(text.contains(&format!("# TYPE {}", name)), "missing TYPE for {}", name);
        }
        assert!(text.contains("nucleos_pool_files 1"));
        assert!(text.contains("nucleos_audit_events_total{operation=\"write\",success=\"true\"} 1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capacity_metric_is_omitted_entirely_when_unset() {
        let dir = scratch_dir("no_capacity");
        let _ = std::fs::remove_dir_all(&dir);
        let os = NucleOS::open(&dir, 10).unwrap();

        let text = collect(&os, &dir).unwrap().to_prometheus_text();
        assert!(!text.contains("nucleos_pool_capacity_max_nucleotides"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
