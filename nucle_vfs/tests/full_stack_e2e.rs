//! # Full Stack End-to-End Integration Tests
//!
//! Tests the complete NucleOS pipeline:
//! Binary data → Codec → ECC → Primers → Pool → CRISPR → ECC → Codec → Binary
//!
//! Also tests the agent interface for natural language → execution.

use nucle_codec::constraints::ConstraintValidator;
use nucle_vfs::syscall::{CodecKind, NucleOS};
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
    assert_eq!(updated_file.version, 2);
    assert_eq!(updated_file.manifest_history.len(), 1);
    assert_eq!(updated_file.manifest_history[0].archive_id, initial_manifest.archive_id);
    assert_eq!(updated_file.manifest_history[0].redundancy, 2);

    // Migration is a new representation of the same logical file, not a
    // destructive delete-and-recreate. Both content versions remain readable.
    assert_eq!(
        os.dna_history("migrate_test.txt")
            .iter()
            .map(|v| v.version)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(os.dna_read_version("migrate_test.txt", 1).unwrap(), original);
    assert_eq!(os.dna_read_version("migrate_test.txt", 2).unwrap(), original);

    // Verify roundtrip still works
    let recovered = os.dna_read("migrate_test.txt").unwrap();
    assert_eq!(recovered, original);
}

/// Fountain migration is additive in both directions: each representation
/// keeps its own codec metadata and remains independently readable.
#[test]
fn test_migrate_to_and_from_fountain_preserves_codec_history() {
    let mut os = NucleOS::new(10);
    let original = b"codec migration test";
    os.dna_write("codec_migrate.txt", original, 2).unwrap();

    let fountain = nucle_vfs::migrate::migrate_object(
        &mut os,
        "codec_migrate.txt",
        None,
        Some("fountain"),
    ).unwrap();
    assert_eq!(fountain.codec, "dna-fountain");

    let yin_yang = nucle_vfs::migrate::migrate_object(
        &mut os,
        "codec_migrate.txt",
        None,
        Some("yin-yang"),
    ).unwrap();
    assert_eq!(yin_yang.codec, "yin-yang");

    let history = os.dna_history("codec_migrate.txt");
    assert_eq!(
        history.iter().map(|version| version.codec.as_str()).collect::<Vec<_>>(),
        vec!["ternary-rotating-cipher", "dna-fountain", "yin-yang"]
    );
    for version in 1..=3 {
        assert_eq!(os.dna_read_version("codec_migrate.txt", version).unwrap(), original);
    }
}

/// Unsupported migration input must fail before adding a version or changing
/// the current representation.
#[test]
fn test_migrate_rejects_unsupported_codec_without_mutating_history() {
    let mut os = NucleOS::new(10);
    os.dna_write("codec_migrate.txt", b"codec migration test", 2).unwrap();

    let result = nucle_vfs::migrate::migrate_object(
        &mut os,
        "codec_migrate.txt",
        None,
        Some("imaginary"),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not supported"));
    assert_eq!(os.dna_history("codec_migrate.txt").len(), 1);
    assert_eq!(
        os.catalog.get_by_name("codec_migrate.txt").unwrap().codec,
        "ternary-rotating-cipher"
    );
}

#[test]
fn test_fountain_vfs_rejects_empty_input_without_registering_a_file() {
    let mut os = NucleOS::new(10);
    let result = os.dna_write_with_codec(
        "empty.bin",
        b"",
        2,
        nucle_vfs::syscall::CodecKind::Fountain,
    );

    assert!(result.unwrap_err().contains("empty input data"));
    assert!(os.catalog.get_by_name("empty.bin").is_none());
    assert_eq!(os.dna_stat().total_strands, 0);
}

#[test]
fn test_fountain_pool_persists_screened_config_and_recovers_known_erasures() {
    let mut os = NucleOS::new(10);
    let original: Vec<u8> = (0..=255).collect();
    os.dna_write_with_codec("screened.bin", &original, 4, CodecKind::Fountain)
        .unwrap();

    let file = os.catalog.get_by_name("screened.bin").unwrap().clone();
    let config = file
        .manifest
        .as_ref()
        .and_then(|manifest| manifest.fountain_config.as_ref())
        .expect("Fountain parameters must be persisted per representation");
    assert!(config.screen_constraints);
    let persisted_config = config.clone();

    let state_dir = std::env::temp_dir().join(format!(
        "nucle_vfs_fountain_metadata_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&state_dir);
    os.persist(&state_dir).unwrap();
    let reopened = NucleOS::open(&state_dir, 10).unwrap();
    assert_eq!(
        reopened
            .catalog
            .get_by_name("screened.bin")
            .unwrap()
            .manifest
            .as_ref()
            .unwrap()
            .fountain_config
            .as_ref(),
        Some(&persisted_config)
    );
    let _ = std::fs::remove_dir_all(&state_dir);

    let primer = os.primers.get(&file.primer_id).unwrap();
    let validator = ConstraintValidator::new(config.constraint_config.clone());
    for entry in os.pool.get_file_strands(&file.file_id) {
        if entry.is_parity {
            continue;
        }
        let core = primer.untag_strand(&entry.strand).unwrap();
        assert!(
            validator.is_valid(&core),
            "every stored Fountain droplet must pass its persisted screen"
        );
        assert!(
            validator.is_valid(&entry.strand),
            "the primer-tagged physical Fountain strand must also pass the screen"
        );
    }

    let data_ids: Vec<u64> = os
        .pool
        .get_file_strand_ids(&file.file_id)
        .into_iter()
        .filter(|id| !os.pool.get_strand(*id).unwrap().is_parity)
        .collect();
    for index in [0, 5, 11, 20] {
        assert!(os.pool.remove_strand(data_ids[index]));
    }

    assert_eq!(os.dna_read("screened.bin").unwrap(), original);
}

#[test]
fn test_fountain_pool_extension_adds_droplets_as_a_readable_new_version() {
    let mut os = NucleOS::new(10);
    let original = b"rateless Fountain pools can create later droplet batches";
    let first = os
        .dna_write_with_codec("extend.bin", original, 3, CodecKind::Fountain)
        .unwrap();
    let extended = os.dna_extend_fountain("extend.bin", 10).unwrap();

    assert_eq!(extended.version, 2);
    assert_eq!(extended.data_strand_count, first.data_strand_count + 10);
    assert_eq!(os.dna_history("extend.bin").len(), 2);
    assert_eq!(os.dna_read_version("extend.bin", 1).unwrap(), original);
    assert_eq!(os.dna_read_version("extend.bin", 2).unwrap(), original);
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
        coverage_log_sigma: 0.0,
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
/// 6. **Was misdiagnosed as an open algorithmic wall -- it was actually a
///    bug in this test's own noise configuration.** This test used to
///    set *both* `synthesis_profile` and `sequencing_profile` to
///    `OxfordNanopore`. That's not a real pipeline: synthesis and
///    sequencing are different physical processes with different error
///    sources, and `NoiseEngine::simulate` correctly applies both stages
///    in sequence (`nucle_synth::noise::NoiseEngine::simulate`) --
///    correctly, that is, *given* the two profiles it's handed. Handing
///    it the same high-error profile twice silently doubles the
///    effective per-base error rate: measured directly via `nucle
///    simulate --profile nanopore`, the combined rate is **~13.2%**, not
///    the ~7% (3% substitution + 2% insertion + 2% deletion) this doc
///    comment used to cite as "Nanopore's real per-base rate." The
///    correct pairing already existed in this codebase --
///    `SimulationConfig::twist_nanopore()` (high-fidelity Twist synthesis
///    + noisy Nanopore sequencing, the realistic real-world pipeline) --
///      it just wasn't the one this test used. Direct ablation on 0
///      through 50 parity strands at the *doubled* rate did produce the
///      same failure at every level, which is genuinely true and was
///      correctly reasoned about -- it just wasn't testing the rate the
///      doc comment claimed it was.
///
///    With the correct pairing, consensus + Reed-Solomon **does**
///    converge reliably at realistic coverage/redundancy -- verified
///    directly, not assumed: 15x coverage / 6 parity strands recovers
///    exactly on 10/10 seeds tried; 30x/4 and the original 50x/12
///    (the exact scale this "wall" was first diagnosed at) both recover
///    exactly on 5/5. The two tests below capture this: a fast,
///    always-run check at a modest, still-realistic setting, and a
///    `#[ignore]`d thorough one reproducing the original 50x/12 scale
///    that used to be the strongest evidence *for* the wall and is now
///    the strongest evidence against it.
///
/// # Fast, always-run check
///
/// Realistic Nanopore sequencing (paired with
/// realistic Twist synthesis, not Nanopore-for-both -- see item 6 above)
/// recovers exactly, at coverage/redundancy modest enough to run on
/// every `cargo test` (single-digit seconds).
#[test]
fn test_roundtrip_survives_nanopore_noise_via_consensus() {
    let mut successes = 0;
    let trials = 5;
    for seed in 0..trials {
        let noise_cfg = SimulationConfig::twist_nanopore().with_coverage(15).with_seed(seed);
        let mut os = NucleOS::new(10).with_noise(noise_cfg);
        let original = b"Consensus voting across coverage copies corrects \
            substitution errors that Reed-Solomon alone cannot.";
        os.dna_write("noisy_nanopore.txt", original, 6).unwrap();

        let recovered = os.dna_read("noisy_nanopore.txt");
        if recovered.is_ok() && recovered.unwrap() == original.to_vec() {
            successes += 1;
        }
    }
    assert_eq!(
        successes, trials,
        "expected Nanopore roundtrip to recover exactly at realistic sequencing noise \
         (Twist synthesis + Nanopore sequencing, 15x coverage / 6 parity strands) -- if \
         this starts failing, something regressed in consensus/ECC recovery, not the \
         noise configuration this test deliberately no longer double-applies (see doc \
         comment item 6)"
    );
}

/// Same claim as the test above, at the original 50x coverage / 12
/// parity strands / 5 seeds this "wall" was first (mis)diagnosed at --
/// kept as a thorough, `#[ignore]`d check for the same cost reasons as
/// before (each `PoaGraph` fold's alignment DP is O(read_len x
/// graph_size), so total cost scales with coverage x redundancy x
/// trials). Run explicitly with
/// `cargo test -p nucle_vfs -- --ignored test_nanopore_recovery_is_reliable_at_the_original_diagnostic_scale_thorough`
/// to reproduce the exact scale this limitation was first reported at,
/// now under the corrected noise pairing.
#[test]
#[ignore]
fn test_nanopore_recovery_is_reliable_at_the_original_diagnostic_scale_thorough() {
    let mut successes = 0;
    let trials = 5;
    for seed in 0..trials {
        let noise_cfg = SimulationConfig::twist_nanopore().with_coverage(50).with_seed(seed);
        let mut os = NucleOS::new(10).with_noise(noise_cfg);
        let original = b"Consensus voting across coverage copies corrects \
            substitution errors that Reed-Solomon alone cannot.";
        os.dna_write("noisy_nanopore.txt", original, 12).unwrap();

        let recovered = os.dna_read("noisy_nanopore.txt");
        if recovered.is_ok() && recovered.unwrap() == original.to_vec() {
            successes += 1;
        }
    }
    assert_eq!(
        successes, trials,
        "expected Nanopore roundtrip to recover exactly at 50x coverage / 12 parity \
         strands under the corrected noise pairing -- if this starts failing, something \
         regressed in consensus/ECC recovery at scale"
    );
}
