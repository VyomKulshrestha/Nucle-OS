//! Guards `Result<T, E>` + `?`'s central promise: a program using
//! none of the new syntax must produce byte-identical execution output
//! (modulo two pre-existing, unrelated sources of non-determinism --
//! see `normalize` below) before and after this feature.
//! `fixtures/execution_baseline.txt` was captured by running every
//! `docs/examples/*.nsl` file through `run_source_file` on the commit
//! immediately before this feature landed -- this test re-runs the same
//! files and diffs against that golden fixture, so a change to the
//! shared execution path (not just a bug in the new interpreter) is a
//! build-breaking regression here, not a "should be fine."

use std::path::Path;

/// The exact set of files the baseline fixture covers -- read from its
/// own `=== name.nsl ===` markers rather than globbing `docs/examples/`
/// fresh, so adding a NEW example (e.g. `result_fallback_store.nsl`,
/// which this feature introduces on purpose) never desyncs this test
/// from a fixture that predates it. This test is only about proving
/// *pre-existing* files are unaffected, not about covering every file
/// that happens to exist today.
fn baseline_file_names(baseline: &str) -> Vec<String> {
    baseline
        .lines()
        // Start markers only (`=== name.nsl === OK`/`ERR`) -- excludes
        // the matching `=== END name.nsl ===` line, which has no `OK`/
        // `ERR` suffix to split on and would otherwise parse as a bogus
        // "END name.nsl ===" entry.
        .filter(|line| line.starts_with("=== ") && !line.starts_with("=== END "))
        .filter_map(|line| line.strip_prefix("=== ")?.split(" === ").next())
        .map(|name| name.to_string())
        .collect()
}

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("examples")
}

fn render(path: &Path) -> String {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    match nucle_lang::run_source_file(path) {
        Ok(report) => format!("=== {} === OK\n{}\n=== END {} ===\n", name, report, name),
        Err(e) => format!("=== {} === ERR\n{}\n=== END {} ===\n", name, e, name),
    }
}

/// Removes two sources of non-determinism that predate this feature and
/// are unrelated to it, so this test compares what actually matters
/// (execution steps, diagnostics, aggregate pool stats) rather than
/// flaking on incidental noise:
///   1. File IDs (`archive-<16 hex chars>`) are derived from
///      `SystemTime::now()` in `nucle_vfs::syscall` -- different on every
///      run by design, not a regression target.
///   2. Per-file lines inside a `PoolStatus` listing (`║   name (ID: ...)`)
///      iterate a catalog whose order isn't guaranteed -- sorted here so
///      two runs with the same files-in-the-pool are treated as equal
///      regardless of which order they were inserted/printed in.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut file_lines: Vec<&str> = Vec::new();
    let mut in_file_block = false;

    for line in s.lines() {
        if line.contains("(ID: archive-") {
            in_file_block = true;
            file_lines.push(line);
            continue;
        }
        if in_file_block {
            // First non-file-listing line after a run of file lines --
            // flush the sorted, ID-redacted buffer before continuing.
            file_lines.sort_unstable();
            for fl in &file_lines {
                out.push_str(&redact_id(fl));
                out.push('\n');
            }
            file_lines.clear();
            in_file_block = false;
        }
        out.push_str(&redact_id(line));
        out.push('\n');
    }
    if in_file_block {
        file_lines.sort_unstable();
        for fl in &file_lines {
            out.push_str(&redact_id(fl));
            out.push('\n');
        }
    }
    out
}

fn redact_id(line: &str) -> String {
    let Some(start) = line.find("archive-") else { return line.to_string() };
    let hex_start = start + "archive-".len();
    let hex_end = line[hex_start..]
        .find(|c: char| !c.is_ascii_hexdigit())
        .map(|i| hex_start + i)
        .unwrap_or(line.len());
    format!("{}archive-<redacted>{}", &line[..start], &line[hex_end..])
}

#[test]
fn execution_output_matches_pre_result_baseline() {
    let baseline_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("execution_baseline.txt");
    let expected_raw = std::fs::read_to_string(&baseline_path)
        .unwrap_or_else(|e| panic!("failed to read baseline fixture {}: {}", baseline_path.display(), e));
    let expected = normalize(&expected_raw);

    let dir = examples_dir();
    let names = baseline_file_names(&expected_raw);
    assert!(!names.is_empty(), "expected the baseline fixture to list at least one file");

    let actual: String = names.iter().map(|name| normalize(&render(&dir.join(name)))).collect();

    assert_eq!(
        actual.trim_end(),
        expected.trim_end(),
        "execution output for docs/examples/*.nsl changed -- Result<T,E>/? must not \
         alter behavior for programs that use none of its new syntax. If this diff is an \
         intentional, unrelated change to an existing example, regenerate the fixture; \
         otherwise this is exactly the regression this test exists to catch."
    );
}
