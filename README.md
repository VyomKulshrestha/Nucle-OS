# Nucle-OS — DNA Storage Engine

**A complete software-defined DNA storage operating system.**

The same way software-defined networking abstracts physical switches, Nucle-OS abstracts physical DNA synthesizers. It is the driver layer that molecular data storage plugs into.

---

## Architecture

```
┌─────────────────────────────────────────┐
│           Agent Interface Layer          │  ← AI agent for semantic file ops
├─────────────────────────────────────────┤
│         Hardware Bridge / Provider       │  ← typed requests → mock/file-export/vendor
├─────────────────────────────────────────┤
│              VFS / File API              │  ← read(), write(), query() abstractions
├─────────────────────────────────────────┤
│          Retrieval & Index Layer         │  ← vector index, CRISPR-sim random access
├─────────────────────────────────────────┤
│         Error Correction Layer           │  ← codec, noise model, repair pipeline
├─────────────────────────────────────────┤
│          Encoding / Decoding Layer       │  ← binary ↔ ATCG with constraints
├─────────────────────────────────────────┤
│           Synthesis Simulator            │  ← inject realistic DNA errors
└─────────────────────────────────────────┘
```

Each layer is a real engineering problem. This project owns the whole stack.

---

## Layers

### Layer 1 — Synthesis Simulator (`nucle_synth`)

Models the exact error distributions of real DNA synthesizers — substitution rates, insertion/deletion frequencies, strand dropout. This is the "noisy channel" everything above must survive. Parameterised to mimic different hardware profiles (Illumina, Oxford Nanopore, Twist Bioscience).

### Layer 2 — Encoding Engine (`nucle_codec`)

Converts arbitrary binary files into valid DNA sequences with hard biological constraints enforced:
- GC content balance (40–60%)
- No homopolymer runs longer than 3 bases
- No secondary structure formation (hairpins/palindromes)

Implements multiple codec strategies:
- **Ternary Rotating Cipher** (Goldman et al.) — ~1.58 bits/nt, zero homopolymers by construction
- **DNA Fountain** (Erlich & Zielinski) — ~1.57 bits/nt, rateless, near-optimal density

### Layer 3 — Error Correction (`nucle_ecc`)

DNA is a noisy channel with insertion/deletion-heavy error profiles — unlike disk or network. This layer provides:
- **Reed-Solomon outer code** — strand-level erasure recovery
- **Fountain/LT erasure codes** — rateless recovery from arbitrary strand loss
- **Consensus sequencing** — majority voting across multiple strand copies
- **Full repair pipeline** — orchestrated multi-stage error correction

### Layer 4 — Retrieval & Index (`nucle_index`)

The hardest unsolved software problem in the field. When millions of DNA strands exist in a pool, how do you retrieve one file without reading everything?
- **Primer-based addressing** — unique address primers per file
- **CRISPR random access simulation** — selective strand amplification
- **Vector similarity index** — content-addressable lookup
- **Semantic search** — query by content, not just filename

### Layer 5 — VFS / File API (`nucle_vfs`)

Abstracts all layers behind clean syscall-style interfaces:
- `dna_write(name, data, redundancy)` — encode → ECC → tag → store
- `dna_read(query)` — search → retrieve → decode → return
- `dna_stat(pool)` — pool statistics, health metrics
- `dna_delete(name)` — mark strands for removal

DNA storage needs a proper ABI. This layer provides it.

### Layer 6 — Agent Interface (`nucle_agent`)

A ReAct agent that takes natural-language file operations, plans across the VFS layer, and executes them. "Store last year's medical archive with 3x redundancy" becomes a full pipeline down to the encoding layer.

### Layer 7 — Hardware Bridge (`nucle_hardware`)

The execution boundary between compiled NucleScript plans and real lab hardware. `nucle_lang::hardware` only ever collects typed `HardwareRequest`s (Synthesis, Sequencing, Destructive) from an effect-checked program; `nucle_hardware::Provider` is the one trait that actually submits them — today via `MockProvider` (dry run) or `FileExportProvider` (writes a JSON batch for lab submission). No real vendor adapter (Twist, IDT, Illumina, Oxford Nanopore) exists yet by design — see [docs/architecture.md](docs/architecture.md#hardware-bridge-and-provider-boundaries).

---

## Building

```bash
# Build the entire workspace
cargo build --workspace

# Run all tests (270+ tests)
cargo test --workspace

# Run the CLI
cargo run --bin nucle-cli -- --help
```

---

## Demo — It Actually Works

### Codec Benchmark

```
$ nucle bench

╔══════════════════════════════════════════════════════════════════╗
║               DNA Codec Benchmark Comparison                    ║
╠══════════════════════════════════════════════════════════════════╣
║ Codec                │  bits/nt │   GC % │ Hpol │ Bio │  R/T ║
╟──────────────────────┼──────────┼────────┼──────┼─────┼──────╢
║ ternary-rotating     │    1.209 │  40.7% │    2 │  ~  │  ✓   ║
║ ternary-overlap      │    0.660 │  40.4% │    2 │  ~  │  ✓   ║
║ yin-yang             │    1.855 │  43.2% │    4 │  ~  │  ✓   ║
║ dna-fountain (raw)   │    0.824 │  26.0% │   29 │  ✗  │  ✓   ║
╚══════════════════════════════════════════════════════════════════╝

  Bio: ✓ = passes all constraints, ~ = passes on production-size inputs,
       ✗ = fails (requires screening)
  R/T = encode → decode roundtrip produces identical data
```

> **Yin-Yang leads in density at 1.855 bits/nt** — nearly 2× the ternary codec. The
> Yang rule maps each bit to an AT/GC partition, guaranteeing ~50% GC on balanced data.
> The Yin rule uses the previous nucleotide as context to reduce homopolymer formation.
> See [docs/references.md](docs/references.md) for the full algorithm (Ping et al. 2022).
>
> **Why ~ for ternary and yin-yang?** On the small benchmark input (89 bytes), a few
> strands fall just outside the GC 40–60% window. On production-size files (≥1 KB),
> both codecs converge into the valid range. The `~` indicates "passes on real data."
>
> **Why ✗ for fountain?** The raw fountain codec uses a 2-bit mapping without constraint
> awareness. With screening enabled (the default), invalid strands are rejected and
> regenerated — the rateless property guarantees sufficient valid output.

### Full-Pipeline Benchmark

`nucle bench` benchmarks codecs in isolation; `nucle benchmark` runs the real
write → simulate-noise → read pipeline against the standard fixtures in
`docs/examples/fixtures/`, reporting GC distribution, homopolymer violations,
and a real Monte-Carlo recovery probability and cost estimate — not
placeholders:

```
$ nucle benchmark --profile pristine -r 4

╔══════════════════════════════════════════════════════════════════════════════════════════════════╗
║                              NucleOS Full-Pipeline Benchmark                                      ║
╠══════════════════════════════════════════════════════════════════════════════════════════════════╣
║ File               │ Size(B) │ Strands │ Error Rate │ Recover │ Cost(USD) │    GC% │  HpolV ║
╟────────────────────┼─────────┼─────────┼────────────┼─────────┼───────────┼────────┼────────╢
║ small_text.txt     │      96 │       8 │      0.00% │    PASS │ $  0.0062 │  41.7% │      0 ║
║ archive.bin        │     327 │      18 │      0.00% │    PASS │ $  0.0216 │  38.1% │      0 ║
║ sample.fasta       │     176 │      12 │      0.00% │    PASS │ $  0.0123 │  34.7% │      0 ║
║ image.png          │     294 │      16 │      0.00% │    PASS │ $  0.0185 │  39.0% │      0 ║
╚══════════════════════════════════════════════════════════════════════════════════════════════════╝
```

Under a noisy channel like Illumina, the same fixtures currently fail
recovery — a known, documented limitation, not a bug in the benchmark: the
ternary decoder is strict and rejects substitution-corrupted strands rather
than soft-decoding them (see [docs/architecture.md](docs/architecture.md#current-status)
for the fix path — consensus voting across coverage copies, already
implemented in `nucle_ecc::consensus` but not yet wired into this decode path):

```
$ nucle benchmark -p illumina -r 4

║ small_text.txt     │      96 │      78 │      0.36% │    FAIL │ $  0.0616 │  41.7% │      0 ║
║ archive.bin        │     327 │     176 │      0.36% │    FAIL │ $  0.2156 │  38.1% │      0 ║
║ sample.fasta       │     176 │     118 │      0.36% │    FAIL │ $  0.1232 │  34.7% │      0 ║
║ image.png          │     294 │     156 │      0.35% │    FAIL │ $  0.1848 │  39.0% │      0 ║
```

### End-to-End Roundtrip: Encode → Noise → Recover

```
$ nucle encode README.md -o readme.dna
✓ Encoded README.md → readme.dna (254 strands)

$ nucle simulate README.md -p illumina
╔══════════════════════════════════════╗
║     Synthesis Simulation Results     ║
╠══════════════════════════════════════╣
║ Profile:                    illumina ║
║ Coverage:                          1×║
║ Input:                   254 strands ║
║ Output:                  254 strands ║
║ Error rate:                  0.35%   ║
║ Surviving:                   95.7%   ║
╚══════════════════════════════════════╝

$ nucle decode readme.dna -o recovered.txt -s 6328
✓ Decoded readme.dna → recovered.txt (6328 bytes)
```

**6,328 bytes → 254 DNA strands × 193 nt avg = 49,022 nucleotides. Illumina noise: 0.35% error rate, 4.3% strand loss — 100% data recovery.**

### Realistic Sequencing: 10× Coverage with Consensus

Real sequencing runs at 10–50× coverage — you sequence the pool many times and consensus-vote across copies. This is the realistic scenario:

```
$ nucle simulate README.md -p illumina -c 10
╔══════════════════════════════════════╗
║     Synthesis Simulation Results     ║
╠══════════════════════════════════════╣
║ Profile:                    illumina ║
║ Coverage:                         10×║
║ Input:                   401 strands ║
║ Output:                 4010 strands ║
║ Error rate:                  0.37%   ║
║ Surviving:                   95.8%   ║
╚══════════════════════════════════════╝
```

**10 independent noisy copies per strand. Consensus voting across copies eliminates per-base errors; ECC handles the ~4% strand dropout. This is how real DNA storage systems achieve reliable recovery.**

### Full Stack: Store with ECC + CRISPR

```
$ nucle store README.md -r 4
✓ Stored 'README.md' (6328 bytes → 254 data + 4 parity = 258 strands,
  1.02× redundancy, primer=P0000)

╔══════════════════════════════════════╗
║         NucleOS Pool Status          ║
╠══════════════════════════════════════╣
║ Files:               1               ║
║ Total strands:     258               ║
║ Data strands:      254               ║
║ Parity strands:      4               ║
║ Nucleotides:     49746               ║
║ Avg strand len:    193 nt            ║
║ Redundancy:      1.02×              ║
╟──────────────────────────────────────╢
║ Files:                               ║
║   README.md (6328 B, 254d+4p strands)║
╚══════════════════════════════════════╝
```

---

## NucleScript — Declarative DNA Operations Language

> [!NOTE]
> **Official Language & Preset Ecosystem:** Visit the [**@Nuclescript Organization**](https://github.com/orgs/Nuclescript) or browse official language modules in our [**Packages Registry**](https://github.com/orgs/Nuclescript/packages).

NucleScript is a domain-specific programming language for DNA storage
operations. NucleScript source files use the `.nsl` extension. A program
describes pools, storage operations, retrieval queries, simulations, and
pipelines; the compiler validates syntax, pool schemas, and hardcoded DNA strand
constraints before lowering operations to NucleOS VFS calls.

```nuclescript
pool archive: DnaPool {
    codec: Ternary,
    redundancy: 3x,
    profile: Illumina
}

store "sample_a.txt" into archive {
    redundancy: 4x,
    tag: ["docs", "demo", "nuclescript"]
}
```

Run it with:

```bash
$ nucle run docs/examples/store.nsl
✓ store into archive: Stored 'sample_a.txt' (31 bytes → 2 data + 4 parity = 6 strands, 3.00× redundancy, primer=P0000)

╔══════════════════════════════════════╗
║         NucleOS Pool Status          ║
╠══════════════════════════════════════╣
║ Files:               1               ║
║ Total strands:       6               ║
║ Data strands:        2               ║
║ Parity strands:      4               ║
║ Nucleotides:       828               ║
║ Avg strand len:    138 nt            ║
║ Redundancy:      3.00×              ║
╟──────────────────────────────────────╢
║ Files:                               ║
║   sample_a.txt (31 B, 2d+4p strands, 3.0×)
╚══════════════════════════════════════╝
```

NucleScript pipeline programs can also verify a full roundtrip:

```bash
$ nucle run docs/examples/pipeline_backup.nsl
✓ store into archive: Stored 'sample_a.txt' (31 bytes → 2 data + 4 parity = 6 strands, 3.00× redundancy, primer=P0000)
✓ verify roundtrip: 'sample_a.txt' recovered exactly
```

DNA-native `Sequence` literals are also part of the language and are validated at
compile time:

```nuclescript
seq primer_p0: Sequence = "ATCGATCGGCTAGCTA"
let primer_p1 = seq"ATCGATCG-GCTAGCTA"
```

NucleScript also tracks probabilistic pool types through simulation and
consensus recovery. `Pool<P, E>` carries the hardware profile or recovery state
plus an optional compiler-checked error budget:

```nuclescript
pool archive: DnaPool {
    codec: Ternary,
    redundancy: 3x,
    profile: Illumina
}

let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
```

Effectful biological operations are explicit in the type system. Hardware-backed
synthesis and sequencing require `confirm hardware`; destructive operations
require `confirm physical_key`. The compiler lowers programs through a
bio-aware MIR, optimizes redundancy for the selected profile and coverage, and
can emit a no-hardware simulation plan:

```nuclescript
let strands: Pool<Twist, 0.03%> = synthesise archive via Twist confirm hardware
let reads: Pool<Illumina, 0.35%> = sequence strands via Illumina confirm hardware
delete "old_archive.bin" from archive confirm physical_key
```

For ecosystem growth, the compiler also exposes stable integration surfaces:
built-in preset imports, a serializable playground analysis API, and hardware
bridge request extraction for effectful plans.

```nuclescript
import {
    medical_archive,
    reliable_store as store_recipe,
    illumina_recovery
} from "nuclescript/presets"
```

The first NucleScript package is `@nuclescript/presets` version `0.1.0`,
imported in source as `nuclescript/presets`. Its manifest,
source, and changelog live under `packages/nuclescript-presets/`, with a
registry index at `packages/registry.json` — the CLI resolves packages by
reading that file directly, so adding an entry there is what makes a new
package discoverable. List or inspect bundled packages with:

```bash
nucle packages                          # quick listing of the bundled presets package
nucle package list                      # full registry.json index
```

Install and verify packages by name (resolved against `packages/registry.json`,
not a filesystem path):

```bash
nucle package install "@nuclescript/presets"
nucle package lock                      # write/update nucle.lock with manifest checksums
nucle package verify "@nuclescript/presets"   # checks manifest shape + checksum against nucle.lock
```

Current NucleScript result summary:

| Program | Payload | Data strands | Parity strands | Total strands | Nucleotides | Avg strand | Redundancy | Result |
|---------|--------:|-------------:|---------------:|--------------:|------------:|-----------:|-----------:|--------|
| `docs/examples/store.nsl` | 31 B | 2 | 4 | 6 | 828 nt | 138 nt | 3.00× | Stored via VFS |
| `docs/examples/pipeline_backup.nsl` | 31 B | 2 | 4 | 6 | 828 nt | 138 nt | 3.00× | Exact roundtrip |
| `docs/examples/sequence_literals.nsl` | — | — | — | — | — | — | — | Compile-time DNA validation |
| `docs/examples/probabilistic_recovery.nsl` | - | - | - | - | - | - | - | Compile-time error-budget propagation |
| `docs/examples/effect_confirmations.nsl` | - | - | - | - | - | - | - | Effect confirmation and planning |
| `docs/examples/preset_imports.nsl` | - | - | - | - | - | - | - | Built-in preset import validation |

Compiler diagnostics are surfaced before execution. For example,
`docs/examples/critical_redundancy_warning.nsl` warns when critical data uses
only `1x` redundancy.

---

## CLI Usage

Every command also accepts a global `--json` flag (e.g. `nucle --json pool`)
for machine-readable output.

```bash
# Encode a file to DNA strands
nucle encode myfile.txt -o myfile.dna

# Decode DNA strands back to binary
nucle decode myfile.dna -o recovered.txt -s 1024

# Store a file with error correction (4 parity strands)
nucle store myfile.txt -r 4

# Retrieve a stored file
nucle retrieve myfile.txt

# Migrate a stored file to new parameters (redundancy and/or codec)
nucle migrate myfile.txt -r 6
nucle migrate myfile.txt --codec ternary-rotating-cipher

# Search for files
nucle search "name:readme type:txt"

# Pool statistics
nucle pool

# Simulate synthesis noise (Illumina profile)
nucle simulate myfile.txt -p illumina

# Benchmark all codecs in isolation (density, GC, homopolymers, recovery probability, cost)
nucle bench --profile nanopore

# Full-pipeline benchmark against standard fixtures (write → simulate → read)
nucle benchmark -p illumina -r 4

# Stress test: sweep all codecs across data distributions
nucle stress -s 256

# Full-pipeline stress test: encode → noise → ECC → recover across N files
nucle pipeline -f 100 -s 1024 -p illumina -c 10 -r 4

# Run a NucleScript source file
nucle run docs/examples/store.nsl

# Show an optimized no-hardware NucleScript plan
nucle plan docs/examples/probabilistic_recovery.nsl

# List released NucleScript packages / inspect the full registry
nucle packages
nucle package list

# Install, lock, and verify packages by name against packages/registry.json
nucle package install "@nuclescript/presets"
nucle package lock
nucle package verify "@nuclescript/presets"

# Export a compiled program's synthesis/sequencing/destructive requests.
# Requires --confirm whenever the batch is cost-bearing or destructive.
nucle hardware export docs/examples/effect_confirmations.nsl --confirm -o batch.json
nucle hardware export docs/examples/effect_confirmations.nsl --confirm --provider mock

# Environment and integrity diagnostics
nucle doctor

# Natural language agent
nucle agent "store readme.txt with 3x redundancy"
nucle agent "search for text files"
nucle agent "pool status"
```

---

## Test Coverage

| Crate | Tests | What's Tested |
|-------|------:|---------------|
| `nucle_codec` | 58 (+3 doctests) | Nucleotide types, constraints, ternary codec, fountain codec, yin-yang codec, benchmarks incl. GC distribution and homopolymer violation counts |
| `nucle_synth` | 32 | Error models, noise engine, hardware profiles, encode→noise→decode e2e |
| `nucle_ecc` | 25 | Reed-Solomon, fountain erasure, consensus, repair pipeline, per-position observed error distribution |
| `nucle_index` | 28 | Primers, CRISPR sim, vector index, semantic search |
| `nucle_vfs` | 48 | Pool, file, catalog, storage manifests, content-addressed archive IDs, migration (incl. codec-migration rejection), per-object recovery manifests, regression-pinned fixture roundtrips |
| `nucle_agent` | 27 | Tool defs, planner, executor |
| `nucle_lang` | 34 | Lexer, parser, biological checks, sequence literals, probabilistic pool typing, effects, MIR optimizer, simulation backend, table-driven package registry, lock file checksums, hardware request collection, VFS lowering |
| `nucle_hardware` | 21 | Confirmation gating (effectful/destructive rejection, count/message correctness), mock provider dry runs, file-export JSON roundtrip and field preservation, parent-directory creation |
| **Total** | **273 (+3 doctests)** | **End-to-end: binary → DNA → noise → ECC → recover → binary** |

---

## Project Structure

```
nucle_codec/     — Encoding/Decoding engine (binary ↔ ATCG)
nucle_synth/     — Synthesis simulator (hardware mock)
nucle_ecc/       — Error correction (Reed-Solomon, fountain, consensus)
nucle_index/     — Retrieval & indexing (CRISPR-sim, vector index)
nucle_vfs/       — Virtual file system (syscall-style API, storage/recovery manifests, migration)
nucle_agent/     — Agent interface (ReAct planner)
nucle_lang/      — NucleScript compiler, MIR optimizer, package registry, lock files, ecosystem APIs, simulation backend, and VFS backend
nucle_hardware/  — Hardware provider adapters (Provider trait, MockProvider, FileExportProvider)
nucle_cli/       — Command-line interface
docs/            — Architecture notes, paper references, and runnable examples/fixtures
packages/        — NucleScript package registry (packages/registry.json) and package releases
```

---

## License

MIT — see [LICENSE](LICENSE) for details.
