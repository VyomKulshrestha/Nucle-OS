# NucleOS Operations

This guide covers the durable NucleOS storage engine. NucleScript language and
editor documentation lives in the other files in this directory.

## Pool resolution and durability

Commands resolve a pool in this order: `--pool-dir`, `NUCLEOS_POOL_DIR`, then
the project-local `.nucleos/`. `--tenant` or `NUCLEOS_TENANT` adds an isolated
`tenants/<name>/` directory. Pool state is atomically replaced under a file
lock and guarded by an optimistic version counter, so a stale process refuses
to overwrite a newer state.

```bash
nucle --pool-dir ./archive store report.bin -r 4 --codec ternary
nucle --pool-dir ./archive retrieve report.bin
nucle --pool-dir ./archive pool
```

Supported storage codecs are `ternary` (alias
`ternary-rotating-cipher`, the default) and `yin-yang` (alias `yinyang`). The
selected codec is persisted with each version and returned in JSON write
results; reads select the decoder from that metadata.

## Version lifecycle

Writing an existing name creates a new immutable content version. Each version
has its own content-addressed archive ID, primer, strands, codec, parity, and
recovery manifest.

```bash
nucle store report.bin --codec yin-yang
nucle store report.bin --codec ternary -r 6
nucle history report.bin
nucle retrieve report.bin --version 1
nucle rename report.bin archive/2026/report.bin
nucle delete archive/2026/report.bin --confirm
```

Rename is metadata-only and moves the complete version chain; archive IDs,
primers, and DNA strands do not change. Rename refuses collisions. Delete
requires `--confirm` and removes the current record plus every historical
version. Migration is additive too: it reads the current content and writes a
new encoded version, preserving every earlier version.

Primer allocation is a lifetime address counter, not a file-count ceiling.
The deterministic library expands when full and regenerates enough addresses
to open an existing pool even if the caller supplies a smaller capacity hint.
Refused writes do not consume an address.

## Capacity, integrity, encryption, and observability

```bash
nucle capacity 1000000
nucle scan
nucle audit --tail 20
nucle --pool-key "passphrase" encrypt-pool
nucle --pool-key "passphrase" retrieve archive/2026/report.bin
nucle serve --port 9898
```

Capacity is measured from the final encoded and parity-protected nucleotide
footprint. Scans execute the real recovery pipeline and persist recovery
manifests. Encryption protects `state.json` with a random ChaCha20-Poly1305
data key wrapped by an Argon2id-derived passphrase key; audit/config metadata
remains plaintext. The metrics exporter binds to localhost by default and
reopens the pool on every scrape.

## Persistent molecular block-device images

The block-device API is parallel to the file VFS: it exposes fixed-size logical
block addresses rather than named, versioned blobs.

```bash
nucle block create synth.dnaimg --backend synthesis-array --block-size 256 --capacity-blocks 64
nucle block write synth.dnaimg 0 input.bin
nucle block read synth.dnaimg 0 --output output.bin
nucle block info synth.dnaimg

nucle block create rewrite.dnaimg --backend rewritable-nanopore --block-size 256 --capacity-blocks 64
nucle block write rewrite.dnaimg 0 first.bin
nucle block write rewrite.dnaimg 0 replacement.bin
```

Images use a versioned JSON envelope and atomic replacement. Backend detection
comes from the image header. Synthesis-array images are permanently write-once
per LBA; rewritable-Nanopore images permit overwrite and persist both the read
counter and per-block write generation so noise seeds never rewind after a
reopen. `--force` on `block create` replaces an existing image; ordinary
create refuses to overwrite one.

## Hardware provider boundary

`nucle hardware export` supports `file-export`, `mock`, and `mock-delayed`.
Unknown names and the library-only `twist`, `idt`, `illumina`, and `nanopore`
providers are rejected before compilation, submission, or output creation.
The vendor adapters remain callable through `nucle_hardware`; the CLI will not
claim it used one until it has vendor-specific credential input and a
resumable job lifecycle.

## Verification snapshot

Measured from local `main` on 13 August 2026:

- `cargo build --workspace --all-targets`: pass.
- `cargo test --workspace --no-fail-fast`: 729 discovered, 728 passed, one
  intentionally ignored.
- NucleOS package Clippy (`--all-targets --no-deps -- -D warnings`): pass for
  all nine NucleOS packages.
- `cargo audit`: no vulnerabilities across 276 dependencies.
- Pristine, Illumina `-r 4`, and Nanopore `-r 12` full-pipeline benchmarks:
  all four standard fixtures recovered exactly in each run.

Benchmark details and current figures are recorded in the root README.
