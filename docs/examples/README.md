# NucleScript Example Programs

These `.nsl` source files demonstrate the current NucleScript language surface.
Run any executable program with:

```bash
nucle run docs/examples/store.nsl
```

| Source file | Demonstrates |
|---|---|
| `hero.nsl` | Compact end-to-end language showcase used by the root README: Yin-Yang pool, probabilistic recovery, pipeline store, and exact roundtrip verification |
| `minimal.nsl` | Smallest runnable NucleScript program: pool, store, retrieve |
| `store.nsl` | Pool declaration plus store operation with tags and redundancy |
| `retrieve_query.nsl` | Declarative retrieval query over stored metadata |
| `simulate_store.nsl` | Simulated store with coverage and recovery expectation |
| `pipeline_backup.nsl` | Pipeline program with encode, protect, store, roundtrip verify |
| `fountain_roundtrip.nsl` | Direct Fountain store plus a Fountain encode/protect/store pipeline with exact VFS roundtrip verification; recovery past the outer Reed–Solomon bound is rank-dependent |
| `multi_store_search.nsl` | Multiple stores followed by a search-style retrieval query |
| `critical_redundancy_warning.nsl` | Compiler warning for critical data with insufficient redundancy |
| `nanopore_recovery_warning.nsl` | Compiler warning for unrealistic Nanopore recovery expectation |
| `idt_vendor.nsl` | Typed IDT oPools synthesis request with both language-level and operator-level confirmation boundaries |
| `strand_constraints.nsl` | Compile-time failure for invalid hardcoded DNA strand constraints |
| `sequence_literals.nsl` | DNA-native `Sequence` literals and `seq"..."` validation |
| `probabilistic_recovery.nsl` | Probabilistic `Pool<P, E>` typing and consensus error propagation |
| `effect_confirmations.nsl` | Synthesis, sequencing, and destructive effect confirmations |
| `preset_imports.nsl` | Built-in preset imports for package registry readiness |
| `yinyang_schema.nsl` | Yin-Yang pool schema executed through the supported VFS storage backend |
| `control_flow.nsl` | Compile-time `if`/`for`: a boolean-combined error-rate condition picking a recovery branch, and a `for` loop unrolled into a `store` |
| `archive_fn.nsl` | A reusable, `///`-documented `fn` wrapping `protect ... for ...`, plus a `test { ... }`/`assert` block verifying `consensus_vote` actually reduces the inferred error rate — run with `nucle test archive_fn.nsl`, or `nucle doc archive_fn.nsl` for its generated Markdown |
| `result_fallback_store.nsl` | `Result<T, E>` + `?`: two functions storing the same file into different pools, each propagating a real VFS failure via `?` instead of aborting the run, plus a top-level binding holding a still-wrapped `Result` |
| `generic_pool_recovery.nsl` | Generics: one `fn recover_from<P>(...)` called with a `Pool<Illumina>` and a `Pool<Nanopore>` argument in the same program — fails to type-check without a type parameter |
| `explicit_type_args_and_file_param.nsl` | Explicit `::<Illumina>` type arguments plus a real `File` parameter value flowing into statement-form storage |
| `match_result_fallback.nsl` | Pattern matching: `match attempt { Ok(file) => file, Err(reason) => ... }` branches on a caught `Result` directly inside one function — calling it twice in one run exercises both the `Ok` arm and the `Err` arm's fallback store |
| `recovery_plan.nsl` | User-defined enums, payload binding, nested `Result`/enum matches, and exhaustive wildcard handling |
| `closure_retry.nsl` | Closures: a genuinely higher-order `fn retry_once(attempt_fn: Fn() -> Result<DnaFile, Str>)` that calls whatever closure it's given, and a `let`-bound closure that captures a `Result` binding from its enclosing function |
| `effect_annotated_closure.nsl` | Effect-annotated `Fn(...)` types that preserve destructive confirmation requirements through indirect closure calls |
| `retention_policy.nsl` | Pool `storage:` conditions and `retain:` requirements: a 10,000-year requirement fails on a room-temperature bone shelf (an `Err` the program catches) and is met by a −20 °C capsule |
| `tiered_archive.nsl` | Most of the language in one program: preset imports, payload enums, a higher-order function taking closures, `?` propagation, generic recovery, compile-time `if`/`for`, a pipeline, and a `test`. Its three pools are separate namespaces, so the `for` loop leaves three independent replicas |
| `wet_lab_run.nsl` | A confirmed synthesis/sequencing run through an effect-annotated closure, and a generic `polish<P>` whose `match` recovers at 10x or 30x; the match's pool type joins at the worse error rate. Its `test` asserts on pools the program simulated at top level |

`patient_records_2026.csv` and `sample_a.txt` through the other `sample_*.txt`
files are small payloads used by the examples.

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
