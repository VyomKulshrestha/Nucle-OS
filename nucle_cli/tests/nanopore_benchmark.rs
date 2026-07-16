//! Regression guard for a real, previously-shipped bug: `nucle simulate`/
//! `bench`/`benchmark`/`pipeline` used to pair `--profile nanopore` with
//! itself for *both* synthesis and sequencing, silently doubling Oxford
//! Nanopore's already-high error rate (measured ~13-14% instead of the
//! documented ~7%) -- which made real recovery at realistic redundancy
//! look like an unsolvable consensus/alignment limitation, when it was
//! actually a noise-configuration bug in the benchmark itself. Fixed via
//! `realistic_synth_seq_pair` (pairs Illumina/Nanopore, both
//! sequencing-only technologies in this project's catalog, with
//! realistic Twist synthesis instead of themselves). See
//! `docs/architecture.md` for the full investigation. Spawns the real,
//! compiled `nucle-cli` binary (`CARGO_BIN_EXE_nucle-cli`).

use std::process::Command;

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

#[test]
fn simulate_reports_the_realistic_undoubled_nanopore_error_rate() {
    let output = Command::new(nucle_cli_bin())
        .arg("simulate").arg("docs/examples/sample_a.txt").arg("-p").arg("nanopore")
        .current_dir("..")
        .output()
        .expect("failed to spawn nucle-cli for simulate");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let line = stdout.lines().find(|l| l.contains("Error rate")).expect("expected an Error rate line");
    let percent_str = line.split(':').nth(1).unwrap().trim().trim_end_matches('║').trim().trim_end_matches('%');
    let rate: f64 = percent_str.parse().expect("failed to parse error rate percentage");

    // The correctly-paired rate is close to the documented ~7% (3% sub +
    // 2% ins + 2% del) for a single synthesis+sequencing pass; the old,
    // doubled-profile bug reported roughly double that (~13-14%). A
    // generous upper bound catches the regression without being brittle
    // against ordinary run-to-run noise variance.
    assert!(rate < 10.0, "expected a realistic (undoubled) Nanopore error rate under 10%, got {}% -- \
        if this is back near ~13-14%, the synthesis/sequencing profile pairing regressed", rate);
}

#[test]
fn benchmark_passes_every_standard_fixture_at_realistic_nanopore_settings() {
    let output = Command::new(nucle_cli_bin())
        .arg("benchmark").arg("-p").arg("nanopore").arg("-r").arg("12")
        .current_dir("..")
        .output()
        .expect("failed to spawn nucle-cli for benchmark");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!stdout.contains("FAIL"), "expected every standard fixture to recover at -r 12 under \
        the corrected Nanopore noise pairing -- a FAIL here would mean either the pairing fix \
        regressed or consensus/ECC recovery itself regressed. Full output:\n{}", stdout);
    assert!(stdout.contains("PASS"), "expected at least one PASS in the benchmark output");
}
