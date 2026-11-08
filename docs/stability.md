# NucleScript 1.0 and What 1.x Keeps

NucleScript has a language version of its own: **1.0**. The Rust crates
that implement it (`nucle_lang`, `nucle-cli` and the rest) stay at 0.x and
can change their APIs between releases; the language does not. `nucle
--version` prints both (`nucle 0.1.0 (NucleScript 1.0)`), `nucle check
--json` reports `"language": "1.0"`, and code can read
`nucle_lang::LANGUAGE_VERSION`.

## What 1.0 is

- the syntax in [grammar.md](grammar.md), its EBNF and the sections after it;
- the type rules: pools typed by profile and error rate, `Result`, enums,
  functions, generics over a pool's profile, closures and their effect
  ceilings;
- the effect rules and which operations need which confirmation
  ([effects.md](effects.md), [soundness.md](soundness.md));
- the diagnostic codes and levels in [errors.md](errors.md);
- what each operation does when a program runs: `store`, `retrieve`,
  `delete`, pipelines, `retain` requirements and `decode reads`;
- the shape of `nucle check --json`.

## What every 1.x release keeps

- A program 1.0 accepts is still accepted: no new errors.
- It does the same thing when it runs, and each declaration has the same
  effect and needs the same confirmations.
- Each diagnostic code keeps its meaning and level.

## What 1.x may change

- **Additions:** new syntax, operations, options, standard-library
  functions, diagnostic codes, and warnings on programs 1.0 accepted. A
  program 1.0 rejected may become valid.
- **Soundness fixes:** if 1.0 accepts a program only because of a checker
  defect that lets an operation past its confirmation or effect ceiling
  (as the ones in [soundness.md](soundness.md) did), a 1.x release may
  reject it. That is the one exception to "no new errors", and each such
  fix is logged below.
- **Wording:** diagnostic messages, `nucle explain` prose and CLI text
  other than `--json` output.
- **Numbers that come from measurement:** the simulator's noise profiles
  and the recovery, decay and cost figures built on them, which follow new
  data.

Anything else needs NucleScript 2.0.

## How this is checked

`nucle_lang/tests/language_1_0.rs` holds the promise in CI.

- `tests/conformance/1.0/` is a frozen copy of the 33 example programs
  that shipped with 1.0; `docs/examples` can go on changing.
- For each program, `expected.txt` records whether it is accepted, its
  diagnostic codes, and the effect of each top-level declaration. Every
  release must check all 33 to the same record.
- A digest of the EBNF in [grammar.md](grammar.md) is pinned, so the
  syntax cannot change without the test being updated and the change
  logged here.

A change the rules above allow, such as a new warning on a corpus
program, is logged below and recorded by running the test with
`NSL_BLESS=1`.

## Log

- **8 November 2026, NucleOS v0.1.66:** soundness fix. A closure returned
  from a function, or bound to one through an unannotated `Fn` type, no
  longer fits an effect ceiling, since the checker cannot see what it
  does; programs that passed one into a `confirm ...` slot are now
  rejected. `nucle explain` reports calls through such a closure with every
  effect. None of the 33 programs in the 1.0 corpus changed.
- **1.0** (7 November 2026): declared at NucleOS v0.1.61, with `decode
  reads`, per-instantiation checks of generic bodies, effect ceilings held
  to every kind of effect, and the effect rules in
  [soundness.md](soundness.md).
