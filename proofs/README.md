# Proofs

`NucleScriptCore.lean` writes NucleScript's effect and confirmation rules
down in Lean 4 for a core of the language, and proves that a program they
accept keeps three of the checker's promises.
[docs/soundness.md](../docs/soundness.md#the-machine-checked-core) says what
the core holds and what it leaves out.

| Theorem | Promise |
|---|---|
| `program_confirmed` | no operation runs without the confirmation it needs |
| `program_effects_sound` | no operation has an effect the rules did not report |
| `program_ceilings_sound` | no operation runs inside a closure call whose ceiling it exceeds |

`Checker.lean` turns the rules into a checker Lean can run. `check`
computes a statement's effects, or rejects it, and `check_sound` proves its
answer is a derivation of the rules, so `accept_effects_sound`,
`accept_ceilings_sound` and `accept_confirmed` hold for every program it
accepts. `Main.lean` builds it into `nsl-check`, which reads programs one
per line and prints `accept <effect>` or `reject` for each.

`nucle_lang/tests/lean_differential.rs` generates programs in the core,
writes each as NucleScript and for `nsl-check`, and fails if the Rust
checker and the Lean checker disagree about any of them.

To check the proofs and run the comparison, with
[elan](https://github.com/leanprover/elan) installed:

```bash
cd proofs
lake build
lake env lean Axioms.lean
cd ..
NSL_LEAN_CHECKER=proofs/.lake/build/bin/nsl-check cargo test -p nucle_lang --test lean_differential -- --nocapture
```

`lake build` fetches the toolchain named in `lean-toolchain` (Lean 4.35.0)
and needs no other package. `Axioms.lean` prints the axioms each theorem
rests on: `propext`, and `Quot.sound` for the checker's (through function
extensionality). CI fails if `sorryAx` ever appears.
`NSL_DIFFERENTIAL_PROGRAMS` sets how many programs the comparison runs
(1,000 by default).
