# Nucle-OS — DNA Storage Engine

[![Release](https://img.shields.io/github/v/release/VyomKulshrestha/Nucle-OS)](https://github.com/VyomKulshrestha/Nucle-OS/releases)

**A complete software-defined DNA storage operating system.**

The same way software-defined networking abstracts physical switches, Nucle-OS abstracts physical DNA synthesizers. It is the driver layer that molecular data storage plugs into.

---

## 15 lines, one command

This is the whole pitch: a pool schema with real biological constraints, a
noise-aware probabilistic recovery type, and a pipeline that encodes,
protects, stores, and cryptographically verifies a real file — end to end,
against the actual engine, not a mock.

```nuclescript
pool medical_archive: DnaPool {
    codec: YinYang,
    redundancy: 4x,
    profile: Illumina
}

let noisy: Pool<Illumina, 0.35%> = simulate medical_archive under Illumina
let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)

pipeline archive_patient_records {
    encode "patient_records_2026.csv" using YinYang,
    protect with redundancy 4x,
    store into medical_archive,
    verify roundtrip
}
```

```
$ nucle run docs/examples/hero.nsl
✓ store into medical_archive: Stored 'patient_records_2026.csv' (109 bytes → 4 data + 4 parity = 8 strands, 2.00× redundancy, primer=P0000)
✓ verify roundtrip: 'patient_records_2026.csv' recovered exactly

╔══════════════════════════════════════╗
║         NucleOS Pool Status          ║
╠══════════════════════════════════════╣
║ Files:               1               ║
║ Total strands:       8               ║
║ Data strands:        4               ║
║ Parity strands:      4               ║
║ Nucleotides:      3204               ║
║ Avg strand len:    400 nt            ║
║ Redundancy:      2.00×              ║
╟──────────────────────────────────────╢
║ Files:                               ║
║   patient_records_2026.csv (ID: archive-7098, 109 B, 4d+4p strands, 2.0×)
╚══════════════════════════════════════╝

--- Recovery Manifest: patient_records_2026.csv ---
Observed Error Rate: 0.0000%
Consensus Method:    majority-vote
Sequencing Profile:  pristine
Recovered Strands:   4
ECC Success:         true
Positions w/ errors: 0 of 4
```

The `pool` declaration is chemistry-checked at compile time (GC balance,
homopolymer limits) before a single strand is generated. The probabilistic
`Pool<Illumina, 0.35%>` type tracks the sequencer's real error rate through
`consensus_vote` — the type system, not a comment, is the proof that noise
was accounted for. And `verify roundtrip` isn't cosmetic: `nucle run` reads
the original file back out through the full encode → protect → store →
decode path and byte-compares it, so `✓ verify roundtrip: recovered exactly`
above is a real assertion that passed, not a printed string. Try it yourself:

```bash
nucle run docs/examples/hero.nsl
```

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

# Run all tests (300+ tests)
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
║ ternary-rotating-cipher │    1.099 │  48.6% │    1 │  ✗  │  ✓   ║
║ ternary-rotating-cipher │    0.628 │  48.9% │    2 │  ✗  │  ✓   ║
║ yin-yang             │    1.798 │  37.8% │    4 │  ✗  │  ✓   ║
║ dna-fountain         │    0.824 │  26.0% │   29 │  ✗  │  ✓   ║
╚══════════════════════════════════════════════════════════════════╝
  Best density:    yin-yang (1.798 bits/nt)
  Fastest encode:  yin-yang (55 μs)

  Bio = all strands pass biological constraints (GC 40–60%, homopolymer ≤ 3)
  R/T = encode → decode roundtrip produces identical data
```

> **Yin-Yang leads in density at 1.798 bits/nt** — nearly 2× the ternary codec. The
> Yang rule maps each bit to an AT/GC partition, guaranteeing ~50% GC on balanced data.
> The Yin rule uses the previous nucleotide as context to reduce homopolymer formation.
> See [docs/references.md](docs/references.md) for the full algorithm (Ping et al. 2022).
>
> The two `ternary-rotating-cipher` rows are the same codec run in its two configurations
> (no overlap, and default overlap) — same name, different density/GC/homopolymer profile.
>
> **Why does every codec show `✗` for Bio here?** The overall GC%/homopolymer columns above
> look fine, but `Bio` also checks two things those columns don't show: *local* GC content in
> a sliding window, and palindromic runs long enough to form a hairpin. On this small 89-byte
> benchmark input, at least one strand from every codec trips one of those two additional
> checks. On production-size files (≥1 KB) this is far less likely, since a single bad local
> window has much less influence on the whole strand set — see
> `docs/examples/fixtures/`-backed results below for real files that do pass.

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
║ small_text.txt     │      96 │       8 │      0.00% │    PASS │ $  0.0065 │  47.5% │      0 ║
║ archive.bin        │     327 │      18 │      0.00% │    PASS │ $  0.0227 │  46.2% │      0 ║
║ sample.fasta       │     176 │      12 │      0.00% │    PASS │ $  0.0130 │  45.3% │      0 ║
║ image.png          │     294 │      16 │      0.00% │    PASS │ $  0.0194 │  47.0% │      0 ║
╚══════════════════════════════════════════════════════════════════════════════════════════════════╝
```

Under a noisy channel like Illumina, this used to fail recovery: the ternary
decoder is strict and rejects substitution-corrupted strands rather than
soft-decoding them, and Reed-Solomon alone only recovers a strand that's
entirely missing, never one that survived corrupted. The fix is consensus
voting across coverage copies — sequencing each strand multiple times and
majority-voting corrects substitution errors regardless of which copy has
them — and it's now wired into the real `dna_read` path (`nucle_ecc::consensus`
→ `nucle_vfs::syscall::dna_read`), not just implemented in isolation:

```
$ nucle benchmark -p illumina -r 4

║ small_text.txt     │      96 │       8 │      0.32% │    PASS │ $  0.0648 │  47.5% │      0 ║
║ archive.bin        │     327 │      18 │      0.32% │    PASS │ $  0.2268 │  46.2% │      0 ║
║ sample.fasta       │     176 │      12 │      0.30% │    PASS │ $  0.1296 │  45.3% │      0 ║
║ image.png          │     294 │      16 │      0.31% │    PASS │ $  0.1944 │  47.0% │      0 ║
```

This fixes Illumina. Nanopore is still broken, and we chased why three
times. Fix one: consensus voting now aligns each read to the group's
reference before voting instead of comparing raw positions, so it tolerates
indels, not just substitutions. Fix two, bigger: primer matching
(`nucle_index::primer::PrimerPair`) required an exact-position match, so a
single indel inside a primer — routine at Nanopore's error rate — made
retrieval drop the whole strand *before it ever reached consensus*, the
real dominant blocker. Fix three: pairwise realignment against one
arbitrarily-picked noisy reference read has a hard ceiling once a read
carries several simultaneous indels at once (the real Nanopore regime),
so `nucle_ecc::consensus` is now genuine partial-order alignment (POA) —
every read folds into one shared graph with edge-weighted voting, so a
majority correctly outvotes a minority stray insertion at any position,
including the very first or last base (previously it couldn't). Consensus
now also polishes over multiple rounds (reseed from the previous round's
own result, re-fold every read, repeat to a fixed point — what Racon/Medaka
do), verified not to regress Illumina this time after an earlier attempt's
double-counted vote weight briefly did. A synthetic worst-case test still
landed 1 base off out of 43 even after polishing converged, and the first
diagnosis for that ("column identity fragmenting") turned out to be wrong
once tested further — the real cause is that sequential graph construction
is fold-order dependent (folding the exact same reads in reverse order
gave the exactly correct answer, no other change), and polishing can't fix
that since every round reuses the same fold order. `build_consensus` now
re-runs the pipeline with a second and, if needed, third fold order and
takes whichever result a majority agree on, which resolves that test
exactly — gated on the first pass's own confidence so realistic
(non-adversarial) cases don't pay the extra cost.

Fix four, and this one wasn't in the consensus algorithm at all: the
ternary codec's own padding used a *constant* trit, and its 4-byte length
header has leading zero bytes for any file under 16MB — a constant trit
run degenerates, through the rotating cipher, into a literal
`TATATATATATATATAT...` repeat dozens of bases long at the start of
essentially every encoded file. That self-inflicted tandem repeat, not
the noise or the aligner, was the actual cause of several residual errors
that looked like a fundamental POA limit — tandem repeats are famously
hard to align under indel noise for reasons that have nothing to do with
how good the aligner is. Fixed by whitening every strand's trits with a
deterministic, position-addressable pseudo-random stream before the
cipher sees them, reversed per-strand at decode
(`TernaryCodec::whiten_segment`). Verified: the pathological repeats are
completely gone from the encoded output, and residual consensus errors
under real Nanopore noise are now small, localized 1-2-base insertions,
not sprawling corruption.

All four fixes are covered by dedicated regression tests, including a
crash found by fuzzing realistic-rate Nanopore noise at 50x coverage.

Fix five: Reed-Solomon itself turned out to have two real bugs, both
silent. First, parity symbols are arbitrary GF(256) values (0-255), but
they were being packed into DNA one base per byte via the same 2-bit
`Nucleotide::from_bits` used for already-restricted data values — any
parity byte above 3 (the overwhelming majority of them) was silently
dropped, destroying nearly every parity strand ever written. Second, a
parity strand that failed to arrive was dropped from its array via
`filter_map` instead of leaving a gap, which shifted every *later*
parity strand onto the wrong evaluation point and corrupted the whole
stripe's math. Fixed by packing each parity byte into 4 bases
(`DnaStrand::from_packed_bytes`/`unpack_bytes`) and keeping erasures as
`Option`-per-slot everywhere so a missing strand's true codeword
position is never lost. On top of that, Reed-Solomon itself was
erasure-only (could rebuild a strand marked missing, but could never
correct one that survived consensus wrong-but-present); it's now a real
combined error-and-erasure decoder (Berlekamp-Welch), so a strand that
comes back from consensus with a residual wrong base gets corrected
automatically, without knowing in advance which strand that was.
Verified directly: dedicated unit tests confirm blind single-strand
correction and correct decode across a parity gap in the middle of the
list; the full workspace suite (all crates, all doctests, the 50x-coverage
Nanopore fuzz test) passes with zero regressions.

`nucle benchmark -p nanopore -r 4` (and even `-r 12`) still fails today at
realistic settings — but ablation testing (comparing `-r 0` through
`-r 50` on the same noisy data) shows the exact same failure at every
redundancy level, which pins the remaining gap on **consensus itself**,
not Reed-Solomon: at Oxford Nanopore's real ~14% combined error rate, POA
consensus does not reliably converge to the correct sequence even before
Reed-Solomon ever runs, so no amount of parity can help. This is the same
limitation `test_nanopore_still_fails_at_realistic_indel_density_despite_alignment_fixes`
already pins down at 50x coverage. Closing it needs a better consensus/
alignment algorithm, not a bigger redundancy budget. See
[docs/architecture.md](docs/architecture.md#current-status) for the
detail.

### End-to-End Roundtrip: Encode → Noise → Recover

```
$ nucle encode README.md -o readme.dna
✓ Encoded README.md → readme.dna (2277 strands)

$ nucle simulate README.md -p illumina
╔══════════════════════════════════════╗
║     Synthesis Simulation Results     ║
╠══════════════════════════════════════╣
║ Profile:                    illumina ║
║ Coverage:                          1×║
║ Input:                  2277 strands ║
║ Output:                 2277 strands ║
║ Error rate:                  0.37%   ║
║ Surviving:                   96.3%   ║
╚══════════════════════════════════════╝

$ nucle decode readme.dna -o recovered.txt -s 56915
✓ Decoded readme.dna → recovered.txt (56915 bytes)
```

**56,915 bytes → 2,277 DNA strands × 162 nt avg = 368,874 nucleotides. Illumina noise: 0.37% error rate, 3.7% strand loss — 100% data recovery.**

### Realistic Sequencing: 10× Coverage with Consensus

Real sequencing runs at 10–50× coverage — you sequence the pool many times and consensus-vote across copies. This is the realistic scenario:

```
$ nucle simulate README.md -p illumina -c 10
╔══════════════════════════════════════╗
║     Synthesis Simulation Results     ║
╠══════════════════════════════════════╣
║ Profile:                    illumina ║
║ Coverage:                         10×║
║ Input:                  2277 strands ║
║ Output:                22770 strands ║
║ Error rate:                  0.36%   ║
║ Surviving:                   96.1%   ║
╚══════════════════════════════════════╝
```

**10 independent noisy copies per strand. Consensus voting across copies eliminates per-base errors; ECC handles the ~4% strand dropout. This is how real DNA storage systems achieve reliable recovery.**

### Full Stack: Store with ECC + CRISPR

```
$ nucle store README.md -r 4
✓ Stored 'README.md' (56915 bytes → 2277 data + 40 parity = 2317 strands,
  1.02× redundancy, primer=P0000)

╔══════════════════════════════════════╗
║         NucleOS Pool Status          ║
╠══════════════════════════════════════╣
║ Files:               1               ║
║ Total strands:    2317               ║
║ Data strands:     2277               ║
║ Parity strands:     40               ║
║ Nucleotides:    487474               ║
║ Avg strand len:    210 nt            ║
║ Redundancy:      1.02×              ║
╟──────────────────────────────────────╢
║ Files:                               ║
║   README.md (56915 B, 2277d+40p strands)║
╚══════════════════════════════════════╝
```

---

## NucleScript — Declarative DNA Operations Language

> [!NOTE]
> **Official Language & Preset Ecosystem:** Visit the [**Nuclescript Organization**](https://github.com/Nuclescript) — official packages live in the [**Packages Registry**](https://github.com/orgs/Nuclescript/packages), and the interactive web playground is [**live in your browser**](https://nuclescript.github.io/playground/) or published standalone at [**Nuclescript/playground**](https://github.com/Nuclescript/playground).

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
║ Nucleotides:      3156               ║
║ Avg strand len:    526 nt            ║
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

`consensus_vote` (and `protect`) are NucleScript's two built-in functions —
ordinary `FunctionTable` entries resolved through the exact same lookup a
call to your own `fn` goes through (arity checking, effect propagation,
"did you mean X?" suggestions), not a separate hardcoded case per
built-in. See [docs/stdlib.md](docs/stdlib.md) for both signatures.

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

`if`/`for` and comparison/boolean operators (`==`, `!=`, `<`, `>`, `<=`, `>=`,
`&&`, `||`, `!`) let a program branch on a pool's inferred error rate or repeat
an operation over a list of pool names, without hand-duplicating blocks. Both
are resolved entirely at **compile time** — the type checker evaluates the
condition once and keeps only the taken branch, and unrolls a `for` by
substitution — so the compiled plan itself never contains a branch or loop:

```nuclescript
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina

if noisy > 0.1 {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
} else {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
}

for target in [archive] {
    store "sample_a.txt" into target { redundancy: 4x }
}
```

`Result<T, E>` and `?` make VFS failures genuinely catchable — the first
real runtime behavior in the language. Before this, `store`/`retrieve`/
`delete` either succeeded or aborted the entire program; there was no way
for a NucleScript program to observe, inspect, or recover from an
operation failure. `store`/`delete` can now also appear in *expression*
position, producing a `Result<T, Str>` a `?` can unwrap or propagate to
the enclosing function's own `Result` return type:

```nuclescript
pool primary: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
pool backup: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }

fn archive_with_fallback() returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
    let saved: DnaFile = attempt?
}
```

This is the one place NucleScript's execution model stopped being "compile
a static plan, then replay it as-is": a `Result`-returning function's body
now actually executes, statement by statement, when called — everything
else in the language (including `if`/`for` above) still resolves entirely
at compile time. A bare statement-form `store`/`retrieve`/`delete` inside
a function body executes for real too (not just at the top level), and a
`File`/`Str`-typed parameter carries its real argument value at runtime —
`Ok(<expr>)`/`Err("...")` construct a `Result` directly, and compose with
`match`/`?` (nested `match`, `?` applied straight to a `match`
expression, `Ok`/`Err` as match-arm bodies). See the ["Result / Error
Propagation"](docs/grammar.md) section of the grammar reference for the
full semantics and
[docs/examples/result_fallback_store.nsl](docs/examples/result_fallback_store.nsl)
for a complete, runnable example.

A function can be generic over `Pool<T>`'s profile — one function
instead of a hardcoded copy per `Illumina`/`Nanopore`/`Twist`:

```nuclescript
pool illumina_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
pool nanopore_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Nanopore }

fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered_a: Pool<Recovered> = recover_from(noisy_illumina)

let noisy_nanopore: Pool<Nanopore, 5%> = simulate nanopore_archive under Nanopore
let recovered_b: Pool<Recovered> = recover_from(noisy_nanopore)
```

`recover_from` is type-checked once, treating `P` as an opaque
placeholder; each call site unifies `P` against its own argument's real
profile — no runtime representation, no per-instantiation re-checking of
the body. An explicit `name::<Illumina>(...)` type argument resolves the
one case inference alone can't: a type parameter that appears only inside
a `Fn(...)`-typed parameter's own signature, never as a directly
`Pool<P>`-shaped argument. See the ["Generics"](docs/grammar.md) section
of the grammar reference for the full semantics and
[docs/examples/generic_pool_recovery.nsl](docs/examples/generic_pool_recovery.nsl)/
[docs/examples/explicit_type_args_and_file_param.nsl](docs/examples/explicit_type_args_and_file_param.nsl)
for complete, runnable examples.

A caught `Result` can now be branched on directly with `match`, instead
of needing a second, independent function call from the caller:

```nuclescript
pool primary: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
pool secondary: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }

fn archive_with_fallback() returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
    let saved: DnaFile = match attempt {
        Ok(file) => file,
        Err(reason) => (store "sample_b.txt" into secondary)?
    }
}
```

`Result` used to be the only sum type in the language, closed to exactly
two variants with a fixed `Ok`-then-`Err` arm order. NucleScript now also
has real user-defined enums, and `match` is one general engine that
handles both:

```nuclescript
enum RecoveryPlan {
    Retry,
    Fallback,
    GiveUp(Str),
}

let plan: RecoveryPlan = RecoveryPlan::Fallback

fn archive_with_plan(plan: RecoveryPlan) returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
    let saved: DnaFile = match attempt {
        Ok(file) => file,
        Err(reason) => match plan {
            Retry => (store "sample_a.txt" into primary)?,
            _ => (store "sample_b.txt" into secondary)?,
        }
    }
}
```

Arm order is now free (checked by variant name, not position), and
exhaustiveness is enforced — every declared variant needs an arm, or a
trailing wildcard `_` covers the rest. `Result`'s own `Ok`/`Err`
construction syntax and runtime representation stay untouched and
distinct from a user `enum`'s — the unification lives entirely at the
matching layer, not construction or storage. See the
["Enums"/"Pattern Matching"](docs/grammar.md) sections of the grammar
reference for the full semantics (including the one still-real
limitation: `Err(...)`'s payload must always be a literal string, even
inside a user-enum match arm) and
[docs/examples/match_result_fallback.nsl](docs/examples/match_result_fallback.nsl)/
[docs/examples/recovery_plan.nsl](docs/examples/recovery_plan.nsl) for
complete, runnable examples.

Functions can now be anonymous, bound to a variable, and passed as
arguments — real closures, with real lexical capture:

```nuclescript
fn retry_once(attempt_fn: Fn() -> Result<DnaFile, Str>) returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = attempt_fn()
    let saved: DnaFile = match attempt {
        Ok(file) => file,
        Err(reason) => attempt_fn()?
    }
}
```

Capture is by snapshot, and that's simply correct here, not a design
compromise: every `let` binding in NucleScript is single-assignment, so
there's no "later mutation" a by-value/by-reference distinction could
ever observe. Closures can now be generic (`fn<T>(...)`, when nested
inside an already-generic enclosing scope) and self-recursive (a
`let`-bound closure can call itself by its own bound name), and `nucle
plan`/`nucle explain` now narrate through a `let`-bound closure's own
call. See the ["Closures"](docs/grammar.md) section of the grammar
reference for the full semantics (including the honest limits still
standing: no mutual recursion between two independently-`let`-bound
closures, and neither narration nor effect analysis sees through a
`Fn(...)`-typed *parameter*'s call) and
[docs/examples/closure_retry.nsl](docs/examples/closure_retry.nsl) for a
complete, runnable example.

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

Four official packages ship with this repository and are published to the
[Nuclescript org's package registry](https://github.com/orgs/Nuclescript/packages),
each versioned independently from NucleOS releases:

| Package | Import source | Purpose |
|---|---|---|
| `@nuclescript/presets` | `nuclescript/presets` | Baseline archive pool schemas, a reliable-store pipeline, and an `archive_with_guarantee` function |
| `@nuclescript/profiles` | `nuclescript/profiles` | Illumina/Nanopore/Twist pool presets at optimizer-recommended redundancy, plus per-profile simulate functions |
| `@nuclescript/benchmarks` | `nuclescript/benchmarks` | Pool schemas and pipelines matching the `docs/examples/fixtures/` workload set |
| `@nuclescript/recovery` | `nuclescript/recovery` | Consensus/recovery pool bindings and a `recover_with_consensus` function |

Each package's manifest, source, README, and changelog live under
`packages/nuclescript-<name>/`, with a registry index at
`packages/registry.json` — the CLI resolves packages by reading that file
directly, so adding an entry there is what makes a new package discoverable.
List or inspect bundled packages with:

```bash
nucle packages                          # quick listing of the bundled presets package
nucle package list                      # full registry.json index
nucle package inspect "@nuclescript/profiles"
```

Install and verify packages by name (resolved against `packages/registry.json`,
not a filesystem path):

```bash
nucle package install "@nuclescript/presets"
nucle package lock                      # write/update nucle.lock with manifest + source checksums
nucle package verify "@nuclescript/presets"   # checks manifest shape + checksum against nucle.lock
```

Current NucleScript result summary:

| Program | Payload | Data strands | Parity strands | Total strands | Nucleotides | Avg strand | Redundancy | Result |
|---------|--------:|-------------:|---------------:|--------------:|------------:|-----------:|-----------:|--------|
| `docs/examples/store.nsl` | 31 B | 2 | 4 | 6 | 3156 nt | 526 nt | 3.00× | Stored via VFS |
| `docs/examples/pipeline_backup.nsl` | 31 B | 2 | 4 | 6 | 3156 nt | 526 nt | 3.00× | Exact roundtrip |
| `docs/examples/sequence_literals.nsl` | — | — | — | — | — | — | — | Compile-time DNA validation |
| `docs/examples/probabilistic_recovery.nsl` | - | - | - | - | - | - | - | Compile-time error-budget propagation |
| `docs/examples/effect_confirmations.nsl` | - | - | - | - | - | - | - | Effect confirmation and planning |
| `docs/examples/preset_imports.nsl` | - | - | - | - | - | - | - | Built-in preset import validation |
| `docs/examples/control_flow.nsl` | 31 B | 2 | 4 | 6 | 3156 nt | 526 nt | 3.00× | Compile-time `if`/`for` desugaring, then stored via VFS |
| `docs/examples/result_fallback_store.nsl` | 31 B + 30 B | 4 | 5 | 9 | 4248 nt | 472 nt | 2.25× | `Result<T, E>`/`?`: a real VFS failure caught inside a function instead of aborting the run; `Ok(...)`/`Err(...)` constructors |
| `docs/examples/generic_pool_recovery.nsl` | - | - | - | - | - | - | - | Generics: `fn recover_from<P>(...)` called with both `Pool<Illumina>` and `Pool<Nanopore>`, an explicit `::<Illumina>()` type argument, and a generic closure nested inside a generic function |
| `docs/examples/match_result_fallback.nsl` | 31 B ×2 + 30 B ×2 | 8 | 8 | 16 | 7120 nt | 445 nt | 2.00× | Pattern matching: `match`'s `Ok` arm stores directly, its `Err` arm's fallback store lands on the second call; nested `match` and `?` on a `match` expression |
| `docs/examples/closure_retry.nsl` | 31 B ×2 + 30 B + 39 B | 8 | 8 | 16 | 7120 nt | 445 nt | 2.00× | Closures: a higher-order `retry_once` genuinely calls its closure argument twice on a caught failure; a captured-binding closure's fallback lands; a self-recursive closure retries into a different fallback target and terminates |
| `docs/examples/explicit_type_args_and_file_param.nsl` | 30 B ×2 | 4 | 4 | 8 | 3560 nt | 445 nt | 2.00× | `recover_generically::<Illumina>(...)` resolves a type parameter inference alone can't, and a `File`-typed parameter's real filename flows into a statement-form `store` |
| `docs/examples/recovery_plan.nsl` | 31 B ×2 | 4 | 4 | 8 | 3560 nt | 445 nt | 2.00× | A user-defined `enum RecoveryPlan`, a nested match (a user-enum match inside a `Result` match's `Err` arm), and exhaustiveness via a trailing wildcard `_` |

Compiler diagnostics are surfaced before execution. For example,
`docs/examples/critical_redundancy_warning.nsl` warns when critical data uses
only `1x` redundancy.

`nucle check` runs lex → parse → typecheck without touching hardware or
executing anything — the fast path for CI or an editor integration. Every
diagnostic carries a real `file:line:column` (threaded from the lexer's
token positions through the parser's AST and into the type checker), a
stable error code, and a rustc-style source snippet — not just a message
with no source location to jump to. See [docs/errors.md](docs/errors.md)
for the full list of codes:

```bash
$ nucle check docs/examples/failures/missing_confirmation.nsl
docs/examples/failures/missing_confirmation.nsl:11:1: error [E-DELETE-UNCONFIRMED]: delete 'old_archive.bin' from 'archive' has Destructive effect and requires explicit physical key confirmation
   |
11 | delete "old_archive.bin" from archive
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

`nucle explain` goes further, turning MIR optimizer notes and the program's
full effect summary (including effects propagated through function calls —
calling a function that deletes something isn't automatically safe just
because it's wrapped in a function) into plain-language explanations. See
[docs/effects.md](docs/effects.md) for the full effect model:

```bash
$ nucle explain docs/examples/critical_redundancy_warning.nsl
--- Execution & Safety Explanation ---

### Optimization Decisions:
- optimiser raised redundancy for 'sample_a.txt' from 1x to 4x under Illumina. Redundancy was increased to satisfy statistical recovery guarantees under this profile's specific error profile.

### Safety & Confirmation Summary:
- pool 'archive' (Pure): Pure effect. [SAFE (Pure)]
- store 'sample_a.txt' (Synthesis): Synthesis effect. [CONFIRMED]
```

### Editor Support

A VS Code extension lives at
[`editors/vscode/nuclescript/`](editors/vscode/nuclescript/) — syntax
highlighting for `.nsl` files (keywords, types, profile/codec constants,
strings, and the `3x`/`99.5%`/date/size-in-bytes number forms `lexer.rs`
actually recognizes), derived directly from the real grammar so it can't
highlight a token the compiler would reject. A snapshot test (`npm test`
inside that directory) tokenizes every file in `docs/examples/` and diffs
against committed snapshots, so a compiler keyword change that isn't
mirrored in the grammar shows up as a CI-visible diff instead of silently
going stale.

The extension also spawns a real language server —
[`nucle_lsp`](nucle_lsp/) — over stdio, so `.nsl` files get live
diagnostics (the same errors/warnings and error codes `nucle check`
reports, as you type), hover (pool/function/strand/sequence/binding
signatures), go-to-definition, and a document outline. `nucle_lsp` is a
thin protocol adapter over `nucle_lang::analyze` — it never duplicates
compiler logic, verified by an integration test that speaks the real
Content-Length-framed JSON-RPC protocol over an in-memory pipe and
cross-checks published diagnostics against `nucle check`'s own output for
the same source. Build it with `cargo build -p nucle_lsp --release`.
Autocomplete, rename, and semantic tokens aren't built yet.

Beyond editing, the extension can format (`Format Document`/format on
save, via `nucle-cli fmt`) and actually run a program (`NucleScript: Run
File` — a ▷ button, `Ctrl+F5`/`Cmd+F5`, or the Explorer context menu —
via `nucle-cli run`, with output in an integrated terminal). Neither
needs a local Rust toolchain: `nucle-lsp` and `nucle-cli` are each
resolved on `PATH` first, then downloaded once from GitHub Releases and
cached (`src/serverDownload.ts`, `src/cliDownload.ts`) if not found —
installing the extension is enough to write, check, format, and run a
`.nsl` file.

The extension is [**published on the VS Code
Marketplace**](https://marketplace.visualstudio.com/items?itemName=nuclescript.nuclescript)
— icon, changelog, license, a
`.github/workflows/release-vscode-extension.yml` that builds `nucle-lsp`
for Windows/Linux/macOS (x64 + arm64) and attaches them to a GitHub
Release, and an in-extension downloader so a marketplace install works
without a local Rust toolchain. See the extension's own
[CONTRIBUTING.md](editors/vscode/nuclescript/CONTRIBUTING.md#publishing-an-update-to-the-marketplace)
for how to ship an update.

### Playground

**🧪 [Try it live in your browser](https://nuclescript.github.io/playground/)**
— no install, no download. `nucle_wasm` compiles the same compiler/codec/ECC
engine to WebAssembly and runs it entirely client-side; a GitHub Actions
workflow (`Nuclescript/playground`'s `.github/workflows/pages.yml`) rebuilds
and redeploys it on every push, so it's always current.

The playground has three tabs, each backed by the real engine (no
reimplemented math, no mocked data):

- **Write & Run** — the same `analyze_source` API `nucle check --json` uses
  internally. Paste a `.nsl` program, get diagnostics, simulation steps, and
  optimizer notes.
- **Benchmark Explorer** — pick a codec/profile, drag the redundancy slider,
  and density/GC%/cost/recovery-probability update live — computed by
  `nucle_codec::benchmark` plus a real Reed-Solomon-aware Monte-Carlo
  recovery estimate, not a lookup table.
- **Pipeline Visualizer** — encodes real input through the actual
  codec/ECC/noise engine and animates each strand through
  encode → synthesize/sequence (noise) → recover, including honest failures
  when redundancy/profile can't reconstruct the data.

Prefer a native server over the browser build? `nucle_playground` is the
same three tabs as a self-contained `tiny_http` server:

```bash
cargo run -p nucle_playground
# open http://127.0.0.1:8080
```

It's also published standalone at
[**Nuclescript/playground**](https://github.com/Nuclescript/playground) — a
self-contained snapshot of this workspace (verified to build independently
from a fresh clone) for anyone who wants to run the playground without
cloning this repo directly. For zero setup at all (no `cargo`, no cloning),
grab a prebuilt binary from its
[**Releases**](https://github.com/Nuclescript/playground/releases) —
Linux/Windows/macOS builds with the frontend embedded, so downloading and
running the single file is enough.

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

# Compile-only validation: lex -> parse -> typecheck, no hardware, no execution
nucle check docs/examples/store.nsl
nucle check docs/examples/store.nsl --json

# Explain effect summary and optimizer decisions in plain language
nucle explain docs/examples/critical_redundancy_warning.nsl

# Format a NucleScript source file in its one canonical style (gofmt-style,
# zero configuration). Prints to stdout by default.
nucle fmt docs/examples/store.nsl
nucle fmt docs/examples/store.nsl --write     # rewrite the file in place
nucle fmt docs/examples/store.nsl --check     # exit non-zero if not already formatted (for CI)

# Run test { ... } blocks (assert reuses if's comparison/boolean operators,
# evaluated at compile time -- see docs/grammar.md#testing-test--assert)
nucle test docs/examples/archive_fn.nsl
nucle test docs/examples/archive_fn.nsl --json

# Generate Markdown docs from /// doc comments on pool/strand/seq/fn/pipeline
nucle doc docs/examples/archive_fn.nsl
nucle doc docs/examples/archive_fn.nsl --output archive_fn.md

# Scaffold a new NucleScript project (nucle check/run succeed against it unmodified)
nucle new my-project
cd my-project && nucle run main.nsl

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
| `nucle_codec` | 62 (+3 doctests) | Nucleotide types, constraints, ternary codec (incl. a regression test proving the fixed strand-index header roundtrips past its old 81-strand capacity, and a test confirming encoding fails loudly instead of silently corrupting once truly past the new, much larger capacity), fountain codec, yin-yang codec, byte↔4-base packing roundtrip, benchmarks incl. GC distribution and homopolymer violation counts |
| `nucle_synth` | 32 | Error models, noise engine, hardware profiles, encode→noise→decode e2e |
| `nucle_ecc` | 39 | Reed-Solomon (incl. combined error-and-erasure Berlekamp-Welch decoding, blind single-strand correction, parity-reindexing regression), fountain erasure, repair pipeline, per-position observed error distribution, partial-order-alignment consensus (frame-shifting indels, boundary insertions outvoted by majority, fold-order-independence, realistic-noise fuzz crash safety) |
| `nucle_index` | 31 | Primers (incl. edit-distance-tolerant boundary matching under indel noise), CRISPR sim, vector index, semantic search |
| `nucle_vfs` | 50 (+1 ignored) | Pool, file, catalog, storage manifests, content-addressed archive IDs, migration (incl. codec-migration rejection), per-object recovery manifests, regression-pinned fixture roundtrips, Illumina/Nanopore noise roundtrips (a slow, realistic-scale Nanopore regression check is `#[ignore]`d; run it explicitly with `cargo test -p nucle_vfs -- --ignored`) |
| `nucle_agent` | 27 | Tool defs, planner, executor |
| `nucle_lang` | 230 | Lexer (incl. `///` doc comments as real, distinct tokens, rejected with a clear parse error anywhere they can't attach), parser, biological checks, sequence literals, probabilistic pool typing, effects (incl. propagation through function calls, `if`/`for` branches, `?` short-circuits, and built-in `consensus_vote`/`protect` calls), compile-time `if`/`for` desugaring with comparison/boolean operators, `consensus_vote`/`protect` resolved as ordinary stdlib `FunctionTable` entries (arity/effects/"did you mean" parity with user functions), canonical formatter (`nucle fmt`, idempotence + parsed-program-equivalence over every shipped example, doc-comment-aware), `test`/`assert` test runner (`nucle test`: compile-time assertion evaluation shared with `if`, real per-test VFS execution, compile-error-vs-test-failure separation), Markdown doc generation from `///` comments (`nucle doc`), MIR optimizer, simulation backend, table-driven package registry (all 4 official packages), lock file checksums, hardware request collection, VFS lowering, function declarations/calls, source spans + stable error codes + "did you mean" suggestions, symbol table for tooling, `nucle check`/`nucle explain` integration tests, `Result<T, E>`/`?` (parsing, typeck validity rules, conservative effect-joining across a `?` short-circuit, and a golden-file regression suite proving zero behavior change for programs that use none of it), generics over `Pool<T>`'s profile (call-site unification, type-parameter conflict/unresolved detection, and a formatter regression test proving `noisy < 0.1`-style comparisons aren't mistaken for a generic angle-bracket list), closures/higher-order functions (real lexical capture, a genuinely higher-order call retrying twice on a caught failure, effect analysis correctly resolving a called closure's real body, the existing arity/type-mismatch/undeclared-function paths proven unaffected), a gap-closing pass over earlier `Result`/generics/pattern-matching/closures work (`Ok(...)`/`Err(...)` constructors and composability with nested `match`/`?`, explicit `::<Illumina>()` type arguments, real statement-form execution and real `File`/`Str`-typed parameter values inside function bodies, generic closures, a self-recursive closure actually terminating for real against a live VFS, and `nucle plan`/`nucle explain` narration reaching into a `let`-bound closure's own body), and user-defined `enum`s + a general N-arm pattern-matching/exhaustiveness engine (one diagnostic code per new `E-ENUM-*`/`E-MATCH-*` failure mode, free arm order, a real user-flagged nested-match limitation fixed, `Result`/`Ok`/`Err` re-run unmodified through `result_backward_compat.rs`'s golden-file suite proving zero behavior change from the unification) |
| `nucle_hardware` | 21 | Confirmation gating (effectful/destructive rejection, count/message correctness), mock provider dry runs, file-export JSON roundtrip and field preservation, parent-directory creation |
| `nucle_lsp` | 11 | Word-at-cursor resolution, hover/definition lookup, and a real Content-Length-framed JSON-RPC integration test (diagnostics, hover, go-to-definition) cross-checked against `nucle check`'s own output |
| `nucle_demo_core` | 5 | Interactive benchmark/pipeline demo engine: end-to-end recovery estimation, unknown-codec/oversized-input rejection |
| **Total** | **508 (+3 doctests, +1 ignored)** | **End-to-end: binary → DNA → noise → ECC → recover → binary** |

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
nucle_lsp/       — NucleScript language server (tower-lsp adapter over nucle_lang::analyze: diagnostics, hover, go-to-definition, document outline)
nucle_cli/       — Command-line interface
nucle_playground/ — Interactive web playground (tiny_http server + static frontend), also published at github.com/Nuclescript/playground
nucle_demo_core/ — Shared, I/O-free benchmark/pipeline-visualizer logic used by both nucle_playground and nucle_wasm
nucle_wasm/      — Same playground compiled to WebAssembly; live at nuclescript.github.io/playground
editors/vscode/nuclescript/ — VS Code extension: TextMate grammar + language server client
docs/            — Architecture notes, paper references, and runnable examples/fixtures
packages/        — NucleScript package registry (packages/registry.json) and package releases (presets, profiles, benchmarks, recovery)
```

---

## License

MIT — see [LICENSE](LICENSE) for details.
