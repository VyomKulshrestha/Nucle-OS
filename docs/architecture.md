# Nucle-OS Architecture

## Design Philosophy

Nucle-OS follows the same bottom-up layered architecture as FerrumOS(https://github.com/VyomKulshrestha/Ferrum-OS). Each layer is a separate Rust crate with well-defined responsibilities and clean interfaces to adjacent layers. Dependencies flow strictly upward — lower layers never depend on higher layers.

## Layer Dependency Graph

```
nucle_cli
    ├── nucle_agent
    │       └── nucle_vfs
    │               └── nucle_index
    │                       └── nucle_ecc
    │                               ├── nucle_codec
    │                               └── nucle_synth
    │                                       └── nucle_codec
    ├── nucle_hardware
    │       └── nucle_lang (below)
    └── nucle_lang
            ├── nucle_vfs
            │       └── nucle_index
            │               └── nucle_ecc
            │                       ├── nucle_codec
            │                       └── nucle_synth
            │                               └── nucle_codec
            ├── nucle_codec
            └── nucle_synth
```

Dependencies flow strictly downward. No layer ever imports from a layer above it.

## NucleScript Language Layer

`nucle_lang` is the NucleScript compiler crate. It sits above the VFS and turns
`.nsl` source files into NucleOS operations:

```text
NucleScript source (.nsl)
    → lexer
    → parser / AST
    → semantic + biological constraint checks
    → bio-aware MIR
    → redundancy/profile optimizer
    → VFS backend or simulation backend
    → NucleOS syscalls or no-hardware plan
```

The compiler currently supports declarative pool definitions, store/retrieve
operations, simulation options, pipeline programs, DNA-native `Sequence`
literals such as `seq"ATCGATCG-GCTAGCTA"`, and probabilistic pool annotations
such as `Pool<Illumina, 0.35%>`. Sequence literals are validated at compile
time for DNA alphabet, GC balance, homopolymer length, and hairpin-prone
palindromes. Probabilistic pool bindings are checked for profile/state
compatibility and propagate an error budget through consensus recovery.
Effect checking classifies operations as `Pure`, `Synthesis`, `Sequencing`, or
`Destructive`; hardware effects require `confirm hardware`, and destructive
effects require `confirm physical_key`. The MIR optimizer raises insufficient
redundancy for the selected profile and coverage before either executable VFS
lowering or no-hardware simulation planning.

The language layer now exposes ecosystem-facing integration points:

- `import { ... } from "nuclescript/presets"` validates built-in presets with
  the same resolver shape a package registry can extend.
- `analyze_source` returns serializable diagnostics, optimizer notes, simulation
  steps, and VFS call counts for browser playgrounds.
- `collect_hardware_requests` extracts synthesis, sequencing, and destructive
  requests from effectful MIR so a hardware bridge can submit them without
  changing NucleScript source syntax.

> See [docs/grammar.md](grammar.md) for the full formal syntax reference and
> [docs/effects.md](effects.md) for the effect model — including how effects
> propagate through function calls, not just literal operations.

## NucleScript Playground

The interactive playground has three tabs, each backed by the real engine rather than reimplemented or mocked logic, and ships two ways from the same source:

- **`nucle_wasm`** compiles the compiler/codec/ECC/noise engine to `wasm32-unknown-unknown` via `wasm-bindgen` and runs it entirely client-side — no server at all. Live at [nuclescript.github.io/playground](https://nuclescript.github.io/playground/), rebuilt and redeployed to GitHub Pages by `Nuclescript/playground`'s `.github/workflows/pages.yml` on every push.
- **`nucle_playground`** is a thin `tiny_http` HTTP server exposing the same three operations over `POST /analyze`, `/benchmark`, `/pipeline-demo`, for anyone who wants a native binary instead of a browser tab.

Both frontends call into the exact same logic — `nucle_wasm` and `nucle_playground` both depend on `nucle_demo_core` (a pure, I/O-free crate holding the benchmark and pipeline-visualizer implementations) plus `nucle_lang::playground::analyze_source` for the Write & Run tab, so there is one implementation to keep correct, not two that can drift.

- **Write & Run**: returns `PlaygroundReport` as JSON containing compiler diagnostics, simulator steps, and optimizer notes — the same `analyze_source` API `nucle check --json` uses.
- **Benchmark Explorer**: accepts `{ codec, profile, redundancy, data }` and returns density, GC distribution, homopolymer violations, and an estimated cost — all from `nucle_codec::benchmark` — plus a `recovery_probability` computed by actually running Reed-Solomon parity + `NoiseEngine` simulation + decode across 20 trials. The frontend debounces control changes (codec/profile/redundancy sliders) and re-runs live.
- **Pipeline Visualizer**: encodes real input via `TernaryCodec`, adds RS parity, runs it through `NoiseEngine`, and returns per-strand before/after sequences plus drop/corruption flags so the frontend can animate encode → noise → recovery. Recovery is attempted for real (RS-decode using surviving strands as input, then codec-decode) — a failure shown in the UI is a genuine failure of the current pipeline at that redundancy/profile, not a scripted outcome.
- **Frontend**: A single glassmorphic dark-themed page with tab navigation between the three modes; plain HTML/JS, no build tooling. The WASM build's copy (`nucle_wasm/www/index.html`) calls straight into WASM functions instead of `fetch()`-ing a server, but is otherwise the same UI.
- **`wasm32` portability note**: `std::time::Instant`/`SystemTime` panic unconditionally on `wasm32-unknown-unknown` ("time not implemented on this platform"). `nucle_codec::benchmark` uses the [`web-time`](https://docs.rs/web-time) crate instead — a drop-in replacement that re-exports `std::time` unchanged on every other target and backs it with `Performance.now()` in the browser — so the same timing code works natively and in WASM.
- **Published standalone**: A self-contained snapshot of this workspace (verified to build independently from a fresh clone) is published at [github.com/Nuclescript/playground](https://github.com/Nuclescript/playground). For zero setup, prebuilt Linux/Windows/macOS binaries of `nucle_playground` (frontend embedded via `include_str!`, no external files needed) are published on that repo's [Releases page](https://github.com/Nuclescript/playground/releases) via a tag-triggered GitHub Actions workflow — free to run and host, since public-repo Actions minutes, Pages hosting, and Release storage all have no cost.

## Biological Constraints

All encoding must satisfy hard constraints imposed by DNA chemistry:

| Constraint | Value | Reason |
|-----------|-------|--------|
| GC Content | 40–60% | Synthesis fidelity, PCR amplification balance |
| Homopolymer max | 3 bases | Sequencing accuracy (especially Nanopore) |
| Secondary structure | No palindromes > 6 nt | Prevents hairpin formation during PCR |
| Strand length | 150–200 nt typical | Synthesis yield vs. data density tradeoff |

## Error Channel Model

DNA storage has a unique error profile unlike any digital channel:

| Error Type | Synthesis (Twist) | Illumina Seq | Nanopore Seq |
|-----------|-------------------|-------------|-------------|
| Substitution | ~0.01% | ~0.1% | ~3-5% |
| Insertion | ~0.005% | ~0.01% | ~2-5% |
| Deletion | ~0.02% | ~0.01% | ~2-5% |
| Strand dropout | 0.5-5% | — | — |

## Codec Strategies

### Ternary Rotating Cipher (Goldman et al., 2013)
- Converts binary → base-3 (ternary)
- Rotating mapping rule eliminates all homopolymers by construction
- Overlapping segments provide natural redundancy
- Effective density: ~1.58 bits/nucleotide

### DNA Fountain (Erlich & Zielinski, 2017)
- Luby Transform (LT) codes applied to DNA storage
- Rateless: can generate unlimited encoded strands
- Built-in screening rejects constraint-violating strands
- Near-optimal density: ~1.57 bits/nucleotide
- Natural erasure resilience — any sufficient subset of strands reconstructs data

### Yin-Yang Codec (Ping et al., 2022)
- Two complementary mapping rules achieve GC balance by construction
- Yang rule: `0 → {A,T}`, `1 → {C,G}` — structural 50% GC guarantee
- Yin rule: context-dependent (previous base) mapping reduces homopolymers
- Highest density: 2.0 bits/nucleotide theoretical, ~1.85 effective
- Best suited for real-world data with natural bit entropy

## Error Correction Architecture

Two-layer coding scheme (industry standard):

1. **Inner code** (per-strand): Handles substitutions, insertions, deletions within individual strands
2. **Outer code** (cross-strand): Handles strand dropouts and residual errors

```
Data → [Outer RS/Fountain] → [Segmentation] → [Inner encoding] → [Constraint screening] → DNA
DNA → [Basecalling] → [Clustering] → [Consensus] → [Inner decode] → [Outer decode] → Data
```

### Current status

The outer code (RS strand-level erasure recovery) and the consensus stage are both implemented and wired together in `dna_read`. The ternary decoder is still strict — it rejects a noise-corrupted strand rather than attempting soft decoding, and RS alone only recovers a strand that's entirely missing, never one that survived corrupted — but neither of those strands ever reaches the decoder directly anymore: `dna_write` records which stored strands are coverage copies of the same logical strand (via `PoolEntry::source_index`), and `dna_read` groups them and runs `nucle_ecc::consensus::build_consensus` per group before RS decode, correcting substitution errors regardless of which individual copy has them. A logical strand with zero surviving copies still becomes an erasure for RS, same as before. This requires actual sequencing coverage (`coverage_depth > 1` in `SimulationConfig`) to have multiple independent reads to vote across — a single read has nothing to vote against.

**Illumina works. Nanopore still doesn't, and the diagnosis moved twice as we kept digging — worth documenting precisely rather than leaving an earlier explanation stale.** First diagnosis: `build_consensus` voted by raw position, which breaks under indels. Fixed by aligning each read to the reference before voting (Needleman-Wunsch). Second, bigger diagnosis: `nucle_index::primer::PrimerPair::{matches_forward, untag_strand}` matched primers by exact position, so a single indel landing inside a primer — routine at Nanopore's ~4%/base indel rate — made CRISPR retrieval drop the whole strand before it ever reached consensus; this, not the voting algorithm, was the dominant blocker. Fixed via bounded edit-distance boundary search (`nucle_index::primer::tests::test_untag_tolerates_*`).

Third: pairwise realignment against one arbitrarily-picked noisy reference read turned out to have a hard ceiling once a single read carries several simultaneous indels (the realistic Nanopore regime for a 150+nt strand) — the reference's own errors and each read's individual drift compounded into wrong votes scattered across the strand. Fixed by replacing pairwise realignment with genuine **partial-order alignment (POA)**: `nucle_ecc::consensus::PoaGraph` folds every read into one shared DAG instead of comparing each to a single anchor, with edge weights (not just node visitation) so a majority "don't insert here" can correctly outvote a minority stray insertion at any position, including the very first or last base (`nucle_ecc::consensus::tests::test_boundary_insertion_outvoted_by_clean_majority`, `test_consensus_corrects_frame_shifting_indels`). Getting this right took real trial and error: a scoring tie let plain substitution runs get spuriously realigned, "maximize total nodes visited" always preferred a detour over stopping short of it, and node identity based on predecessor-set equality could alias two genuinely different reference positions together once enough reads had passed through — each is now a dedicated regression test. The graph is also fuzz-tested against realistic Nanopore error rates at 50x coverage for crash-safety (`test_high_coverage_realistic_nanopore_fuzz_does_not_crash`): a self-loop could otherwise form via the exact-base-match fast path, which the sibling-reuse cycle checks didn't cover, so the cycle check now lives in the single choke point every edge passes through (`PoaGraph::add_pred_if_missing`) instead of being re-derived per call site.

With all of that fixed, `build_consensus` also runs multi-round polishing now, not just a single POA pass: after the first pass, it reseeds a fresh graph from that pass's own (already-corrected) result — unweighted, so the backbone doesn't get double-counted as an extra vote — and re-folds every read, repeating to a fixed point, the same iterative approach real long-read polishers (Racon, Medaka) use. Getting this to *actually* be safe took a real caught-and-fixed regression of its own: the first attempt at it briefly broke the working Illumina case, from double-counting a read's vote once via the unweighted-seed omission being missing and once via the fold — fixed by `PoaGraph::seed_unweighted`, which is exactly the fix, not a workaround to avoid polishing. Polishing is verified safe (full workspace suite green, including Illumina) and does measurably help.

A synthetic 30-read stress test (3-6 edits each over a 43nt sequence — a higher combined edit rate than Nanopore's own ~7%) still landed 1 base off even after polishing converged, and the *first* diagnosis for that — "column identity occasionally fragmenting near a compounding cluster of edits" — turned out to be wrong once actually tested. The real cause: sequential POA construction is fold-order dependent. Whichever read gets folded into the graph first shapes which alternate nodes exist at all, and a later read can "snap" onto an early alternate for loosely-related reasons even when its own content doesn't really support it, letting an accidental fold-order majority snowball around the wrong interpretation at a position where several reads' unrelated edits happen to cluster. Polishing alone can't fix this, because every round reuses the same fold order — confirmed directly by folding the exact same reads in reverse order and getting the exactly correct answer instead, with no other change. `build_consensus` now re-runs the pipeline with a second (reversed) and, if needed, a third (rotated) fold order, taking whichever result at least two of three orderings agree on. This fully resolves the stress test (exact match, not just "close"; see `nucle_ecc::consensus::tests::test_consensus_exactly_recovers_original_under_many_simultaneous_indels_per_read`), and it's gated on the primary pass's own weakest per-position confidence (`< 90%`) so the extra orderings only run when there's a real reason to doubt the first answer — a clean, high-agreement result at realistic Illumina-grade noise returns after one pass, unchanged in cost.

Nanopore's own full end-to-end recovery still failed at realistic settings even with all of that, and chasing why surfaced a bug that wasn't in the consensus algorithm at all: `nucle_codec::ternary::TernaryCodec` pads unused strand length with a *constant* trit, and its 4-byte length header has several leading zero bytes for any file under 16MB. The rotating cipher maps a constant trit to a constant rotation relative to whatever base precedes it, so a long run of one trit value degenerates into a short, fixed-period base cycle once it goes through the cipher — a run of trit `0` became a literal `TATATATATATATATATATATAT...` repeat, dozens of bases long, at the start of essentially every encoded file. That self-inflicted tandem repeat — nothing to do with noise or the aligner — is exactly the kind of region that's hardest for any consensus scheme to recover under indel noise, and was the real cause of several residual errors that looked, at first glance, like a fundamental POA limitation. Fixed with `TernaryCodec::whiten_segment`: every strand's trits (including its padding) are XOR-added (mod 3) with a deterministic, position-addressable pseudo-random stream before the cipher sees them, and un-whitened per-strand at decode using that strand's own known index — position-addressable rather than a sequential-state PRNG specifically because overlapping segments see the same absolute trit position more than once, and decode doesn't know a strand's real-data/padding boundary until after decoding its length header, so unwhitening has to work without needing that boundary.

Verified: the ground-truth encoded strands for the `nucle_vfs` Nanopore tests no longer contain any long repeats (previously up to 140+ characters of a 2-base alternation), and residual consensus errors under real Nanopore noise are now small, localized 1-2-base insertions rather than sprawling repeat-region corruption. At real Nanopore noise density nearly every position is genuinely contested, so the confidence gate rarely skips the extra fold orderings — a real, non-hidden compute cost that buys real correctness where it's needed.

That narrowed the remaining gap to "Reed-Solomon can't correct a strand that survives consensus wrong-but-present rather than missing" — but Reed-Solomon itself turned out to have two real, previously undiscovered bugs of its own, unrelated to consensus or the codec. First: parity symbols are arbitrary GF(256) values spanning the full 0-255 range, but `dna_write` packed each one into DNA via the same 2-bit `Nucleotide::from_bits` used for data strand bytes (which are always pre-restricted to 0-3) — any parity byte outside that range was silently dropped, destroying the overwhelming majority of every parity strand's content without ever raising an error. Second: `consensus_then_rs_decode`'s parity conversion collapsed a strand that failed consensus out of the array entirely (`filter_map`) rather than leaving a gap at its position, which shifted every later parity strand onto the wrong evaluation point (`x = k + j` used the strand's position in the surviving subset, not its true configured index) and corrupted the whole stripe regardless of how many strands were actually wrong. Fixed by packing each parity byte into 4 bases (`Nucleotide::byte_to_bases`/`bases_to_byte`, `DnaStrand::from_packed_bytes`/`unpack_bytes`) and representing every erasure — data or parity — as `Option`-per-slot end to end, so a missing strand's true codeword position is never lost (`reed_solomon::tests::test_rs_parity_reindexing_does_not_corrupt_decode`).

With those two bugs fixed, `ReedSolomon::decode_stripe` was rewritten from plain erasure-only Lagrange interpolation to genuine combined error-and-erasure decoding via the Berlekamp-Welch algorithm (`GF256::solve_linear_system`, `GF256::poly_divmod`, `ReedSolomon::try_welch_decode`): it can now blindly correct up to `parity_count / 2` strands that come back from consensus wrong-but-present, without ever being told which ones, in addition to reconstructing up to `parity_count` known-missing strands — combinable per the standard `2*errors + erasures <= parity_count` bound. Verified directly with dedicated unit tests (`test_rs_corrects_silent_error_without_knowing_position`, `test_rs_combines_erasure_and_blind_error`), and the full workspace suite — including the 50x-coverage Nanopore fuzz test — passes with zero regressions.

Nanopore's full end-to-end recovery still fails at realistic settings even with all of this (`nucle_vfs::tests::test_nanopore_still_fails_at_realistic_indel_density_despite_alignment_fixes`, verified at 50x coverage and 12 parity strands; `nucle benchmark -p nanopore -r 4` and `-r 12` both still report FAIL at the CLI's real defaults, ~14% combined error rate). But this time the diagnosis is pinned down by direct ablation rather than inferred: comparing `-r 0` (consensus only, no Reed-Solomon at all) through `-r 50` (25-strand blind-error tolerance) on the identical noisy input produces the *exact same* decode failure at every redundancy level — proof that Reed-Solomon was never in the critical path for this specific failure, because consensus itself does not reliably converge to the correct sequence at Oxford Nanopore's real per-base error rate, before Reed-Solomon ever gets a chance to help. No amount of outer-code redundancy can fix an error that happens upstream of it. Closing this needs a better consensus/alignment algorithm for extreme indel density — an active research problem, not a parameter to tune — and remains open.

## Retrieval Architecture

```
Query → [Vector Index] → [Primer Resolution] → [CRISPR-sim Amplification] → [Strand Retrieval]
```

- Each file tagged with unique PCR primer pair
- Vector index enables content-addressable lookup
- CRISPR simulation models selective amplification
- Cross-talk modeling accounts for non-specific amplification

## VFS Abstraction

The VFS layer presents DNA storage as a device:

```rust
// Core syscall-style interface
fn dna_write(name: &str, data: &[u8], redundancy: u32) -> Result<FileHandle>;
fn dna_read(query: &str) -> Result<Vec<u8>>;
fn dna_stat(pool: &DnaPool) -> Result<PoolStats>;
fn dna_delete(name: &str) -> Result<()>;
```

### Explicitly out of scope

The VFS is a **session-scoped in-memory abstraction**, not a persistent filesystem. The following are deliberate non-goals for the current design:

- **Persistence across process restarts** — the pool exists only for the lifetime of the `NucleOS` instance. Serialisation to disk is left to the caller.
- **Encryption** — data is stored as plaintext DNA. A production system would add an encryption layer between the VFS and the codec, but that's orthogonal to the storage stack.
- **Access control / permissions** — no user model, no file ownership. Every caller has full read/write to the pool.
- **Concurrent writes** — the pool is single-writer. Concurrent access requires external synchronisation.
- **POSIX semantics** — no directories, no symlinks, no `seek()`. The API is flat key-value: name → blob.

These boundaries are intentional. The VFS owns the question "how do I store and retrieve a named blob in DNA?" — everything else belongs to layers above it.

## Hardware Bridge and Provider Boundaries

The hardware boundary separates the high-level compiler planner from the physical/simulation hardware execution:

```
[NucleScript compilation] → [HardwareRequest batches] → [Provider implementation]
                                                            ├── MockProvider (simulates)
                                                            └── FileExportProvider (JSON export)
```

- **HardwareRequest**: Models a typed transaction representing a physical operation (Synthesis, Sequencing, or Destructive deletion). Lives in `nucle_lang::hardware` — that module only ever defines and collects request *types*; it does not implement an execution trait itself. (An earlier `HardwareBridge` trait duplicated that concern with zero implementations and was removed in favor of `Provider` below, so there is exactly one execution-side trait, not two unrelated ones.)
- **Provider Trait**: The sole execution boundary, defined in `nucle_hardware::provider`. Real vendor adapters (Twist, IDT, Illumina, Oxford Nanopore) would implement it in their own module under `nucle_hardware/src/`, once the request model has been exercised via `MockProvider`/`FileExportProvider` for a while — see the "Deferred" section of the NucleOS action plan for why this is intentionally not done yet.
- **`nucle hardware export`**: The CLI entry point. It first runs the compiler's own effect/confirmation check (`nucle_lang::typeck::check_program`) — a `.nsl` program missing `confirm hardware`/`confirm physical_key` in source is rejected before its requests are ever collected. It then requires an explicit `--confirm` flag whenever the collected batch contains any non-`Pure` effect, as a second, operator-level acknowledgment distinct from the language-level one. `--provider` selects `file-export` (default, writes to `--output`) or `mock` (dry run, nothing persisted); an unrecognized name (e.g. a vendor like `twist`) is accepted but falls back to `file-export` with a printed notice, since no vendor-specific adapter exists yet.

## `nucle doctor`

Environment sanity check, run from the workspace root, so a confusing bug
report can first be ruled out as "the environment isn't what we think it is."
Each check reports pass/fail/skipped independently rather than a single
opaque status:

- **Workspace crate versions** — reads every crate's `Cargo.toml` and checks
  it inherits `version.workspace = true` rather than a hardcoded override
  (the actual mechanism that keeps workspace versions consistent, not a
  runtime comparison of values Cargo already guarantees are equal).
- **Presets package manifest** — runs the same manifest validation
  `nucle package verify` uses (non-empty name/exports, known export kinds).
- **Standard fixtures present** — checks `docs/examples/fixtures/` has the
  expected text/binary/FASTA/image files and the `project_tree/` directory.
- **Example programs parse** — actually lexes and parses every `.nsl` file
  under `docs/examples/` (excluding `failures/`), not just checking they exist.
- **Failure-mode examples parse** — same, but for `docs/examples/failures/`:
  those programs are supposed to fail *type checking* by design, so this
  only asserts they're still syntactically valid NucleScript.
- **VFS write/read roundtrip** — since `NucleOS` holds no on-disk state, this
  runs a real `dna_write`/`dna_read` roundtrip against an ephemeral in-memory
  instance as the VFS pipeline's equivalent of a scratch read/write check.

A check that can't run at all from the current directory (e.g. a directory
genuinely doesn't exist) is reported `skipped`, not `failed` — it degrades
gracefully rather than treating "couldn't check" the same as "checked and
it's broken."
