# NucleScript Packages

NucleScript packages are reusable biological storage presets: pool schemas,
recovery profiles, pipeline templates, and functions that can be imported by
`.nsl` source files. Official packages are published under the
[Nuclescript GitHub organization](https://github.com/Nuclescript)'s
[package registry](https://github.com/orgs/Nuclescript/packages), independent
of NucleOS's own release cadence.

Four packages are bundled with this repository today: `presets`, `profiles`,
`benchmarks`, and `recovery`.

## What an import does

`import { name as alias } from "nuclescript/<package>"` brings the package's
real declaration into the program. Before type checking, each imported pool,
`let` binding, function, enum, strand or sequence is spliced in from the
package's source, after the `import` line, together with every other
declaration from that package it refers to. Importing `illumina_recovery`
therefore also brings `illumina_noisy` and `medical_archive`, in that order.
An `as` alias renames the imported item only. Spliced code takes the
`import` line's source position, so a diagnostic about it points at the
import. Importing a name the program also declares is an ordinary duplicate
(`E-POOL-DUPLICATE`, `E-FUNCTION-DUPLICATE`, ...).

Pipeline exports such as `reliable_store` are checked by name but not
spliced: declaring a pipeline runs it, and importing a template should not
store files as a side effect.

Imported functions are called like local ones. A `Pool<Profile>` parameter
takes a declared pool of that profile (the function's `store ... into
target` writes to the pool the caller passed), and a `Recovery` parameter
takes a recovered pool, which is what packages export as recovery
profiles:

```nuclescript
import { archive_with_guarantee, medical_archive, illumina_recovery } from "nuclescript/presets"

let saved: DnaFile = archive_with_guarantee("records.csv", medical_archive, illumina_recovery)
```

## @nuclescript/presets 0.1.0

GitHub package: `@nuclescript/presets`

Import source:

```nuclescript
import {
    medical_archive,
    reliable_store,
    illumina_recovery
} from "nuclescript/presets"
```

Package files:

- `packages/registry.json`
- `packages/nuclescript-presets/package.json`
- `packages/nuclescript-presets/src/presets.nsl`
- `packages/nuclescript-presets/README.md`
- `packages/nuclescript-presets/CHANGELOG.md`

Exports:

| Export | Kind | Purpose |
|---|---|---|
| `medical_archive` | `pool_schema` | Conservative Ternary + Illumina archive defaults |
| `twist_archive` | `pool_schema` | Twist-oriented low-error archive defaults |
| `reliable_store` | `pipeline` | Encode, protect, store, and verify workflow |
| `illumina_recovery` | `recovery_profile` | Illumina consensus recovery defaults |
| `archive_with_guarantee` | `function` | Reusable archive workflow: protect data for a recovery guarantee, then store into a target pool |

## @nuclescript/profiles 0.2.0

Illumina, Nanopore, Twist, and IDT hardware profile presets with recommended
redundancy and measured error rates. See
[packages/nuclescript-profiles/README.md](../packages/nuclescript-profiles/README.md).

| Export | Kind | Purpose |
|---|---|---|
| `illumina_standard` / `illumina_high_coverage` | `pool_schema` | Illumina defaults at 4x / 3x redundancy |
| `nanopore_standard` / `nanopore_high_coverage` | `pool_schema` | Nanopore defaults at 8x / 6x redundancy |
| `twist_standard` | `pool_schema` | Twist defaults at 2x redundancy |
| `recommended_pool_for_illumina/nanopore/twist` | `function` | Simulate a pool under the given profile |

## @nuclescript/benchmarks 0.1.0

Standard benchmark workload pool schemas and pipelines matching the fixture
set under `docs/examples/fixtures/`. See
[packages/nuclescript-benchmarks/README.md](../packages/nuclescript-benchmarks/README.md).

| Export | Kind | Purpose |
|---|---|---|
| `benchmark_text_archive` / `_binary_archive` / `_fasta_archive` / `_image_archive` | `pool_schema` | Per-workload pool defaults |
| `small_text_workload` / `binary_archive_workload` / `fasta_workload` / `image_workload` | `pipeline` | Full encode→protect→store→verify roundtrip per workload |

## @nuclescript/recovery 0.1.0

Consensus and recovery templates for turning a noisy channel into a
recovered pool. See
[packages/nuclescript-recovery/README.md](../packages/nuclescript-recovery/README.md).

| Export | Kind | Purpose |
|---|---|---|
| `recovery_archive` | `pool_schema` | Illumina defaults at 4x redundancy |
| `recovery_consensus_10x` / `recovery_consensus_20x` | `recovery_profile` | Consensus-recovered bindings at 10x/20x coverage |
| `recover_with_consensus` | `function` | Simulate + consensus-vote a source pool at 10x coverage |

Inspect bundled packages:

```bash
nucle packages          # quick listing of the bundled presets package
nucle package list      # full packages/registry.json index
```

`packages/registry.json` is the actual source of truth the CLI resolves
against — `nucle_lang::package::get_registry()` parses it at startup and
looks up each listed entry's manifest by the path recorded there. Adding a
package means adding both its manifest under `packages/<name>/package.json`
*and* an entry in `packages/registry.json` pointing at it; a manifest that
exists on disk but isn't listed in the registry is invisible to the CLI.

Install a package **by name** (resolved against the registry, not a
filesystem path):

```bash
nucle package install "@nuclescript/presets"
```

An unregistered name fails clearly instead of a generic parse error:

```bash
$ nucle package install "@nuclescript/does-not-exist"
Package '@nuclescript/does-not-exist' not found in packages/registry.json.
```

Write or refresh `nucle.lock` (JSON format, matching the rest of the CLI's
structured output), which records a SHA-256 checksum of every registered
package's manifest **and** its `.nsl` source files concatenated — so editing
either trips a mismatch, not just the manifest:

```bash
nucle package lock
```

```json
{
  "packages": [
    {
      "name": "@nuclescript/presets",
      "version": "0.1.0",
      "import_source": "nuclescript/presets",
      "checksum": "<sha256 of package.json + presets.nsl>"
    }
  ]
}
```

Verify a package's manifest shape (non-empty name, version, and exports, with known export kinds) and, if `nucle.lock` exists, that its checksum still matches:

```bash
nucle package verify "@nuclescript/presets"
```

`nucle run`/`nucle check` also check every package a `.nsl` program imports
against `nucle.lock`, but **only warn** — they never hard-fail on a mismatch,
since the registry is still entirely local/built-in and a warning is enough
to catch "forgot to re-lock after editing presets.nsl":

```bash
$ nucle check docs/examples/preset_imports.nsl
Warning: package 'nuclescript/presets' has drifted from nucle.lock (locked=..., actual=...). Run 'nucle package lock' to refresh.
Check status: OK (no errors or warnings)
```

Inspect a package's details, version, repository, and exported items, returning manifest fields and exports:

```bash
nucle package inspect "@nuclescript/presets"
```

Manifest validation rules enforced:
- **Core Fields**: Non-empty `name`, `version`, and `import` source.
- **Export Entries**: Each export must specify a non-empty `name`, `description`, and a known `kind` (`PoolSchema`, `Pipeline`, `RecoveryProfile`, or `Function`; snake-case spellings and `fn` are accepted aliases).
- **Resolver Verification**: Each exported item must successfully resolve against the package's internal presets structure.

Validate an import in a NucleScript program:

```bash
nucle plan docs/examples/preset_imports.nsl
```

The compiler validates package source names and exported symbols against a registry initialized from all installed package manifests, then expands each import from the package's embedded source (see "What an import does" above).
