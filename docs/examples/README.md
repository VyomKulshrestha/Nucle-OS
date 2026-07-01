# NucleScript Example Programs

These `.nsl` source files demonstrate the current NucleScript language surface.
Run any executable program with:

```bash
nucle run docs/examples/store.nsl
```

| Source file | Demonstrates |
|---|---|
| `minimal.nsl` | Smallest runnable NucleScript program: pool, store, retrieve |
| `store.nsl` | Pool declaration plus store operation with tags and redundancy |
| `retrieve_query.nsl` | Declarative retrieval query over stored metadata |
| `simulate_store.nsl` | Simulated store with coverage and recovery expectation |
| `pipeline_backup.nsl` | Pipeline program with encode, protect, store, roundtrip verify |
| `multi_store_search.nsl` | Multiple stores followed by a search-style retrieval query |
| `critical_redundancy_warning.nsl` | Compiler warning for critical data with insufficient redundancy |
| `nanopore_recovery_warning.nsl` | Compiler warning for unrealistic Nanopore recovery expectation |
| `strand_constraints.nsl` | Compile-time failure for invalid hardcoded DNA strand constraints |
| `sequence_literals.nsl` | DNA-native `Sequence` literals and `seq"..."` validation |
| `probabilistic_recovery.nsl` | Probabilistic `Pool<P, E>` typing and consensus error propagation |
| `effect_confirmations.nsl` | Synthesis, sequencing, and destructive effect confirmations |
| `preset_imports.nsl` | Built-in preset imports for package registry readiness |
| `yinyang_schema.nsl` | Future codec schema parsing with current-backend compatibility warning |

`sample_a.txt` and `sample_b.txt` are small payloads used by the examples.

Use `nucle plan <source.nsl>` for examples that demonstrate compile-time
analysis, probabilistic typing, effects, optimizer notes, or the simulation
backend without touching hardware or executing VFS mutations.

## `fixtures/`

Standard benchmark/regression workloads, exercised by `nucle benchmark`,
`nucle doctor`, and `nucle_vfs`'s regression test suite: `small_text.txt`,
`archive.bin`, `sample.fasta`, `image.png`, and `project_tree/` (a small,
metadata-heavy directory tree of multiple files with varied names/sizes,
used to exercise multi-file storage as a single unit).

## `failures/`

Runnable programs that demonstrate real compiler diagnostics — see
[failures/README.md](failures/README.md) for the exact expected output of
each. These intentionally fail *type checking* (or emit warnings) by design,
but must still be syntactically valid NucleScript; `nucle doctor` checks
that invariant separately from the top-level examples above.
