# Nucle-OS — DNA Storage Engine

**A complete software-defined DNA storage operating system.**

The same way software-defined networking abstracts physical switches, Nucle-OS abstracts physical DNA synthesizers. It is the driver layer that molecular data storage plugs into.

---

## Architecture

```
┌─────────────────────────────────────────┐
│           Agent Interface Layer          │  ← AI agent for semantic file ops
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

---

## Building

```bash
# Build the entire workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Run the CLI
cargo run --bin nucle-cli -- --help
```

---

## Project Structure

```
nucle_codec/     — Encoding/Decoding engine (binary ↔ ATCG)
nucle_synth/     — Synthesis simulator (hardware mock)
nucle_ecc/       — Error correction (Reed-Solomon, fountain, consensus)
nucle_index/     — Retrieval & indexing (CRISPR-sim, vector index)
nucle_vfs/       — Virtual file system (syscall-style API)
nucle_agent/     — Agent interface (ReAct planner)
nucle_cli/       — Command-line interface
```

---

## License

MIT — see [LICENSE](LICENSE) for details.
