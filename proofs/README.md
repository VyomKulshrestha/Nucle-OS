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

To check them, with [elan](https://github.com/leanprover/elan) installed:

```bash
cd proofs
lake build
lake env lean Axioms.lean
```

`lake build` fetches the toolchain named in `lean-toolchain` (Lean 4.35.0)
and needs no other package. `Axioms.lean` prints the axioms each theorem
rests on; `propext` is the only one, and CI fails if `sorryAx` ever appears.
