//! # Syscall-Style API — `dna_write()`, `dna_read()`, `dna_stat()`
//!
//! The top-level API that ties **every** layer together:
//!
//! ```text
//! dna_write(name, data, redundancy)
//!   → Codec encode → RS ECC parity → Primer tag → Pool store
//!
//! dna_read(name)
//!   → Catalog lookup → CRISPR retrieve → Primer untag
//!   → RS ECC decode → Codec decode → data
//!
//! dna_stat()  → Pool stats, file listing, health metrics
//! dna_delete() → Remove from Pool + Catalog + Search
//! ```
//!
//! Full stack: VFS → Index → ECC → Codec → Synth

use crate::pool::{DnaPool, PoolEntry};
use crate::file::{DnaFile, StorageManifest, SimulationAssumptions};
use crate::catalog::Catalog;
use nucle_codec::base::{DnaCodec, DnaStrand, StrandCollection};
use nucle_codec::ternary::{TernaryCodec, TernaryConfig};
use nucle_codec::yinyang::{YinYangCodec, YinYangConfig};
use nucle_ecc::reed_solomon::{ReedSolomon, RsConfig};
use nucle_ecc::pipeline::{compute_error_distribution, consensus_then_rs_decode_with_retry, RecoveryManifest};
use nucle_index::primer::PrimerLibrary;
use nucle_index::crispr_sim::{CrisprSimulator, CrisprConfig};
use nucle_index::search::{SearchEngine, FileMeta, SearchResult};
use nucle_synth::noise::{NoiseEngine, SimulationConfig};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// CodecKind — which binary <-> DNA codec a write/read pass uses
// ---------------------------------------------------------------------------

/// Selects which [`DnaCodec`] implementation `dna_write_with_codec` uses.
///
/// `dna_read` doesn't take one of these: it recovers the codec a file was
/// written with from the name stored on its [`DnaFile`] record, so encoding
/// choice is a write-time decision only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    Ternary,
    YinYang,
}

impl CodecKind {
    fn to_boxed(self) -> Box<dyn DnaCodec> {
        match self {
            Self::Ternary => Box::new(TernaryCodec::new(TernaryConfig::no_overlap())),
            Self::YinYang => Box::new(YinYangCodec::new(YinYangConfig::default())),
        }
    }

    /// Reverse-lookup from the name a codec reports via [`DnaCodec::name`],
    /// as persisted on a [`DnaFile`]'s `codec` field.
    pub fn from_codec_name(name: &str) -> Option<Self> {
        match name {
            "ternary-rotating-cipher" => Some(Self::Ternary),
            "yin-yang" => Some(Self::YinYang),
            _ => None,
        }
    }
}

/// Optionally run strands through synthesis/sequencing noise, returning
/// each surviving strand alongside the logical (pre-coverage-expansion)
/// index of the strand it's a copy of.
///
/// When `simulate` is false, this is the identity mapping (`source_index ==
/// position`). When true, `NoiseEngine::simulate` produces `coverage_depth`
/// independent noisy copies per input strand in copy-major order with a
/// flat, monotonically increasing `strand_id`; dividing that id by
/// `coverage_depth` recovers which original strand a given copy belongs to.
/// Copies that were dropped entirely (`!is_intact`) carry no information and
/// are not stored.
fn simulate_with_provenance(
    simulate: bool,
    noise_config: &SimulationConfig,
    strands: Vec<DnaStrand>,
    original_size: usize,
) -> (Vec<DnaStrand>, Vec<usize>) {
    if !simulate {
        let sources = (0..strands.len()).collect();
        return (strands, sources);
    }

    let collection = StrandCollection::from_strands(strands, original_size);
    let engine = NoiseEngine::new(noise_config.clone());
    let result = engine.simulate(&collection);
    let coverage = noise_config.coverage_depth.max(1) as u64;

    result.pool.strands.iter()
        .filter(|s| s.is_intact && !s.is_truncated)
        .map(|s| (s.sequence.clone(), (s.strand_id / coverage) as usize))
        .unzip()
}

// ---------------------------------------------------------------------------
// Durable persistence — see NucleOS::open/persist
// ---------------------------------------------------------------------------

/// Filename a pool's durable state is written under, inside its pool
/// directory (see [`NucleOS::open`]/[`NucleOS::persist`]).
pub const STATE_FILE_NAME: &str = "state.json";

/// Advisory lock file `persist()` holds for its check-then-write sequence.
/// The optimistic version check alone has a real TOCTOU race: two
/// processes can both observe "no conflict" before either one's write
/// reaches disk, and the second `rename()` would then silently clobber the
/// first's already-persisted state (confirmed empirically -- a real,
/// if rare, race in this project's own concurrency test before this lock
/// was added). The lock doesn't replace the version check; it just makes
/// check-then-write atomic with respect to any other process's persist().
const LOCK_FILE_NAME: &str = ".lock";

/// The only state that can't be deterministically reconstructed at
/// `open()` time -- see `NucleOS::open`'s doc comment for why `primers`/
/// `search`/`crispr`/noise settings aren't part of this.
///
/// `version` is an optimistic-concurrency counter, incremented on every
/// successful `persist()` -- see `NucleOS::persist`'s doc comment for what
/// it protects against.
///
/// `max_nucleotides` is a pool-level setting (see `NucleOS::
/// set_max_nucleotides`), not a per-invocation flag, so it has to persist
/// alongside the data it limits -- otherwise a `--max-pool-size` given on
/// one `nucle store` call would silently stop applying the moment a later,
/// unrelated command opened the same pool without repeating it.
#[derive(Serialize, Deserialize)]
struct PersistedState {
    #[serde(default)]
    version: u64,
    pool: DnaPool,
    catalog: Catalog,
    primers_used: usize,
    #[serde(default)]
    max_nucleotides: Option<usize>,
}

// ---------------------------------------------------------------------------
// NucleOS — the unified DNA storage OS
// ---------------------------------------------------------------------------

/// The main DNA storage operating system.
///
/// Integrates all 6 layers:
/// 1. **Codec** — binary ↔ DNA encoding (ternary rotating cipher)
/// 2. **Synth** — synthesis/sequencing noise simulation
/// 3. **ECC** — Reed-Solomon strand-level erasure recovery
/// 4. **Index** — primer addressing + CRISPR retrieval + vector search
/// 5. **VFS** — pool, catalog, file metadata
/// 6. **Agent** — natural-language operation planning
pub struct NucleOS {
    /// DNA strand storage pool.
    pub pool: DnaPool,
    /// File metadata catalog.
    pub catalog: Catalog,
    /// Primer library for file addressing.
    pub primers: PrimerLibrary,
    /// Search engine for metadata-similarity file lookup.
    pub search: SearchEngine,
    /// CRISPR simulator for selective retrieval.
    crispr: CrisprSimulator,
    /// Whether to simulate synthesis/sequencing noise on write.
    pub simulate_noise: bool,
    /// Noise simulation config (used when simulate_noise is true).
    pub noise_config: SimulationConfig,
    /// Number of primer pairs used so far.
    primers_used: usize,
    /// The `PersistedState::version` this instance last loaded or wrote --
    /// 0 for an instance that's never touched a pool directory. Used by
    /// `persist()`'s optimistic-concurrency check; irrelevant otherwise.
    loaded_version: u64,
    /// Maximum total nucleotides this pool may hold, or `None` for
    /// unlimited (today's behavior, unchanged unless explicitly set via
    /// `set_max_nucleotides`). Checked in `dna_write_with_codec`.
    max_nucleotides: Option<usize>,
    /// Where this instance's `audit.log` lives, or `None` for an ephemeral
    /// in-memory instance (`NucleOS::new`) that was never opened against a
    /// real pool directory — such instances (benchmarks, `nucle doctor`'s
    /// roundtrip probe) don't have anywhere durable to log to, and aren't
    /// real user activity worth an audit trail anyway. Set by `open()`.
    pool_dir: Option<std::path::PathBuf>,
}

impl NucleOS {
    /// Initialize a new NucleOS instance.
    ///
    /// `max_files`: maximum number of files (determines primer library size).
    pub fn new(max_files: usize) -> Self {
        let primers = PrimerLibrary::generate(max_files.max(10), 20, 42);
        let search = SearchEngine::new(primers.clone());
        Self {
            pool: DnaPool::new(),
            catalog: Catalog::new(),
            primers,
            search,
            crispr: CrisprSimulator::new(CrisprConfig::ideal()),
            simulate_noise: false,
            noise_config: SimulationConfig::pristine(),
            primers_used: 0,
            loaded_version: 0,
            max_nucleotides: None,
            pool_dir: None,
        }
    }

    /// Create with default capacity (100 files).
    pub fn default_os() -> Self {
        Self::new(100)
    }

    /// Enable noise simulation with a given config.
    pub fn with_noise(mut self, config: SimulationConfig) -> Self {
        self.simulate_noise = true;
        self.noise_config = config;
        self
    }

    /// Set CRISPR retrieval config (for simulating imperfect retrieval).
    pub fn with_crispr(mut self, config: CrisprConfig) -> Self {
        self.crispr = CrisprSimulator::new(config);
        self
    }

    /// The pool's configured capacity limit in total nucleotides, or
    /// `None` for unlimited.
    pub fn max_nucleotides(&self) -> Option<usize> {
        self.max_nucleotides
    }

    /// Sets (or, with `None`, clears) this pool's capacity limit. This is
    /// pool-level configuration, not a per-write setting -- call
    /// `persist()` afterward for it to actually stick and apply to later
    /// invocations against the same `pool_dir`.
    pub fn set_max_nucleotides(&mut self, max_nucleotides: Option<usize>) {
        self.max_nucleotides = max_nucleotides;
    }

    // -----------------------------------------------------------------------
    // open / persist — durable, cross-process state
    // -----------------------------------------------------------------------

    /// Opens a pool directory, loading whatever was persisted there by a
    /// previous [`Self::persist`] call, or initializing fresh state if the
    /// directory has no `state.json` yet (a brand-new pool). This is what
    /// makes a `nucle store` in one process visible to a `nucle retrieve`
    /// in a later, separate one — [`Self::new`] alone never touches disk.
    ///
    /// `primers`/`search`/`crispr`/`simulate_noise`/`noise_config` are never
    /// persisted directly: `primers` regenerates identically from the same
    /// deterministic seed given the same `max_files` (see [`Self::new`]),
    /// and `search` is rebuilt by replaying every file already in the
    /// restored `catalog` — only `pool`, `catalog`, and the primer-index
    /// counter (`primers_used`, which must survive deletions to avoid
    /// ever reassigning an in-use primer) are real, undiscoverable state.
    pub fn open(pool_dir: &std::path::Path, max_files: usize) -> Result<Self, String> {
        let state_path = pool_dir.join(STATE_FILE_NAME);
        if !state_path.exists() {
            let mut os = Self::new(max_files);
            os.pool_dir = Some(pool_dir.to_path_buf());
            return Ok(os);
        }

        let json = std::fs::read_to_string(&state_path)
            .map_err(|e| format!("failed to read pool state at '{}': {}", state_path.display(), e))?;
        let persisted: PersistedState = serde_json::from_str(&json)
            .map_err(|e| format!("failed to parse pool state at '{}': {}", state_path.display(), e))?;

        let primers = PrimerLibrary::generate(max_files.max(10), 20, 42);
        let mut search = SearchEngine::new(primers.clone());
        for file in persisted.catalog.list() {
            search.register_file(FileMeta {
                file_id: file.file_id.clone(),
                filename: file.filename.clone(),
                size: file.size,
                content_hash: file.content_hash.clone(),
                primer_id: file.primer_id.clone(),
                strand_count: file.data_strand_count + file.parity_strand_count,
            });
        }

        Ok(Self {
            pool: persisted.pool,
            catalog: persisted.catalog,
            primers,
            search,
            crispr: CrisprSimulator::new(CrisprConfig::ideal()),
            simulate_noise: false,
            noise_config: SimulationConfig::pristine(),
            primers_used: persisted.primers_used,
            loaded_version: persisted.version,
            max_nucleotides: persisted.max_nucleotides,
            pool_dir: Some(pool_dir.to_path_buf()),
        })
    }

    /// Best-effort audit-log append for `pool_dir`, a no-op for an
    /// ephemeral instance that was never `open()`ed against a real
    /// directory. A failure to write the audit entry is deliberately
    /// swallowed rather than surfaced as an error from `dna_write`/
    /// `dna_read`/`dna_delete`: it's an observability trail, not a
    /// data-durability guarantee like `persist()` -- losing one audit line
    /// doesn't lose the file data the operation itself already committed
    /// (or refused) on its own terms.
    fn record_audit(&self, operation: &str, filename: &str, archive_id: Option<String>, success: bool, detail: String) {
        if let Some(dir) = &self.pool_dir {
            let event = crate::audit::AuditEvent::new(operation, filename, archive_id, success, detail);
            let _ = crate::audit::append(dir, &event);
        }
    }

    /// Persists `pool`/`catalog`/the primer-index counter to `pool_dir`,
    /// creating it if needed. Writes to a temporary file and `rename`s it
    /// over the real path — an atomic swap on both Windows and Unix — so a
    /// process killed mid-write never leaves a half-written `state.json`
    /// behind; the last successfully persisted state is always intact.
    ///
    /// **Optimistic concurrency check, made atomic by a real file lock**:
    /// before writing, re-reads whatever version is *currently* on disk.
    /// If it's moved past the version this instance last loaded (or
    /// wrote), some other process has persisted a newer state since —
    /// writing now would silently discard that write, so this refuses
    /// instead with a clear, retryable error. Two `open()`s against the
    /// same directory racing is now a real scenario (pools are durable),
    /// where it wasn't when every process got its own empty,
    /// throwaway state. On success, `self.loaded_version` advances, so a
    /// second `persist()` call on the same instance (no intervening
    /// external change) succeeds too, rather than conflicting with itself.
    ///
    /// The check-then-write above isn't safe on its own -- two processes
    /// could both pass the check before either one's `rename()` lands,
    /// and the second write would then silently clobber the first. An
    /// exclusive lock on `pool_dir/.lock`, held for this whole sequence,
    /// closes that window: whichever process gets the lock first runs its
    /// entire check-and-write before the other even starts checking.
    pub fn persist(&mut self, pool_dir: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(pool_dir)
            .map_err(|e| format!("failed to create pool directory '{}': {}", pool_dir.display(), e))?;

        let lock_path = pool_dir.join(LOCK_FILE_NAME);
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("failed to open pool lock file '{}': {}", lock_path.display(), e))?;
        let mut file_lock = fd_lock::RwLock::new(lock_file);
        let _guard = file_lock
            .write()
            .map_err(|e| format!("failed to acquire pool lock at '{}': {}", lock_path.display(), e))?;

        let state_path = pool_dir.join(STATE_FILE_NAME);
        if state_path.exists() {
            let existing_json = std::fs::read_to_string(&state_path)
                .map_err(|e| format!("failed to read pool state at '{}': {}", state_path.display(), e))?;
            let existing: PersistedState = serde_json::from_str(&existing_json)
                .map_err(|e| format!("failed to parse pool state at '{}': {}", state_path.display(), e))?;
            if existing.version != self.loaded_version {
                return Err(format!(
                    "pool at '{}' was changed by another process since this one opened it \
                     (on-disk version {}, expected {}) -- retry the command",
                    pool_dir.display(), existing.version, self.loaded_version
                ));
            }
        } else if self.loaded_version != 0 {
            return Err(format!(
                "pool state at '{}' is missing but this instance expected version {} -- retry the command",
                pool_dir.display(), self.loaded_version
            ));
        }

        let new_version = self.loaded_version + 1;
        let persisted = PersistedState {
            version: new_version,
            pool: self.pool.clone(),
            catalog: self.catalog.clone(),
            primers_used: self.primers_used,
            max_nucleotides: self.max_nucleotides,
        };
        let json = serde_json::to_string_pretty(&persisted)
            .map_err(|e| format!("failed to serialize pool state: {}", e))?;

        let tmp_path = pool_dir.join(format!("{}.tmp", STATE_FILE_NAME));
        std::fs::write(&tmp_path, &json)
            .map_err(|e| format!("failed to write pool state to '{}': {}", tmp_path.display(), e))?;
        std::fs::rename(&tmp_path, pool_dir.join(STATE_FILE_NAME))
            .map_err(|e| format!("failed to finalize pool state: {}", e))?;

        self.loaded_version = new_version;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // dna_write — store a file into DNA
    // -----------------------------------------------------------------------

    /// Store binary data as a file in DNA storage, using the default
    /// Ternary codec. See [`Self::dna_write_with_codec`] to pick a codec.
    ///
    /// `redundancy`: number of RS parity strands (0 = no ECC).
    pub fn dna_write(
        &mut self,
        filename: &str,
        data: &[u8],
        redundancy: usize,
    ) -> Result<WriteResult, String> {
        self.dna_write_with_codec(filename, data, redundancy, CodecKind::Ternary)
    }

    /// Store binary data as a file in DNA storage.
    ///
    /// Full pipeline:
    /// 1. **Codec**: encode binary → DNA strands (`codec`)
    /// 2. **ECC**: compute RS parity strands (if redundancy > 0)
    /// 3. **Primers**: tag each strand with unique file primer pair
    /// 4. **Synth** (optional): simulate synthesis noise
    /// 5. **Pool**: store all tagged strands
    ///
    /// `redundancy`: number of RS parity strands (0 = no ECC).
    pub fn dna_write_with_codec(
        &mut self,
        filename: &str,
        data: &[u8],
        redundancy: usize,
        codec_kind: CodecKind,
    ) -> Result<WriteResult, String> {
        let result = self.dna_write_with_codec_impl(filename, data, redundancy, codec_kind);
        let archive_id = result.as_ref().ok().map(|r| r.file_id.clone());
        let detail = match &result {
            Ok(r) => format!(
                "stored {} bytes across {} strands ({} data + {} parity)",
                data.len(), r.total_strand_count, r.data_strand_count, r.parity_strand_count
            ),
            Err(e) => e.clone(),
        };
        self.record_audit("write", filename, archive_id, result.is_ok(), detail);
        result
    }

    fn dna_write_with_codec_impl(
        &mut self,
        filename: &str,
        data: &[u8],
        redundancy: usize,
        codec_kind: CodecKind,
    ) -> Result<WriteResult, String> {
        // Check if filename already exists
        if self.catalog.contains_name(filename) {
            return Err(format!("file '{}' already exists", filename));
        }

        // Assign a primer pair
        let primer_pair = self.primers
            .assign_next(self.primers_used)
            .ok_or("no primer pairs available")?
            .clone();
        self.primers_used += 1;

        // ── Layer 1: Codec ── encode binary → DNA strands
        let codec = codec_kind.to_boxed();
        let encoded = codec.encode(data)
            .map_err(|e| format!("encoding failed: {}", e))?;

        let data_strand_count = encoded.strands.len();

        // ── Layer 3: ECC ── compute RS parity strands
        let mut parity_strands: Vec<DnaStrand> = Vec::new();
        if redundancy > 0 {
            let rs = ReedSolomon::new(RsConfig::new(redundancy));

            // Convert DNA strands to byte vectors for RS
            let strand_bytes: Vec<Vec<u8>> = encoded.strands.iter()
                .map(|s| s.bases().iter().map(|n| n.to_bits()).collect())
                .collect();

            let parity_bytes = rs.encode_block(&strand_bytes)
                .map_err(|e| format!("ECC encoding failed: {}", e))?;

            // Parity symbols are arbitrary GF(256) values (0-255), unlike
            // data strand bytes which are always a single to_bits() value
            // (0-3) -- pack each parity byte into 4 bases so no value is
            // silently dropped for exceeding the 2-bit range.
            for parity in &parity_bytes {
                parity_strands.push(DnaStrand::from_packed_bytes(parity));
            }
        }

        // ── Layer 4: Index ── tag each strand with primer pair
        let tagged_data: Vec<DnaStrand> = encoded.strands.iter()
            .map(|s| primer_pair.tag_strand(s))
            .collect();

        let tagged_parity: Vec<DnaStrand> = parity_strands.iter()
            .map(|s| primer_pair.tag_strand(s))
            .collect();

        // ── Layer 2: Synth (optional) ── simulate noise
        //
        // When coverage_depth > 1, `simulate()` produces several independent
        // noisy copies of each strand (real sequencing reads a pool many
        // times over). Each copy's `source_index` (its position divided by
        // coverage_depth, per `NoiseEngine::simulate`'s copy-major loop
        // order) is preserved through to storage so `dna_read` can regroup
        // and consensus-vote coverage copies of the same logical strand
        // before Reed-Solomon ever sees them -- RS alone can only recover a
        // strand that's entirely missing, never one that survived corrupted.
        let (final_data, final_data_sources) = simulate_with_provenance(
            self.simulate_noise,
            &self.noise_config,
            tagged_data,
            data.len(),
        );
        let (final_parity, final_parity_sources) = simulate_with_provenance(
            self.simulate_noise,
            &self.noise_config,
            tagged_parity,
            0, // parity has no original data size
        );

        // ── Compute content hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hasher.finalize();
        let content_hash = hash[..8].to_vec();

        // ── Timestamp
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // ── Generate file ID (archive ID)
        // Hash of filename + content_hash + codec + profile + created_at
        let mut id_hasher = Sha256::new();
        id_hasher.update(filename.as_bytes());
        id_hasher.update(&content_hash);
        id_hasher.update(codec.name().as_bytes());
        let profile_str = if self.simulate_noise {
            self.noise_config.synthesis_profile.name()
        } else {
            "pristine"
        };
        id_hasher.update(profile_str.as_bytes());
        id_hasher.update(&created_at.to_be_bytes());
        let id_hash = id_hasher.finalize();
        let file_id = format!("archive-{}", &id_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()[..16]);

        // ── Capacity check ── refuse before touching the pool at all if a
        // limit is set and this file's real, final strand set (post-codec,
        // post-ECC, post-noise-simulation -- the exact nucleotide count
        // that would actually be stored) would exceed it. Checked here
        // rather than against an upfront estimate from `data.len()`: the
        // codec/ECC/noise work above is pure, in-memory, and has no effect
        // on the persisted pool by itself, so there's no real cost to
        // computing the exact figure first -- only the actual insertion
        // below is the thing capacity is protecting.
        if let Some(max) = self.max_nucleotides {
            let incoming: usize = final_data.iter().map(|s| s.len()).sum::<usize>()
                + final_parity.iter().map(|s| s.len()).sum::<usize>();
            let used = self.pool.total_nucleotides();
            if used + incoming > max {
                return Err(format!(
                    "pool capacity exceeded: storing '{}' needs {} more nucleotides, \
                     but only {} of {} remain ({} already used)",
                    filename, incoming, max.saturating_sub(used), max, used
                ));
            }
        }

        // ── Layer 5: VFS ── store in pool
        for (i, (strand, &source_index)) in final_data.iter().zip(final_data_sources.iter()).enumerate() {
            self.pool.add_strand(PoolEntry {
                strand: strand.clone(),
                file_id: file_id.clone(),
                strand_index: i,
                source_index,
                is_parity: false,
            });
        }

        for (i, (strand, &source_index)) in final_parity.iter().zip(final_parity_sources.iter()).enumerate() {
            self.pool.add_strand(PoolEntry {
                strand: strand.clone(),
                file_id: file_id.clone(),
                strand_index: data_strand_count + i,
                source_index,
                is_parity: true,
            });
        }

        // Logical counts describe the file structure that was asked for
        // (data strands + requested parity), not the physical footprint
        // once coverage simulation multiplies strand copies -- that
        // physical total is what `dna_stat()`'s pool-wide numbers report,
        // by counting actual stored entries rather than these fields.
        let parity_count = parity_strands.len();
        let total_strands = data_strand_count + parity_count;
        let redundancy_ratio = if data_strand_count == 0 {
            0.0
        } else {
            total_strands as f64 / data_strand_count as f64
        };

        let simulation_assumptions = if self.simulate_noise {
            Some(SimulationAssumptions {
                seed: self.noise_config.seed,
                coverage_depth: self.noise_config.coverage_depth,
                synthesis_profile: self.noise_config.synthesis_profile.name().to_string(),
                sequencing_profile: self.noise_config.sequencing_profile.name().to_string(),
            })
        } else {
            None
        };

        let manifest = StorageManifest {
            archive_id: file_id.clone(),
            codec: codec.name().to_string(),
            profile: profile_str.to_string(),
            redundancy: parity_count,
            primer_set: primer_pair.id.clone(),
            index_strategy: "primer-addressing".to_string(),
            simulation_assumptions,
            created_at: created_at as i64,
            recovery_manifest: None,
        };

        // ── Register in catalog
        let dna_file = DnaFile {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            size: data.len(),
            content_hash: content_hash.clone(),
            created_at,
            primer_id: primer_pair.id.clone(),
            data_strand_count,
            parity_strand_count: parity_count,
            rs_parity_per_stripe: redundancy,
            codec: codec.name().to_string(),
            redundancy: redundancy_ratio,
            manifest: Some(manifest),
            manifest_history: Vec::new(),
        };
        self.catalog.register(dna_file);

        // ── Register in search engine
        self.search.register_file(FileMeta {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            size: data.len(),
            content_hash,
            primer_id: primer_pair.id.clone(),
            strand_count: total_strands,
        });

        Ok(WriteResult {
            file_id,
            filename: filename.to_string(),
            data_size: data.len(),
            data_strand_count,
            parity_strand_count: parity_count,
            total_strand_count: total_strands,
            primer_id: primer_pair.id,
            redundancy: redundancy_ratio,
        })
    }

    // -----------------------------------------------------------------------
    // dna_read — retrieve a file from DNA
    // -----------------------------------------------------------------------

    /// Read a file back from DNA storage.
    ///
    /// Full pipeline:
    /// 1. **Catalog**: look up file metadata
    /// 2. **CRISPR**: selectively retrieve strands by primer matching
    /// 3. **Primers**: untag strands (remove primer flanking regions)
    /// 4. **ECC**: RS decode to recover any missing strands
    /// 5. **Codec**: decode DNA → binary data
    pub fn dna_read(&mut self, filename: &str) -> Result<Vec<u8>, String> {
        // Looked up before the real read so a failure (corruption, missing
        // strands) still logs the archive_id the read was actually for --
        // this lookup can't be affected by anything dna_read_impl does,
        // since a read never removes or renames a catalog entry.
        let archive_id = self.catalog.get_by_name(filename).map(|f| f.file_id.clone());
        let result = self.dna_read_impl(filename);
        let detail = match &result {
            Ok(bytes) => format!("read {} bytes", bytes.len()),
            Err(e) => e.clone(),
        };
        self.record_audit("read", filename, archive_id, result.is_ok(), detail);
        result
    }

    fn dna_read_impl(&mut self, filename: &str) -> Result<Vec<u8>, String> {
        // ── Layer 5: VFS ── look up file
        // Cloned (not borrowed) so the catalog can be mutably re-borrowed
        // below to persist the recovery manifest onto this object's entry.
        let dna_file = self.catalog.get_by_name(filename)
            .ok_or(format!("file '{}' not found", filename))?
            .clone();

        // ── Layer 4: Index ── CRISPR selective retrieval
        let primer_pair = self.primers.get(&dna_file.primer_id)
            .ok_or(format!("primer '{}' not found", dna_file.primer_id))?;

        // Collect all strands from pool for CRISPR retrieval
        let all_pool_strands: Vec<DnaStrand> = self.pool
            .all_strands()
            .into_iter()
            .cloned()
            .collect();

        let retrieval = self.crispr.retrieve(&all_pool_strands, primer_pair);

        if retrieval.target_strands.is_empty() {
            return Err(format!(
                "CRISPR retrieval failed for file '{}': no strands amplified",
                filename
            ));
        }

        // ── Layer 4: Index ── untag strands (remove primers), grouped by
        // which logical strand each retrieved copy is a coverage-read of
        // (see `simulate_with_provenance` in `dna_write`). With no coverage
        // simulation this is just one read per group, same as before.
        let pool_entries = self.pool.get_file_strands(&dna_file.file_id);

        let mut data_groups: HashMap<usize, Vec<DnaStrand>> = HashMap::new();
        let mut parity_groups: HashMap<usize, Vec<DnaStrand>> = HashMap::new();
        for &entry in &pool_entries {
            let Some(strand) = primer_pair.untag_strand(&entry.strand) else { continue };
            let groups = if entry.is_parity { &mut parity_groups } else { &mut data_groups };
            groups.entry(entry.source_index).or_default().push(strand);
        }

        // ── Layer 3: ECC ── consensus-vote coverage copies of each logical
        // strand, then Reed-Solomon over the consensus results (see
        // `nucle_ecc::pipeline::consensus_then_rs_decode` for why this is
        // what actually lets redundancy help under substitution-heavy
        // noise, which RS alone cannot).
        let dense_data_groups: Vec<Vec<DnaStrand>> = (0..dna_file.data_strand_count)
            .map(|i| data_groups.remove(&i).unwrap_or_default())
            .collect();
        let dense_parity_groups: Vec<Vec<DnaStrand>> = (0..dna_file.parity_strand_count)
            .map(|i| parity_groups.remove(&i).unwrap_or_default())
            .collect();

        // Kept so we can diff pre- vs. post-correction strands for a real
        // observed error distribution: one arbitrary raw read per position
        // (or an empty placeholder where nothing survived) as the baseline.
        let pre_correction_strands: Vec<DnaStrand> = dense_data_groups.iter()
            .map(|g| g.first().cloned().unwrap_or_else(|| DnaStrand::new(Vec::new())))
            .collect();

        // ── Layer 1: Codec ── set up ahead of the ECC step so its
        // validator can be reused as the ECC retry's ground-truth check
        // (see `consensus_then_rs_decode_with_retry`'s doc comment).
        let codec_kind = CodecKind::from_codec_name(&dna_file.codec)
            .ok_or_else(|| format!("unknown codec '{}' recorded for this file", dna_file.codec))?;
        let codec = codec_kind.to_boxed();
        let is_valid = |strands: &[DnaStrand]| -> bool {
            if strands.is_empty() {
                return false;
            }
            let collection = StrandCollection::from_strands(strands.to_vec(), dna_file.size);
            let Ok(decoded) = codec.decode(&collection) else { return false };
            let mut hasher = Sha256::new();
            hasher.update(&decoded);
            hasher.finalize()[..8].to_vec() == dna_file.content_hash
        };

        let decoded_strands: Vec<DnaStrand> = if dna_file.parity_strand_count > 0 {
            consensus_then_rs_decode_with_retry(
                &dense_data_groups,
                &dense_parity_groups,
                RsConfig::new(dna_file.rs_parity_per_stripe),
                is_valid,
            )
        } else {
            consensus_then_rs_decode_with_retry(&dense_data_groups, &[], RsConfig::new(0), is_valid)
        };

        if decoded_strands.is_empty() {
            return Err("no data strands after ECC decode".into());
        }

        // Real per-position signal: where correction actually changed bases,
        // derived from the strands themselves rather than a profile estimate.
        let observed_error_distribution = compute_error_distribution(&pre_correction_strands, &decoded_strands);

        let recovered_strands_count = decoded_strands.len();
        let collection = StrandCollection::from_strands(decoded_strands, dna_file.size);
        let decoded = codec.decode(&collection)
            .map_err(|e| format!("codec decoding failed: {}", e))?;

        // ── Verify content hash
        let mut hasher = Sha256::new();
        hasher.update(&decoded);
        let hash = hasher.finalize();
        let recovered_hash = hash[..8].to_vec();

        let ecc_success = recovered_hash == dna_file.content_hash;
        let observed_error = if self.simulate_noise {
            let rates = self.noise_config.sequencing_profile.error_rates();
            rates.substitution + rates.insertion + rates.deletion
        } else {
            0.0
        };
        let seq_profile = if self.simulate_noise {
            self.noise_config.sequencing_profile.name().to_string()
        } else {
            "pristine".to_string()
        };

        let recovery_manifest = RecoveryManifest {
            observed_error_rate: observed_error,
            consensus_method: "majority-vote".to_string(),
            sequencing_profile: seq_profile,
            recovered_strands: recovered_strands_count,
            ecc_success,
            observed_error_distribution,
        };

        // Persist onto this object's own storage manifest (keyed by its
        // archive_id via the catalog entry), not session-global state — so
        // reading a different file afterward can't clobber this one's history.
        if let Some(file) = self.catalog.get_by_name_mut(filename) {
            if let Some(manifest) = file.manifest.as_mut() {
                manifest.recovery_manifest = Some(recovery_manifest);
            }
        }

        if !ecc_success {
            return Err(format!(
                "content hash mismatch: data may be corrupted (expected {:?}, got {:?})",
                &dna_file.content_hash, &recovered_hash
            ));
        }

        Ok(decoded)
    }

    // -----------------------------------------------------------------------
    // dna_stat — pool and file statistics
    // -----------------------------------------------------------------------

    /// Get comprehensive pool statistics.
    pub fn dna_stat(&self) -> PoolStatus {
        PoolStatus {
            file_count: self.catalog.len(),
            total_strands: self.pool.total_strands(),
            data_strands: self.pool.total_data_strands(),
            parity_strands: self.pool.total_parity_strands(),
            total_nucleotides: self.pool.total_nucleotides(),
            avg_strand_length: self.pool.avg_strand_length(),
            redundancy: self.pool.redundancy_ratio(),
            files: self.catalog.list().iter().map(|f| file_info(f)).collect(),
        }
    }

    /// Lists files whose name starts with `prefix` (an empty prefix lists
    /// everything) -- a directory-listing-style view over the catalog's
    /// flat, path-string namespace (e.g. `dna_list("docs/")` after storing
    /// under names like `"docs/report.txt"`; see `Catalog::list_prefixed`).
    pub fn dna_list(&self, prefix: &str) -> Vec<FileInfo> {
        self.catalog.list_prefixed(prefix).iter().map(|f| file_info(f)).collect()
    }

    // -----------------------------------------------------------------------
    // dna_delete — remove a file
    // -----------------------------------------------------------------------

    /// Delete a file from DNA storage. Removes strands, catalog entry, and search index.
    pub fn dna_delete(&mut self, filename: &str) -> Result<DeleteResult, String> {
        // Looked up before the real delete: once dna_delete_impl succeeds,
        // the catalog entry (and its file_id) is gone, so this is the only
        // point the archive_id is still available to attach to the event.
        let archive_id = self.catalog.get_by_name(filename).map(|f| f.file_id.clone());
        let result = self.dna_delete_impl(filename);
        let detail = match &result {
            Ok(r) => format!("removed {} strands", r.strands_removed),
            Err(e) => e.clone(),
        };
        self.record_audit("delete", filename, archive_id, result.is_ok(), detail);
        result
    }

    fn dna_delete_impl(&mut self, filename: &str) -> Result<DeleteResult, String> {
        let dna_file = self.catalog.get_by_name(filename)
            .ok_or(format!("file '{}' not found", filename))?;
        let file_id = dna_file.file_id.clone();
        let strand_count = dna_file.total_strands();

        self.pool.remove_file(&file_id);
        self.catalog.remove(&file_id);
        self.search.remove_file(&file_id);

        Ok(DeleteResult {
            filename: filename.to_string(),
            strands_removed: strand_count,
        })
    }

    // -----------------------------------------------------------------------
    // dna_search — search for files
    // -----------------------------------------------------------------------

    /// Search for files matching a query.
    pub fn dna_search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.search.search(query, top_k)
    }
}

// ---------------------------------------------------------------------------
// Result Types
// ---------------------------------------------------------------------------

/// Result of a dna_write operation.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub file_id: String,
    pub filename: String,
    pub data_size: usize,
    pub data_strand_count: usize,
    pub parity_strand_count: usize,
    pub total_strand_count: usize,
    pub primer_id: String,
    pub redundancy: f64,
}

impl fmt::Display for WriteResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Stored '{}' ({} bytes → {} data + {} parity = {} strands, \
             {:.2}× redundancy, primer={})",
            self.filename,
            self.data_size,
            self.data_strand_count,
            self.parity_strand_count,
            self.total_strand_count,
            self.redundancy,
            self.primer_id
        )
    }
}

/// Result of a dna_delete operation.
#[derive(Debug, Clone, Serialize)]
pub struct DeleteResult {
    pub filename: String,
    pub strands_removed: usize,
}

/// A file summary in pool status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub filename: String,
    pub size: usize,
    pub data_strands: usize,
    pub parity_strands: usize,
    pub total_strands: usize,
    pub codec: String,
    pub redundancy: f64,
    pub manifest: Option<StorageManifest>,
}

/// Shared by `dna_stat`/`dna_list` so both build a `FileInfo` the same way.
fn file_info(f: &DnaFile) -> FileInfo {
    FileInfo {
        filename: f.filename.clone(),
        size: f.size,
        data_strands: f.data_strand_count,
        parity_strands: f.parity_strand_count,
        total_strands: f.total_strands(),
        codec: f.codec.clone(),
        redundancy: f.redundancy,
        manifest: f.manifest.clone(),
    }
}

/// Pool status report.
#[derive(Debug, Clone, Serialize)]
pub struct PoolStatus {
    pub file_count: usize,
    pub total_strands: usize,
    pub data_strands: usize,
    pub parity_strands: usize,
    pub total_nucleotides: usize,
    pub avg_strand_length: f64,
    pub redundancy: f64,
    pub files: Vec<FileInfo>,
}

impl fmt::Display for PoolStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "╔══════════════════════════════════════╗")?;
        writeln!(f, "║         NucleOS Pool Status          ║")?;
        writeln!(f, "╠══════════════════════════════════════╣")?;
        writeln!(f, "║ Files:          {:>6}               ║", self.file_count)?;
        writeln!(f, "║ Total strands:  {:>6}               ║", self.total_strands)?;
        writeln!(f, "║ Data strands:   {:>6}               ║", self.data_strands)?;
        writeln!(f, "║ Parity strands: {:>6}               ║", self.parity_strands)?;
        writeln!(f, "║ Nucleotides:    {:>6}               ║", self.total_nucleotides)?;
        writeln!(f, "║ Avg strand len: {:>6.0} nt            ║", self.avg_strand_length)?;
        writeln!(f, "║ Redundancy:     {:>5.2}×              ║", self.redundancy)?;
        writeln!(f, "╟──────────────────────────────────────╢")?;
        writeln!(f, "║ Files:                               ║")?;
        for fi in &self.files {
            if let Some(ref m) = fi.manifest {
                let id_short = if m.archive_id.len() > 12 { &m.archive_id[..12] } else { &m.archive_id };
                writeln!(
                    f,
                    "║   {} (ID: {}, {} B, {}d+{}p strands, {:.1}×)",
                    fi.filename, id_short, fi.size, fi.data_strands, fi.parity_strands, fi.redundancy
                )?;
            } else {
                writeln!(
                    f,
                    "║   {} ({} B, {}d+{}p strands, {:.1}×)",
                    fi.filename, fi.size, fi.data_strands, fi.parity_strands, fi.redundancy
                )?;
            }
        }
        writeln!(f, "╚══════════════════════════════════════╝")?;
        for fi in &self.files {
            if let Some(r) = fi.manifest.as_ref().and_then(|m| m.recovery_manifest.as_ref()) {
                writeln!(f, "\n--- Recovery Manifest: {} ---", fi.filename)?;
                writeln!(f, "Observed Error Rate: {:.4}%", r.observed_error_rate * 100.0)?;
                writeln!(f, "Consensus Method:    {}", r.consensus_method)?;
                writeln!(f, "Sequencing Profile:  {}", r.sequencing_profile)?;
                writeln!(f, "Recovered Strands:   {}", r.recovered_strands)?;
                writeln!(f, "ECC Success:         {}", r.ecc_success)?;
                if !r.observed_error_distribution.is_empty() {
                    let flagged = r.observed_error_distribution.iter().filter(|(_, rate)| *rate > 0.0).count();
                    writeln!(f, "Positions w/ errors: {} of {}", flagged, r.observed_error_distribution.len())?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic roundtrip ──

    #[test]
    fn test_write_read_no_ecc() {
        let mut os = NucleOS::new(10);
        let data = b"Hello, NucleOS! No ECC roundtrip test.";

        let result = os.dna_write("hello.txt", data, 0).unwrap();
        assert_eq!(result.parity_strand_count, 0);
        assert!((result.redundancy - 1.0).abs() < 0.01);

        let recovered = os.dna_read("hello.txt").unwrap();
        assert_eq!(recovered, data.to_vec());
    }

    #[test]
    fn test_write_read_with_ecc() {
        let mut os = NucleOS::new(10);
        let data = b"Hello, NucleOS! With RS ECC parity strands.";

        let result = os.dna_write("ecc.txt", data, 4).unwrap();
        assert_eq!(result.parity_strand_count, 4);
        assert!(result.redundancy > 1.0);

        let recovered = os.dna_read("ecc.txt").unwrap();
        assert_eq!(recovered, data.to_vec());
    }

    // ── Capacity limits ──

    #[test]
    fn unlimited_by_default_matches_todays_behavior() {
        let mut os = NucleOS::new(10);
        assert_eq!(os.max_nucleotides(), None);
        // A reasonably large write still succeeds with no limit configured.
        assert!(os.dna_write("big.bin", &vec![0u8; 5000], 2).is_ok());
    }

    #[test]
    fn a_write_that_would_exceed_capacity_is_refused_before_touching_the_pool() {
        let mut os = NucleOS::new(10);
        os.set_max_nucleotides(Some(100));
        let before = os.pool.total_nucleotides();

        let result = os.dna_write("too_big.bin", &vec![0u8; 5000], 2);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("capacity exceeded"));

        // Nothing was added to the pool, and no catalog entry exists --
        // the refusal must be all-or-nothing, not a partial write.
        assert_eq!(os.pool.total_nucleotides(), before);
        assert!(os.catalog.get_by_name("too_big.bin").is_none());
    }

    #[test]
    fn a_write_that_fits_within_capacity_still_succeeds() {
        let mut os = NucleOS::new(10);
        // Establish real headroom by measuring one write's actual cost, so
        // this test doesn't hardcode a nucleotide count that could drift
        // if the codec's own encoding ratio ever changes.
        let mut probe = NucleOS::new(10);
        probe.dna_write("probe.bin", b"small file", 1).unwrap();
        let one_files_nucleotides = probe.pool.total_nucleotides();

        os.set_max_nucleotides(Some(one_files_nucleotides * 3));
        assert!(os.dna_write("fits.bin", b"small file", 1).is_ok());
    }

    #[test]
    fn set_max_nucleotides_persists_and_is_enforced_after_reopening() {
        let dir = scratch_pool_dir("capacity_persists");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let mut os = NucleOS::open(&dir, 10).unwrap();
            os.set_max_nucleotides(Some(50));
            os.persist(&dir).unwrap();
        }

        let mut reopened = NucleOS::open(&dir, 10).unwrap();
        assert_eq!(reopened.max_nucleotides(), Some(50));
        let result = reopened.dna_write("too_big.bin", &vec![0u8; 5000], 2);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("capacity exceeded"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_the_capacity_limit_removes_the_restriction() {
        let mut os = NucleOS::new(10);
        os.set_max_nucleotides(Some(1));
        assert!(os.dna_write("blocked.bin", &vec![0u8; 5000], 2).is_err());

        os.set_max_nucleotides(None);
        assert!(os.dna_write("now_fine.bin", &vec![0u8; 5000], 2).is_ok());
    }

    // ── System-wide audit log ──

    #[test]
    fn an_ephemeral_new_instance_never_touched_a_pool_dir_logs_nothing() {
        let mut os = NucleOS::new(10);
        os.dna_write("a.txt", b"data", 0).unwrap();
        os.dna_read("a.txt").unwrap();
        os.dna_delete("a.txt").unwrap();
        // No pool_dir was ever set, so there's nowhere real to have logged
        // to -- this just proves record_audit's no-op path doesn't panic
        // or otherwise misbehave for the common in-memory-only case.
    }

    #[test]
    fn a_successful_write_read_and_delete_each_append_one_event() {
        let dir = scratch_pool_dir("audit_happy_path");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("a.txt", b"hello audit log", 0).unwrap();
        os.dna_read("a.txt").unwrap();
        os.dna_delete("a.txt").unwrap();

        let events = crate::audit::read_events(&dir).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].operation, "write");
        assert!(events[0].success);
        assert!(events[0].archive_id.is_some());
        assert_eq!(events[1].operation, "read");
        assert!(events[1].success);
        assert_eq!(events[2].operation, "delete");
        assert!(events[2].success);
        // The delete event -- and its archive_id -- is the file's only
        // remaining trace once the catalog entry itself is gone.
        assert_eq!(events[2].archive_id, events[0].archive_id);
        assert!(os.catalog.get_by_name("a.txt").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_operation_still_appends_an_event() {
        let dir = scratch_pool_dir("audit_failure_path");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os = NucleOS::open(&dir, 10).unwrap();
        assert!(os.dna_read("does_not_exist.txt").is_err());

        let events = crate::audit::read_events(&dir).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "read");
        assert!(!events[0].success);
        assert!(events[0].archive_id.is_none());
        assert!(events[0].detail.contains("not found"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_migration_logs_as_a_read_delete_write_trail() {
        let dir = scratch_pool_dir("audit_migration");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("m.txt", b"migrate me", 1).unwrap();
        crate::migrate::migrate_object(&mut os, "m.txt", Some(3), None).unwrap();

        let events = crate::audit::read_events(&dir).unwrap();
        // 1 initial write, then migrate's own read + delete + write.
        assert_eq!(events.len(), 4);
        assert_eq!(
            events.iter().map(|e| e.operation.as_str()).collect::<Vec<_>>(),
            vec!["write", "read", "delete", "write"]
        );
        assert!(events.iter().all(|e| e.success));
        assert!(events.iter().all(|e| e.filename == "m.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Hierarchical, path-like names ──

    #[test]
    fn same_leaf_name_under_different_path_prefixes_does_not_collide() {
        let mut os = NucleOS::new(10);
        os.dna_write("docs/readme.txt", b"the docs one", 1).unwrap();
        os.dna_write("downloads/readme.txt", b"the downloads one", 1).unwrap();

        assert_eq!(os.dna_read("docs/readme.txt").unwrap(), b"the docs one".to_vec());
        assert_eq!(os.dna_read("downloads/readme.txt").unwrap(), b"the downloads one".to_vec());
    }

    #[test]
    fn dna_list_filters_by_prefix() {
        let mut os = NucleOS::new(10);
        os.dna_write("docs/readme.txt", b"a", 1).unwrap();
        os.dna_write("docs/notes.txt", b"b", 1).unwrap();
        os.dna_write("downloads/readme.txt", b"c", 1).unwrap();

        let docs = os.dna_list("docs/");
        assert_eq!(docs.len(), 2);
        assert!(docs.iter().all(|f| f.filename.starts_with("docs/")));

        assert_eq!(os.dna_list("").len(), 3);
    }

    // ── Binary data ──

    #[test]
    fn test_binary_data_with_ecc() {
        let mut os = NucleOS::new(10);
        let data: Vec<u8> = (0..=255).collect();

        os.dna_write("binary.bin", &data, 2).unwrap();
        let recovered = os.dna_read("binary.bin").unwrap();
        assert_eq!(recovered, data);
    }

    // ── Multiple files ──

    #[test]
    fn test_multiple_files_isolation() {
        let mut os = NucleOS::new(10);

        os.dna_write("f1.txt", b"First file", 2).unwrap();
        os.dna_write("f2.txt", b"Second file", 0).unwrap();
        os.dna_write("f3.txt", b"Third file", 4).unwrap();

        let status = os.dna_stat();
        assert_eq!(status.file_count, 3);
        assert!(status.parity_strands > 0); // f1 and f3 have parity

        assert_eq!(os.dna_read("f1.txt").unwrap(), b"First file");
        assert_eq!(os.dna_read("f2.txt").unwrap(), b"Second file");
        assert_eq!(os.dna_read("f3.txt").unwrap(), b"Third file");
    }

    // ── CRISPR retrieval ──

    #[test]
    fn test_crispr_retrieval_path() {
        let mut os = NucleOS::new(10);

        os.dna_write("target.txt", b"CRISPR target data", 0).unwrap();
        os.dna_write("other.txt", b"Other file data here", 0).unwrap();

        // Read goes through CRISPR — should only get target strands
        let recovered = os.dna_read("target.txt").unwrap();
        assert_eq!(recovered, b"CRISPR target data");
    }

    // ── ECC + CRISPR combined ──

    #[test]
    fn test_full_stack_ecc_and_crispr() {
        let mut os = NucleOS::new(10);
        let data = b"Full stack integration: Codec -> ECC -> Primers -> CRISPR -> Pool";

        let result = os.dna_write("fullstack.txt", data, 3).unwrap();
        assert!(result.parity_strand_count > 0);

        let recovered = os.dna_read("fullstack.txt").unwrap();
        assert_eq!(recovered, data.to_vec(), "full stack roundtrip failed");
    }

    // ── Error cases ──

    #[test]
    fn test_duplicate_filename_error() {
        let mut os = NucleOS::new(10);
        os.dna_write("test.txt", b"data", 0).unwrap();
        assert!(os.dna_write("test.txt", b"other", 0).is_err());
    }

    #[test]
    fn test_read_nonexistent_error() {
        let mut os = NucleOS::new(10);
        assert!(os.dna_read("missing.txt").is_err());
    }

    // ── Delete ──

    #[test]
    fn test_delete_with_ecc() {
        let mut os = NucleOS::new(10);
        os.dna_write("temp.txt", b"temporary data", 2).unwrap();

        let status = os.dna_stat();
        assert_eq!(status.file_count, 1);
        assert!(status.parity_strands > 0);

        let del = os.dna_delete("temp.txt").unwrap();
        assert!(del.strands_removed > 0);

        let status = os.dna_stat();
        assert_eq!(status.file_count, 0);
        assert_eq!(status.total_strands, 0);
    }

    // ── Status & search ──

    #[test]
    fn test_pool_status_shows_ecc() {
        let mut os = NucleOS::new(10);
        os.dna_write("test.txt", b"Status test", 4).unwrap();

        let status = os.dna_stat();
        let display = format!("{}", status);
        assert!(display.contains("NucleOS"));
        assert!(display.contains("test.txt"));
        assert!(status.parity_strands > 0);
        assert!(status.redundancy > 1.0);
    }

    #[test]
    fn test_search() {
        let mut os = NucleOS::new(10);
        os.dna_write("readme.txt", b"read me", 0).unwrap();
        os.dna_write("photo.jpg", b"photo data here", 2).unwrap();

        let results = os.dna_search("readme", 5);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_write_result_display() {
        let result = WriteResult {
            file_id: "f1".into(),
            filename: "test.txt".into(),
            data_size: 100,
            data_strand_count: 5,
            parity_strand_count: 2,
            total_strand_count: 7,
            primer_id: "P0000".into(),
            redundancy: 1.4,
        };
        let display = format!("{}", result);
        assert!(display.contains("test.txt"));
        assert!(display.contains("100 bytes"));
        assert!(display.contains("5 data"));
        assert!(display.contains("2 parity"));
    }

    // ── Content hash verification ──

    #[test]
    fn test_content_hash_integrity() {
        let mut os = NucleOS::new(10);
        let data = b"Hash integrity test data";

        os.dna_write("hash.txt", data, 2).unwrap();

        // Verify the catalog stores the correct hash
        let file = os.catalog.get_by_name("hash.txt").unwrap();
        let mut hasher = Sha256::new();
        hasher.update(data);
        let expected_hash = hasher.finalize()[..8].to_vec();
        assert_eq!(file.content_hash, expected_hash);

        // Read verifies hash automatically
        let recovered = os.dna_read("hash.txt").unwrap();
        assert_eq!(recovered, data.to_vec());
    }

    // ── Durable persistence (open/persist) ──

    /// A unique-per-test scratch pool directory so parallel test threads
    /// never collide, matching the pattern already used by
    /// `nucle_hardware`'s own file-based tests.
    fn scratch_pool_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nucle_vfs_test_pool_{}_{}", name, std::process::id()))
    }

    #[test]
    fn open_on_a_fresh_directory_is_equivalent_to_new() {
        let dir = scratch_pool_dir("fresh");
        let _ = std::fs::remove_dir_all(&dir);

        let os = NucleOS::open(&dir, 10).unwrap();
        assert_eq!(os.catalog.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_stored_and_persisted_is_readable_after_reopening() {
        // This must succeed across two genuinely separate NucleOS
        // instances, not just one process's own memory.
        let dir = scratch_pool_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let mut os = NucleOS::open(&dir, 10).unwrap();
            os.dna_write("persisted.txt", b"durable data", 2).unwrap();
            os.persist(&dir).unwrap();
        }

        // A brand-new NucleOS instance, as a later CLI invocation would be.
        let mut reopened = NucleOS::open(&dir, 10).unwrap();
        let recovered = reopened.dna_read("persisted.txt").unwrap();
        assert_eq!(recovered, b"durable data".to_vec());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_is_rebuilt_from_the_restored_catalog() {
        let dir = scratch_pool_dir("search_rebuild");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let mut os = NucleOS::open(&dir, 10).unwrap();
            os.dna_write("searchable_report.txt", b"quarterly numbers", 2).unwrap();
            os.persist(&dir).unwrap();
        }

        let reopened = NucleOS::open(&dir, 10).unwrap();
        let results = reopened.dna_search("name:searchable_report.txt", 5);
        assert!(!results.is_empty(), "search must find a file stored before this process started");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn primers_used_survives_a_delete_so_a_primer_is_never_reassigned() {
        // primers_used only ever increments (see dna_write); deleting a
        // file must not roll it back, or a later write could reuse an
        // already-in-use primer. Proven across a real reopen, not just
        // within one process.
        let dir = scratch_pool_dir("primer_counter");
        let _ = std::fs::remove_dir_all(&dir);

        let first_primer;
        {
            let mut os = NucleOS::open(&dir, 10).unwrap();
            let r1 = os.dna_write("a.txt", b"first", 1).unwrap();
            first_primer = r1.primer_id.clone();
            os.dna_delete("a.txt").unwrap();
            os.persist(&dir).unwrap();
        }

        let mut reopened = NucleOS::open(&dir, 10).unwrap();
        let r2 = reopened.dna_write("b.txt", b"second", 1).unwrap();
        assert_ne!(r2.primer_id, first_primer, "must not reassign a's primer to b after reopening");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_is_atomic_a_stray_tmp_file_is_never_loaded() {
        // Simulates a process killed between writing the temp file and the
        // rename that finalizes it: open() must still load the last good
        // state.json, never a half-written .tmp.
        let dir = scratch_pool_dir("atomic");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let mut os = NucleOS::open(&dir, 10).unwrap();
            os.dna_write("good.txt", b"last good state", 1).unwrap();
            os.persist(&dir).unwrap();
        }

        // Simulate a crash mid-persist: a leftover, truncated temp file
        // sitting next to the real, already-finalized state.json.
        std::fs::write(dir.join(format!("{}.tmp", STATE_FILE_NAME)), b"{ not even valid json").unwrap();

        let reopened = NucleOS::open(&dir, 10).unwrap();
        assert!(reopened.catalog.get_by_name("good.txt").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Optimistic concurrency ──

    #[test]
    fn a_stale_persist_after_a_concurrent_write_is_rejected_not_silently_lost() {
        // Simulates two processes racing on the same pool: both open the
        // same starting state, A persists first, then B -- still holding
        // its now-stale view -- must be refused, not silently overwrite
        // A's already-persisted write.
        let dir = scratch_pool_dir("concurrency_conflict");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os_a = NucleOS::open(&dir, 10).unwrap();
        let mut os_b = NucleOS::open(&dir, 10).unwrap();

        os_a.dna_write("a.txt", b"from process A", 1).unwrap();
        os_a.persist(&dir).unwrap();

        os_b.dna_write("b.txt", b"from process B", 1).unwrap();
        let result = os_b.persist(&dir);
        assert!(result.is_err(), "a stale persist must be rejected, not silently discard A's write");
        assert!(result.unwrap_err().contains("retry"), "the error should be actionable");

        // A's write must be intact; B's rejected attempt must not appear.
        let reopened = NucleOS::open(&dir, 10).unwrap();
        assert!(reopened.catalog.get_by_name("a.txt").is_some());
        assert!(reopened.catalog.get_by_name("b.txt").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_concurrent_opens_of_a_brand_new_pool_the_second_persist_still_conflicts() {
        // Edge case: both instances start from "no state.json yet"
        // (loaded_version 0), not just from an already-persisted version.
        let dir = scratch_pool_dir("concurrent_new_pool");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os_a = NucleOS::open(&dir, 10).unwrap();
        let mut os_b = NucleOS::open(&dir, 10).unwrap();

        os_a.dna_write("a.txt", b"a", 1).unwrap();
        os_a.persist(&dir).unwrap();

        os_b.dna_write("b.txt", b"b", 1).unwrap();
        assert!(os_b.persist(&dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sequential_persists_on_the_same_instance_both_succeed() {
        // Guards against the version check conflicting with itself: after a
        // successful persist, this same instance's own loaded_version must
        // advance so a later persist (no external change in between) isn't
        // treated as stale.
        let dir = scratch_pool_dir("sequential_persist");
        let _ = std::fs::remove_dir_all(&dir);

        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("first.txt", b"one", 1).unwrap();
        os.persist(&dir).unwrap();

        os.dna_write("second.txt", b"two", 1).unwrap();
        os.persist(&dir).unwrap();

        let reopened = NucleOS::open(&dir, 10).unwrap();
        assert!(reopened.catalog.get_by_name("first.txt").is_some());
        assert!(reopened.catalog.get_by_name("second.txt").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_state_json_from_before_versioning_existed_still_loads() {
        // Backward compatibility: a state.json written by an earlier
        // release (before the version field existed) has no "version" key
        // at all -- #[serde(default)] must make it load as version 0, not
        // fail to parse.
        let dir = scratch_pool_dir("pre_versioning");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let pre_versioning_json = r#"{
            "pool": {"strands": {}, "next_id": 0, "file_index": {}},
            "catalog": {"files": {}, "name_index": {}, "primer_index": {}},
            "primers_used": 0
        }"#;
        std::fs::write(dir.join(STATE_FILE_NAME), pre_versioning_json).unwrap();

        let mut os = NucleOS::open(&dir, 10).unwrap();
        os.dna_write("new.txt", b"first write under versioning", 1).unwrap();
        os.persist(&dir).unwrap();

        let reopened = NucleOS::open(&dir, 10).unwrap();
        assert!(reopened.catalog.get_by_name("new.txt").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
