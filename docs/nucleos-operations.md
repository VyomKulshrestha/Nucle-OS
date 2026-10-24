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
`ternary-rotating-cipher`, the default), `fountain` (canonical persisted name
`dna-fountain`), and `yin-yang` (alias `yinyang`). The selected codec is
persisted with each version and returned in JSON write results; reads select
the decoder from that version's metadata. NucleScript uses the same path:
`codec: Fountain` on a pool drives direct stores, and
`encode "..." using Fountain` drives pipeline stores and roundtrip checks.

Fountain uses screened, deterministic rateless batches. Seed and payload
whitening prevent binary headers or low-entropy content from defeating the DNA
constraints; candidates must pass the persisted GC, local-GC, homopolymer,
palindrome, and length profile both before and after primer tagging. Screening
budget exhaustion and empty input are errors, never zero/partial-strand files.
Peeling decode falls back to GF(2) elimination for any full-rank survivor set.

Use `nucle extend-fountain report.bin --droplets 20` to materialize a larger
batch. Extension is a new immutable version with freshly computed parity; the
older physical representation stays readable. The software checks demonstrate
constraint compliance, not successful physical synthesis. Recovery from a
rank-deficient subset is impossible; the hard arbitrary known-erasure bound is
provided by `-r/--redundancy` Reed-Solomon parity:
`2*errors + erasures <= parity_count` after consensus. To choose that parity
for a channel, `nucle parity report.bin --codec hedges -p array-lofi --coverage 3`
sends the file's strands through the simulated reads and decoder and prints
the `--redundancy` that keeps the chance of losing the file under 0.1%
(`--max-failure` changes the target).

## Version lifecycle

Writing an existing name creates a new immutable content version. Each version
has its own content-addressed archive ID, primer, strands, codec, parity, and
recovery manifest.

```bash
nucle store report.bin --codec yin-yang
nucle migrate report.bin --codec fountain
nucle extend-fountain report.bin --droplets 20
nucle migrate report.bin --codec ternary -r 6
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

`nucle hardware export` supports `file-export`, `mock`, `mock-delayed`,
`twist`, `idt`, `illumina`, and `nanopore`. Vendor providers require
`--config`; the strict JSON config names environment variables for secrets and
never embeds credentials in the generated job record. Successful submission
writes an append-only `--job-file` and reports `Submitted`. Poll it from a new
process with `nucle hardware status JOB --config CONFIG`.

`nucle hardware capabilities [provider]` exposes each adapter's static
request/profile contract before credentials are loaded. The provider boundary
rejects a mismatched profile (for example, a `Twist` synthesis batch sent to
IDT) before job-file creation or network access. Library callers that block on
a handle can use `wait_timeout`; expiry leaves the remote job and resume token
intact for a later status check.

The compiler confirmation, operator `--confirm`, and optional `confirm-users`
checks all run before the first vendor request. Unknown providers, missing or
unknown config fields, provider/config mismatches, insecure non-loopback HTTP,
missing credentials, malformed resume records, and attempts to overwrite a job
file fail closed. See [Hardware provider CLI](hardware-providers.md) for each
vendor's schema and operational boundary.

## Verification snapshot

Measured locally on 30 August 2026:

- The current inventory is 764 discovered tests: 763 runnable and one
  intentionally expensive Nanopore test ignored.
- All 763 runnable tests, including three doctests, passed with
  `cargo test --workspace --no-fail-fast -q` in 161.5 seconds.
- Workspace Clippy (`cargo clippy --workspace --all-targets -- -D warnings`):
  pass for every workspace crate and target.
- `cargo audit`: no known vulnerabilities across 279 locked dependencies.
- The 21 August 2026 Pristine, Illumina `-r 4`, and Nanopore `-r 12`
  full-pipeline benchmarks remain the latest codec/recovery measurements;
  all four standard fixtures recovered exactly in each run.

Benchmark details and current figures are recorded in the root README.
