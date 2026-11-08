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
  `Pure` (it never reaches real hardware). A `pipeline` with a `store into`
  step is `Synthesis` too: it stores for real.
- `retrieve` is always `Pure`, and so is `decode reads`, which reads two
  local files of sequencing reads and references.
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
errors disappear. Recursive and mutually recursive functions are resolved as
a least fixed point: a recursive back-edge contributes the neutral `Pure`
effect, while every concrete operation in the component is still joined. A
genuinely pure recursive component therefore remains `Pure`; a cycle that
contains a delete still resolves to `Destructive` and requires confirmation.

See `docs/examples/failures/unconfirmed_function_delete.nsl` for a runnable
version of this example, and `nucle_lang/tests/functions.rs` for the
corresponding tests (`test_calling_destructive_function_requires_confirmation`,
`test_calling_confirmed_destructive_function_passes`).

## Closures in scope

A call to a closure resolves to the closure's own body wherever the
closure was `let`-bound earlier in the same block, at top level or inside
a function, so a function that deletes through its own closure is
`Destructive` to its callers and in `nucle explain`. Another name for a
closure (`let again: Fn() -> Void = wipe`) resolves to the same closure.

A closure the analysis cannot trace, one returned by a function or taken
out of an enum, may do anything: a call through it has every effect, and
it fits no ceiling unless the `Fn` type it comes through names one
(`returns Fn() -> Void confirm hardware`). A `Fn` parameter without a
ceiling is the same inside a ceiling: a function cannot pass it on to a
`confirm hardware` parameter, or call it inside a closure it passes there,
because its caller may hand it a closure that deletes.

An effect ceiling (`Fn(...) -> T confirm hardware`) is checked against
every kind of effect a closure may have, not just the worst one: a closure
that stores and deletes needs both confirmations, so it fits neither
ceiling. See [docs/soundness.md](soundness.md) for the rules in full and
how they are checked by running programs.

## What surfaces this

- **`nucle check --json`** — every confirmation failure is a `Diagnostic`
  with `level: "error"` and a real `span: { line, column, end_line,
  end_column }` pointing at the unconfirmed declaration, in the
  `diagnostics` array; `ok` is `false` if any exist. This is the same
  `typeck::check_program` pass that `nucle run` and `nucle plan` use
  internally.
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
