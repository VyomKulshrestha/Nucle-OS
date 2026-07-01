# @nuclescript/profiles

Reference hardware profile presets for Illumina, Nanopore, and Twist —
recommended pool defaults and reusable simulation functions matching
NucleOS's own optimizer recommendations and measured error rates.

```nuclescript
import {
    illumina_standard,
    nanopore_standard,
    twist_standard,
    recommended_pool_for_illumina
} from "nuclescript/profiles"
```

## Exports

| Export | Kind | Purpose |
|---|---|---|
| `illumina_standard` | Pool schema | Illumina defaults at 4x redundancy (low coverage) |
| `illumina_high_coverage` | Pool schema | Illumina defaults at 3x redundancy (10x+ coverage) |
| `nanopore_standard` | Pool schema | Nanopore defaults at 8x redundancy (low coverage) |
| `nanopore_high_coverage` | Pool schema | Nanopore defaults at 6x redundancy (10x+ coverage) |
| `twist_standard` | Pool schema | Twist defaults at 2x redundancy |
| `recommended_pool_for_illumina` | Function | Simulate a pool under Illumina, returning its 0.35% error binding |
| `recommended_pool_for_nanopore` | Function | Simulate a pool under Nanopore, returning its 5.00% error binding |
| `recommended_pool_for_twist` | Function | Simulate a pool under Twist, returning its 0.03% error binding |

## Package

- GitHub package: `@nuclescript/profiles`
- Import source: `nuclescript/profiles`
- Version: `0.1.0`
- Source: `src/profiles.nsl`

Redundancy values mirror `nucle_lang::middle::recommended_redundancy`; error
rates mirror `nucle_lang::probabilistic::profile_error_rate_percent`. If
either changes in the compiler, this package's presets should be updated to
match.
