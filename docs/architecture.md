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

The outer code (RS strand-level erasure recovery) is implemented. The inner code (per-strand error repair) is not yet implemented — the ternary decoder is strict and rejects noise-corrupted strands rather than attempting soft decoding. `nucle pipeline` with Illumina noise surfaces this: substitution errors introduce homopolymers that the rotating cipher decoder cannot tolerate. A consensus voting layer across coverage copies is the standard fix (implemented in `nucle_ecc::consensus` but not yet wired into the decode pipeline).

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
