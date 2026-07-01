# NucleScript Packages

NucleScript packages are reusable biological storage presets: pool schemas,
recovery profiles, and pipeline templates that can be imported by `.nsl` source
files.

The first released package is bundled with this repository.

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

Write or refresh `nucle.lock`, which records a SHA-256 checksum of every
registered package's manifest:

```bash
nucle package lock
```

Verify a package's manifest shape (non-empty name, version, and exports, with known export kinds) and, if `nucle.lock` exists, that its checksum still matches:

```bash
nucle package verify "@nuclescript/presets"
```

Inspect a package's details, version, repository, and exported items, returning manifest fields and exports:

```bash
nucle package inspect "@nuclescript/presets"
```

Manifest validation rules enforced:
- **Core Fields**: Non-empty `name`, `version`, and `import` source.
- **Export Entries**: Each export must specify a non-empty `name`, `description`, and a known `kind` (`PoolSchema`, `Pipeline`, or `RecoveryProfile`).
- **Resolver Verification**: Each exported item must successfully resolve against the package's internal presets structure.

Validate an import in a NucleScript program:

```bash
nucle plan docs/examples/preset_imports.nsl
```

The current compiler validates package source names and exported symbols at compile time against a registry initialized from all installed package manifests. Registry-backed package expansion can build on the manifest shape without changing NucleScript source syntax.
