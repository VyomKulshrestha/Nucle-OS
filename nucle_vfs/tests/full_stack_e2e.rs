//! # Full Stack End-to-End Integration Tests
//!
//! Tests the complete NucleOS pipeline:
//! Binary data → Codec → ECC → Primers → Pool → CRISPR → ECC → Codec → Binary
//!
//! Also tests the agent interface for natural language → execution.

use nucle_vfs::syscall::NucleOS;
use nucle_agent::executor::Executor;
use nucle_agent::planner::Planner;
use nucle_agent::tools::ToolName;
use nucle_synth::noise::SimulationConfig;
use nucle_synth::profiles::HardwareProfile;

/// Full roundtrip: store text file with ECC and retrieve it.
#[test]
fn test_full_stack_text_roundtrip() {
    let mut os = NucleOS::new(10);

    let original = b"NucleOS: A complete software-defined DNA storage operating system. \
        This test verifies the full pipeline from binary data through encoding, \
        error correction, primer tagging, pool storage, CRISPR retrieval, \
        ECC recovery, decoding, and hash verification.";

    // Store with 4 parity strands
    let write_result = os.dna_write("fullstack.txt", original, 4).unwrap();
    assert!(write_result.data_strand_count > 0);
    assert_eq!(write_result.parity_strand_count, 4);
    assert!(write_result.redundancy > 1.0);

    // Verify pool state
    let status = os.dna_stat();
    assert_eq!(status.file_count, 1);
    assert!(status.parity_strands > 0);
    assert!(status.total_strands > status.data_strands);

    // Retrieve and verify exact match
    let recovered = os.dna_read("fullstack.txt").unwrap();
    assert_eq!(recovered, original.to_vec(), "full stack roundtrip data mismatch");
}

/// Full roundtrip with binary data (all byte values).
#[test]
fn test_full_stack_binary_roundtrip() {
    let mut os = NucleOS::new(10);
    let original: Vec<u8> = (0..=255).collect();

    os.dna_write("binary256.bin", &original, 2).unwrap();
    let recovered = os.dna_read("binary256.bin").unwrap();
    assert_eq!(recovered, original);
}

/// Multi-file isolation: store 3 files, verify each decodes independently.
#[test]
fn test_multi_file_isolation() {
    let mut os = NucleOS::new(10);

    let file1 = b"Alpha file content";
    let file2 = b"Beta file with different data";
    let file3: Vec<u8> = (0..200).collect();

    os.dna_write("alpha.txt", file1, 2).unwrap();
    os.dna_write("beta.txt", file2, 3).unwrap();
    os.dna_write("gamma.bin", &file3, 0).unwrap();

    assert_eq!(os.dna_stat().file_count, 3);

    assert_eq!(os.dna_read("alpha.txt").unwrap(), file1.to_vec());
    assert_eq!(os.dna_read("beta.txt").unwrap(), file2.to_vec());
    assert_eq!(os.dna_read("gamma.bin").unwrap(), file3);
}

/// Delete then re-store: verify slot reuse works.
#[test]
fn test_delete_and_rewrite() {
    let mut os = NucleOS::new(10);

    os.dna_write("temp.txt", b"temporary", 0).unwrap();
    assert_eq!(os.dna_stat().file_count, 1);

    os.dna_delete("temp.txt").unwrap();
    assert_eq!(os.dna_stat().file_count, 0);
    assert_eq!(os.dna_stat().total_strands, 0);

    // Re-store with same name
    os.dna_write("temp.txt", b"new content", 2).unwrap();
    let recovered = os.dna_read("temp.txt").unwrap();
    assert_eq!(recovered, b"new content");
}

/// Search across multiple files.
#[test]
fn test_search_integration() {
    let mut os = NucleOS::new(10);

    os.dna_write("readme.txt", b"readme content", 0).unwrap();
    os.dna_write("photo.jpg", b"jpeg binary data", 0).unwrap();
    os.dna_write("notes.md", b"markdown notes", 0).unwrap();

    let results = os.dna_search("readme", 5);
    assert!(!results.is_empty(), "search should find results for 'readme'");
}

/// Agent executor: natural language store + retrieve.
#[test]
fn test_agent_store_and_status() {
    let mut os = NucleOS::new(10);

    // Pre-store a file
    os.dna_write("readme.txt", b"agent test data", 2).unwrap();

    // Use agent to check status
    let report = Executor::run(&mut os, "pool status").unwrap();
    assert!(report.success);

    // Use agent to list
    let report = Executor::run(&mut os, "list files").unwrap();
    assert!(report.success);
}

/// Agent planner: verify all command types parse correctly.
#[test]
fn test_planner_all_commands() {
    // Store
    let plan = Planner::plan("store backup.dat with 3x redundancy").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::StoreFile);

    // Retrieve
    let plan = Planner::plan("get readme.txt").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::RetrieveFile);

    // Search
    let plan = Planner::plan("find text files").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::SearchFiles);

    // Delete
    let plan = Planner::plan("remove old.log").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::DeleteFile);

    // Status
    let plan = Planner::plan("pool status").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::PoolStatus);

    // List
    let plan = Planner::plan("ls files").unwrap();
    assert_eq!(plan.steps[0].tool, ToolName::ListFiles);
}

/// Stress test: store and retrieve 10 files.
#[test]
fn test_ten_files_stress() {
    let mut os = NucleOS::new(20);

    for i in 0..10 {
        let name = format!("file_{:02}.dat", i);
        let data: Vec<u8> = (0u8..50).map(|b| b.wrapping_add(i as u8)).collect();
        os.dna_write(&name, &data, 1).unwrap();
    }

    assert_eq!(os.dna_stat().file_count, 10);

    // Verify each file
    for i in 0..10 {
        let name = format!("file_{:02}.dat", i);
        let expected: Vec<u8> = (0u8..50).map(|b| b.wrapping_add(i as u8)).collect();
        let recovered = os.dna_read(&name).unwrap();
        assert_eq!(recovered, expected, "file {} data mismatch", name);
    }
}

/// Migration test: store a file, migrate it, check manifest history and new manifest.
#[test]
fn test_migrate_preserves_history() {
    let mut os = NucleOS::new(10);
    let original = b"Migration test data content.";

    // 1. Store
    let _write_result = os.dna_write("migrate_test.txt", original, 2).unwrap();
    let initial_file = os.catalog.get_by_name("migrate_test.txt").unwrap().clone();
    assert!(initial_file.manifest.is_some());
    let initial_manifest = initial_file.manifest.clone().unwrap();

    // 2. Migrate to new redundancy (4)
    let new_manifest = nucle_vfs::migrate::migrate_object(&mut os, "migrate_test.txt", Some(4), None).unwrap();
    assert_eq!(new_manifest.redundancy, 4);

    let updated_file = os.catalog.get_by_name("migrate_test.txt").unwrap();
    assert_eq!(updated_file.manifest_history.len(), 1);
    assert_eq!(updated_file.manifest_history[0].archive_id, initial_manifest.archive_id);
    assert_eq!(updated_file.manifest_history[0].redundancy, 2);

    // Verify roundtrip still works
    let recovered = os.dna_read("migrate_test.txt").unwrap();
    assert_eq!(recovered, original);
}

/// Migrating to an unsupported codec must fail clearly, not silently no-op:
/// NucleOS's storage pipeline implements Ternary and YinYang end-to-end,
/// but not the raw Fountain codec.
#[test]
fn test_migrate_rejects_unsupported_codec() {
    let mut os = NucleOS::new(10);
    os.dna_write("codec_migrate.txt", b"codec migration test", 2).unwrap();

    let result = nucle_vfs::migrate::migrate_object(&mut os, "codec_migrate.txt", None, Some("fountain"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not supported"));

    // Migrating to a genuinely supported codec should work and actually
    // switch the file's stored codec, not just accept the name.
    let result = nucle_vfs::migrate::migrate_object(
        &mut os,
        "codec_migrate.txt",
        None,
        Some("yin-yang"),
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap().codec, "yin-yang");
    assert_eq!(os.dna_read("codec_migrate.txt").unwrap(), b"codec migration test");
}

/// Recovery manifest test: verify that reading a file populates the last recovery manifest correctly.
#[test]
fn test_recovery_manifest_generation() {
    let mut os = NucleOS::new(10);
    let original = b"Recovery manifest generation test.";

    os.dna_write("recovery_test.txt", original, 2).unwrap();

    // Recovery manifest should be None before any read of this object
    let before = os.catalog.get_by_name("recovery_test.txt").unwrap().manifest.clone().unwrap();
    assert!(before.recovery_manifest.is_none());

    // Read file
    let recovered = os.dna_read("recovery_test.txt").unwrap();
    assert_eq!(recovered, original);

    // Recovery manifest should be attached to this object's own manifest after read
    let after = os.catalog.get_by_name("recovery_test.txt").unwrap().manifest.clone().unwrap();
    let manifest = after.recovery_manifest.expect("recovery manifest should be set after read");
    assert_eq!(manifest.consensus_method, "majority-vote");
    assert!(manifest.ecc_success);
}

/// Recovery manifests are per-object: reading a second file must not
/// clobber the first file's manifest (the old session-global design did).
#[test]
fn test_recovery_manifest_is_per_object_not_global() {
    let mut os = NucleOS::new(10);
    os.dna_write("first.txt", b"first file contents", 2).unwrap();
    os.dna_write("second.txt", b"second file, different contents", 2).unwrap();

    os.dna_read("first.txt").unwrap();
    os.dna_read("second.txt").unwrap();

    let first = os.catalog.get_by_name("first.txt").unwrap().manifest.clone().unwrap();
    let second = os.catalog.get_by_name("second.txt").unwrap().manifest.clone().unwrap();
    assert!(first.recovery_manifest.is_some(), "first.txt should keep its own recovery manifest");
    assert!(second.recovery_manifest.is_some(), "second.txt should have its own recovery manifest");
}

/// Archive IDs are content-addressed, so re-reading the same object
/// (without migrating it) must yield the same archive_id every time.
#[test]
fn test_archive_id_stable_across_repeated_reads() {
    let mut os = NucleOS::new(10);
    os.dna_write("stable.txt", b"stable archive id test", 1).unwrap();
    let id_before = os.catalog.get_by_name("stable.txt").unwrap().manifest.clone().unwrap().archive_id;

    os.dna_read("stable.txt").unwrap();
    os.dna_read("stable.txt").unwrap();

    let id_after = os.catalog.get_by_name("stable.txt").unwrap().manifest.clone().unwrap().archive_id;
    assert_eq!(id_before, id_after, "archive_id must not change across repeated reads");
}

/// The core claim of this whole system: redundancy should actually help
/// recover data under realistic (substitution-heavy) sequencing noise, not
/// just when a strand is entirely missing. Before consensus voting was
/// wired into `dna_read`, a strand that survived but was corrupted flowed
/// straight into Reed-Solomon as if it were correct data, and RS -- an
/// erasure decoder, not an error decoder -- had no way to catch that. With
/// coverage_depth copies of each strand consensus-voted before RS ever
/// sees them, most substitution errors are corrected regardless of which
/// copy has them, and the roundtrip succeeds under Illumina noise.
#[test]
fn test_roundtrip_survives_illumina_noise_via_consensus() {
    let noise_cfg = SimulationConfig {
        seed: 7,
        coverage_depth: 10,
        synthesis_profile: HardwareProfile::Illumina,
        sequencing_profile: HardwareProfile::Illumina,
        simulate_decay: false,
        decay_rate: 0.0,
        storage_time: 0.0,
    };
    let mut os = NucleOS::new(10).with_noise(noise_cfg);

    let original = b"Consensus voting across coverage copies corrects \
        substitution errors that Reed-Solomon alone cannot.";

    os.dna_write("noisy.txt", original, 4).unwrap();
    let recovered = os.dna_read("noisy.txt")
        .expect("roundtrip should survive Illumina noise once coverage copies are consensus-voted");
    assert_eq!(recovered, original.to_vec());

    // The recovery manifest should reflect that consensus genuinely ran and
    // that positions actually needed correcting (not a no-op on pristine data).
    let manifest = os.catalog.get_by_name("noisy.txt").unwrap()
        .manifest.clone().unwrap()
        .recovery_manifest.expect("recovery manifest should be set after read");
    assert!(manifest.ecc_success);
    assert!(
        manifest.observed_error_distribution.iter().any(|(_, rate)| *rate > 0.0),
        "Illumina noise at 10x coverage should show up as real, non-zero per-position error rates"
    );
}

/// Nanopore recovery needed a chain of real, distinct fixes, and this test
/// documents what's now fixed and what honestly still isn't:
///
/// 1. **Fixed**: `nucle_ecc::consensus::build_consensus` used to align reads
///    by raw position (truncate-to-median-length), which only works when
///    errors are substitutions. See
///    `nucle_ecc::consensus::tests::test_consensus_corrects_frame_shifting_indels`.
/// 2. **Fixed**: `nucle_index::primer::PrimerPair::{matches_forward,
///    untag_strand}` used exact-position primer matching, so a single indel
///    landing inside a primer (routine at Nanopore's ~4%/base indel rate)
///    made CRISPR retrieval drop the strand *before it ever reached
///    consensus* -- the dominant blocker, not the voting algorithm. See
///    `nucle_index::primer::tests::test_untag_tolerates_*`.
/// 3. **Fixed**: pairwise realignment against one arbitrarily-picked noisy
///    reference read had a hard ceiling once a read carried several
///    simultaneous indels at once. `nucle_ecc::consensus` is now genuine
///    partial-order alignment (every read folds into one shared graph with
///    edge-weighted voting) plus multi-round polishing plus fold-order-
///    independence checking. See the `PoaGraph`-related tests in
///    `nucle_ecc::consensus::tests`.
/// 4. **Fixed, and this one wasn't in the consensus algorithm at all**: the
///    ternary codec's own padding (`TernaryCodec::segment_trits`) filled
///    unused strand length with a *constant* trit, and its 4-byte length
///    header has several leading zero bytes for any file under 16MB -- a
///    constant trit run maps, through the rotating cipher, to a short
///    fixed-period base cycle (a run of trit 0 became a literal
///    "TATATATA..." repeat). That self-inflicted tandem repeat, not
///    anything about the noise or the consensus algorithm, was the actual
///    cause of several residual per-strand errors that looked like a
///    fundamental alignment limit -- tandem repeats are famously hard to
///    align under indel noise for reasons that have nothing to do with how
///    good the aligner is, so a codec that gratuitously creates them was
///    making Nanopore recovery harder than the noise itself required. Fixed
///    by whitening every strand's trits with a deterministic,
///    position-addressable pseudo-random stream before the cipher sees
///    them (`TernaryCodec::whiten_segment`), reversed per-strand at decode.
///
/// 5. **Fixed, in a layer that turned out to have its own bugs**:
///    `nucle_ecc::reed_solomon` had two real, previously undiscovered
///    bugs of its own. Parity symbols are arbitrary GF(256) values
///    (0-255), but were packed into DNA via the same 2-bit
///    `Nucleotide::from_bits` used for already-restricted data bytes --
///    any parity byte above 3 was silently dropped, destroying nearly
///    every parity strand. And a parity strand that failed consensus was
///    dropped from its array (`filter_map`) instead of leaving a gap,
///    reindexing every later parity strand onto the wrong evaluation
///    point. Fixed via 4-base byte packing
///    (`DnaStrand::from_packed_bytes`/`unpack_bytes`) and `Option`-per-slot
///    erasures end to end. On top of that, RS itself was upgraded from
///    erasure-only to genuine combined error-and-erasure decoding
///    (Berlekamp-Welch, `ReedSolomon::try_welch_decode`), so it can now
///    blindly correct a strand that comes back wrong-but-present, not
///    just reconstruct one that's missing. See
///    `reed_solomon::tests::test_rs_parity_reindexing_does_not_corrupt_decode`
///    and `test_rs_corrects_silent_error_without_knowing_position`.
///
/// 6. **Still broken, and this is real -- but now precisely located**:
///    even with Reed-Solomon genuinely fixed, this test still fails at
///    realistic settings. Direct ablation (comparing 0 parity strands
///    through 50 on identical noisy input) produces the *exact same*
///    failure at every redundancy level, which proves Reed-Solomon was
///    never in the critical path here: consensus itself does not reliably
///    converge to the correct sequence at Nanopore's real per-base rate
///    (substitution 3% + insertion 2% + deletion 2%), before Reed-Solomon
///    ever gets a chance to help. This is confirmed below to persist even
///    at 50x coverage and 12 parity strands, which rules out "just not
///    enough redundancy" from either direction now. Closing this needs a
///    better consensus/alignment algorithm for extreme indel density, not
///    a bigger parity budget, and remains open.

#[test]
fn test_nanopore_still_fails_at_realistic_indel_density_despite_alignment_fixes() {
    let mut still_failing = 0;
    let trials = 5;
    for seed in 0..trials {
        let noise_cfg = SimulationConfig {
            seed,
            coverage_depth: 50,
            synthesis_profile: HardwareProfile::OxfordNanopore,
            sequencing_profile: HardwareProfile::OxfordNanopore,
            simulate_decay: false,
            decay_rate: 0.0,
            storage_time: 0.0,
        };
        let mut os = NucleOS::new(10).with_noise(noise_cfg);
        let original = b"Consensus voting across coverage copies corrects \
            substitution errors that Reed-Solomon alone cannot.";
        os.dna_write("noisy_nanopore.txt", original, 12).unwrap();

        let recovered = os.dna_read("noisy_nanopore.txt");
        if recovered.is_err() || recovered.unwrap() != original.to_vec() {
            still_failing += 1;
        }
    }
    assert_eq!(
        still_failing, trials,
        "expected Nanopore roundtrip to still fail at realistic per-read indel density \
         even at 50x coverage / 12 parity strands (see doc comment) -- if this starts \
         passing, multi-read consensus has been added and this test (and the docs \
         describing the limitation) should be updated"
    );
}
