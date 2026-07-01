# @nuclescript/benchmarks

Standard benchmark workload templates matching the fixture set NucleOS ships
under `docs/examples/fixtures/`: small text, a binary archive, FASTA sequence
data, and an image payload. Each pipeline is a ready-to-run
encode → protect → store → verify workflow.

```nuclescript
import {
    small_text_workload,
    binary_archive_workload,
    fasta_workload,
    image_workload
} from "nuclescript/benchmarks"
```

## Exports

| Export | Kind | Purpose |
|---|---|---|
| `benchmark_text_archive` | Pool schema | Illumina defaults sized for small text |
| `benchmark_binary_archive` | Pool schema | Illumina defaults sized for binary data |
| `benchmark_fasta_archive` | Pool schema | Twist defaults sized for FASTA data |
| `benchmark_image_archive` | Pool schema | Nanopore defaults sized for image data |
| `small_text_workload` | Pipeline | Full roundtrip for a small text file |
| `binary_archive_workload` | Pipeline | Full roundtrip for a binary archive |
| `fasta_workload` | Pipeline | Full roundtrip for FASTA sequence data |
| `image_workload` | Pipeline | Full roundtrip for an image payload |

## Package

- GitHub package: `@nuclescript/benchmarks`
- Import source: `nuclescript/benchmarks`
- Version: `0.1.0`
- Source: `src/benchmarks.nsl`

These pipelines mirror the workloads `nucle benchmark` runs against
`docs/examples/fixtures/` in the core NucleOS repository, so results from
`nucle benchmark` and from running these pipelines directly are comparable.
