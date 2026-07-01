# @nuclescript/recovery

Consensus and recovery templates: reusable pool schemas and a function
wrapping the `simulate` → `consensus_vote` workflow used throughout NucleOS
to turn a noisy channel into a recovered, error-budgeted pool.

```nuclescript
import {
    recovery_archive,
    recover_with_consensus
} from "nuclescript/recovery"
```

## Exports

| Export | Kind | Purpose |
|---|---|---|
| `recovery_archive` | Pool schema | Illumina defaults at 4x redundancy for consensus workflows |
| `recovery_consensus_10x` | Recovery profile | Consensus-recovered binding at 10x coverage |
| `recovery_consensus_20x` | Recovery profile | Consensus-recovered binding at 20x coverage |
| `recover_with_consensus` | Function | Simulate a source pool under Illumina, then consensus-vote at 10x coverage |

## Package

- GitHub package: `@nuclescript/recovery`
- Import source: `nuclescript/recovery`
- Version: `0.1.0`
- Source: `src/recovery.nsl`

Consensus error-rate scaling follows
`nucle_lang::probabilistic::consensus_error_rate_percent` — doubling coverage
quarters the residual error rate (inverse-square scaling), matching how real
sequencing consensus behaves.
