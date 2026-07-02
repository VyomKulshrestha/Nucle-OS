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
use nucle_ecc::pipeline::{compute_error_distribution, consensus_then_rs_decode, RecoveryManifest};
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
    /// Search engine for semantic file lookup.
    pub search: SearchEngine,
    /// CRISPR simulator for selective retrieval.
    crispr: CrisprSimulator,
    /// Whether to simulate synthesis/sequencing noise on write.
    pub simulate_noise: bool,
    /// Noise simulation config (used when simulate_noise is true).
    pub noise_config: SimulationConfig,
    /// Number of primer pairs used so far.
    primers_used: usize,
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

            // Convert parity bytes back to DNA strands
            for parity in &parity_bytes {
                let bases: Vec<_> = parity.iter()
                    .filter_map(|&b| nucle_codec::base::Nucleotide::from_bits(b).ok())
                    .collect();
                parity_strands.push(DnaStrand::new(bases));
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

        let decoded_strands: Vec<DnaStrand> = if dna_file.parity_strand_count > 0 {
            consensus_then_rs_decode(
                &dense_data_groups,
                &dense_parity_groups,
                RsConfig::new(dna_file.rs_parity_per_stripe),
            )
        } else {
            consensus_then_rs_decode(&dense_data_groups, &[], RsConfig::new(0))
        };

        if decoded_strands.is_empty() {
            return Err("no data strands after ECC decode".into());
        }

        // Real per-position signal: where correction actually changed bases,
        // derived from the strands themselves rather than a profile estimate.
        let observed_error_distribution = compute_error_distribution(&pre_correction_strands, &decoded_strands);

        // ── Layer 1: Codec ── decode DNA → binary
        let recovered_strands_count = decoded_strands.len();
        let collection = StrandCollection::from_strands(decoded_strands, dna_file.size);
        let codec_kind = CodecKind::from_codec_name(&dna_file.codec)
            .ok_or_else(|| format!("unknown codec '{}' recorded for this file", dna_file.codec))?;
        let codec = codec_kind.to_boxed();
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
            files: self.catalog.list().iter().map(|f| FileInfo {
                filename: f.filename.clone(),
                size: f.size,
                data_strands: f.data_strand_count,
                parity_strands: f.parity_strand_count,
                total_strands: f.total_strands(),
                codec: f.codec.clone(),
                redundancy: f.redundancy,
                manifest: f.manifest.clone(),
            }).collect(),
        }
    }

    // -----------------------------------------------------------------------
    // dna_delete — remove a file
    // -----------------------------------------------------------------------

    /// Delete a file from DNA storage. Removes strands, catalog entry, and search index.
    pub fn dna_delete(&mut self, filename: &str) -> Result<DeleteResult, String> {
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
}
