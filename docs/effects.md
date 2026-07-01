# NucleScript Effect Model

Every NucleScript expression and operation is classified with one of four
effects. The classification isn't decorative — the compiler uses it to
decide what needs an explicit confirmation token before it's allowed to
compile, and `nucle explain` uses it to describe what a program will
actually do before you run it.

## The four effects

| Effect | Meaning | Confirmation required? |
|---|---|---|
| `Pure` | No hardware interaction, no data loss. Reads, in-memory computation, simulation. | No |
| `Synthesis` | Writes DNA to a physical synthesizer. Costs money and lab time. | Yes — `confirm hardware` |
| `Sequencing` | Reads DNA from a physical sequencer. Costs money and lab time. | Yes — `confirm hardware` |
| `Destructive` | Permanently deletes physical material. Cannot be undone. | Yes — `confirm physical_key` |

Source: `nucle_lang/src/ast.rs`'s `Effect` enum; classification logic lives in
`nucle_lang/src/effects.rs`.

## Where effects come from

- `simulate`/`consensus_vote` are always `Pure` — they never touch hardware.
- `synthesise ... via <profile>` is `Synthesis`; `sequence ... via <profile>`
  is `Sequencing`. Both require `confirm hardware` in source, checked by
  `expr_has_required_confirmation`.
- `store` is `Synthesis` unless written as `simulate store ...`, which is
  `Pure` (it never reaches real hardware).
- `retrieve` is always `Pure`.
- `delete` is always `Destructive` and requires `confirm physical_key`.
- A **function's** effect is the *join* of every declaration in its body
  (`join_effects` in `effects.rs`): if any statement in the body is
  `Destructive`, the whole function is `Destructive`, and so on down the
  precedence `Destructive > Synthesis > Sequencing > Pure`.

## Effects propagate through function calls

Calling a function is not automatically `Pure` or pre-confirmed just because
it's wrapped in a function. A `let` binding that calls a function inherits
that function's real effect and confirmation state:

```nuclescript
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }

fn purge() returns Void {
    delete "old_archive.bin" from archive
}

let result: Void = purge()
```

This fails type checking with **two** errors — the delete's own missing
confirmation, and the call site's:

```
error: delete 'old_archive.bin' from 'archive' has Destructive effect and requires explicit physical key confirmation
error: binding 'result' has Destructive effect and requires explicit hardware confirmation
```

Adding `confirm physical_key` to the `delete` inside `purge()` (not at the
call site — the confirmation lives where the effect originates) makes both
errors disappear. A (mutually) recursive function is resolved conservatively:
if the effect can't be fully determined because it depends on itself, it's
treated as `Destructive` and unconfirmed rather than silently `Pure`.

See `docs/examples/failures/unconfirmed_function_delete.nsl` for a runnable
version of this example, and `nucle_lang/tests/functions.rs` for the
corresponding tests (`test_calling_destructive_function_requires_confirmation`,
`test_calling_confirmed_destructive_function_passes`).

## What surfaces this

- **`nucle check --json`** — every confirmation failure is a `Diagnostic`
  with `level: "error"` in the `diagnostics` array; `ok` is `false` if any
  exist. This is the same `typeck::check_program` pass that `nucle run`
  and `nucle plan` use internally.
- **`nucle explain <source.nsl>`** — prints an effect summary for every
  top-level declaration (`nucle_lang::effects::effect_summary`), including
  functions: each entry shows its effect and one of `SAFE (Pure)`,
  `CONFIRMED`, or `REQUIRES CONFIRMATION`, plus a plain-language warning for
  anything unconfirmed (see `nucle_lang/src/diagnostics.rs`). It also
  explains *why* the optimizer changed something, e.g. a redundancy bump for
  an under-provisioned profile/coverage combination.

```bash
nucle explain docs/examples/failures/missing_confirmation.nsl
nucle explain docs/examples/critical_redundancy_warning.nsl
```
