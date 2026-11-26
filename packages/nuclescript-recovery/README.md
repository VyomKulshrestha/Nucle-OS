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

Consensus error rates follow
`nucle_lang::probabilistic::consensus_error_rate_percent`: the source's rate
times the share of error the consensus decoder left on real MinION and
Illumina reads at that coverage. An Illumina source's 0.35% becomes
0.013% at 10x and 0.0074% at 20x; see
[docs/architecture.md](../../docs/architecture.md#measured-consensus-curve).
