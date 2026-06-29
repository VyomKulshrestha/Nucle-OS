# @nuclescript/presets

Reference NucleScript presets for archive-oriented DNA storage programs.

This is the first package published from the NucleScript ecosystem. It is
imported by source name:

```nuclescript
import {
    medical_archive,
    reliable_store,
    illumina_recovery
} from "nuclescript/presets"
```

## Exports

| Export | Kind | Purpose |
|---|---|---|
| `medical_archive` | Pool schema | Ternary + Illumina archive defaults with conservative redundancy |
| `twist_archive` | Pool schema | Twist-oriented archive defaults for low-error synthesis planning |
| `reliable_store` | Pipeline | Encode, protect, store, and verify with optimizer-visible redundancy |
| `illumina_recovery` | Recovery profile | Illumina consensus recovery defaults for planning and simulation |

## Package

- GitHub package: `@nuclescript/presets`
- Import source: `nuclescript/presets`
- Version: `0.1.0`
- Source: `src/presets.nsl`

The current compiler validates package imports and export names at compile
time. Future registry work can replace the built-in resolver with the package
manifest in this directory without changing NucleScript source programs.
