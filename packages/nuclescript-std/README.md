# @nuclescript/std

The NucleScript standard library, written in NucleScript. Each function is
generic over the pool's profile and loops over the files the pool holds
when the call runs (`for f in files of pool`), so one definition serves
Illumina, Nanopore, Twist and IDT pools.

```nuclescript
import { clear, each_file, prune } from "nuclescript/std"

pool scratch: DnaPool { codec: Ternary, redundancy: 2x, profile: Nanopore }

store "sample_a.txt" into scratch
store "sample_b.txt" into scratch

// Deletes both files. The call is Destructive, like the delete inside.
let wiped: Void = clear(scratch)

// Runs the closure on each file; its delete still needs confirming.
let tidy: Void = prune(scratch, fn(f: File) -> Void {
    delete f from scratch confirm physical_key
})
```

## Exports

| Export | Kind | Purpose |
|---|---|---|
| `clear<P>(target: Pool<P>)` | Function | Deletes every file `target` holds |
| `each_file<P>(target: Pool<P>, action: Fn(File) -> Void)` | Function | Runs `action` on each file, in name order; the call has `action`'s effect |
| `prune<P>(target: Pool<P>, action: Fn(File) -> Void confirm physical_key)` | Function | `each_file` whose action may delete and nothing worse: a closure that synthesises or sequences is rejected where it is written (`E-FN-EFFECT-ARG-MISMATCH`) |

## Package

- GitHub package: `@nuclescript/std`
- Import source: `nuclescript/std`
- Version: `0.1.0`
- Source: `src/std.nsl`
- Tests: `nucle_lang/tests/std_package.rs` runs each function against a
  real VFS and checks its effect and ceiling.
