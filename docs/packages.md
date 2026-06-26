# NucleScript Packages

NucleScript packages are reusable biological storage presets: pool schemas,
recovery profiles, and pipeline templates that can be imported by `.nsl` source
files.

The first released package is bundled with this repository.

## @nuclescript/presets 0.1.0

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
nucle packages
```

Validate an import in a NucleScript program:

```bash
nucle plan docs/examples/preset_imports.nsl
```

The current compiler validates package source names and exported symbols at
compile time. Registry-backed package expansion can build on the manifest shape
without changing NucleScript source syntax.
