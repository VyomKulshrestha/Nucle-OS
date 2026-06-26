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
| `yinyang_schema.nsl` | Future codec schema parsing with current-backend compatibility warning |

`sample_a.txt` and `sample_b.txt` are small payloads used by the examples.
