# Soundness of the Type and Effect Rules

The NucleScript checker makes four promises about any program it accepts.
This page states them, gives the rules they rest on, and describes how they
are checked: by running programs under a monitor that compares what each
one did with what the checker said it would do, and by a proof, checked by
Lean, that the rules keep all four promises for a core of the language.
The last section says where each stops.

## The promises

1. **Confirmation.** No `delete` runs unless its own source carries
   `confirm physical_key`, and no `synthesise`/`sequence` compiles without
   `confirm hardware` (or the narrower `confirm synthesis` /
   `confirm sequencing`).
2. **Effect reports.** No top-level declaration performs an operation with
   a worse effect than `nucle explain` reports for it, in the order
   `Pure < Sequencing < Synthesis < Destructive`.
3. **Effect ceilings.** A closure bound into a `Fn(...) -> T confirm
   hardware` slot never deletes while it runs, one bound into a
   `confirm synthesis` slot only synthesises (or stores), one bound into
   a `confirm sequencing` slot only sequences, and one bound into a
   `confirm physical_key` slot never stores, synthesises or sequences.
4. **Value shapes.** Every value bound to an annotated name, passed as an
   argument or returned from a function has a shape its declared type
   allows, and `?` and `match` only ever meet the values they are typed
   for.

## The effect rules

An expression or declaration may perform any of three effects: Synthesis,
Sequencing and Destructive (Pure is the empty set). `nucle explain` and the
confirmation check report the worst one. Ceilings need the whole set,
because a closure that stores and deletes needs both confirmations: its
worst effect, Destructive, fits `confirm physical_key`, but its store does
not.

Write `Γ ⊢ d : E` for "in scope Γ, `d` may perform the effects in `E`".
Γ holds the program's functions, the closures `let`-bound so far in the
enclosing blocks, and the ceilings of any annotated `Fn(...)` parameters.

| Form | Effects |
|---|---|
| `store f into p` | {Synthesis} |
| `simulate store f into p`, `retrieve`, `simulate p under P`, `assert`, pool, strand, sequence and enum declarations | ∅ |
| `delete f from p` | {Destructive} |
| `synthesise ...` | {Synthesis} |
| `sequence ...` | {Sequencing} |
| `pipeline` with a `store into` step | {Synthesis} |
| a closure literal `fn(...) -> T { ... }` | ∅: defining one runs nothing |
| `f(a₁, ..., aₙ)`, `f` a function or a closure in Γ | E(body of `f`) ∪ E(aᵢ) for each closure aᵢ passed to an unannotated `Fn` parameter |
| `p()`, `p` a parameter typed `Fn(...) -> T confirm k` | allowed(`k`), below: {Synthesis, Sequencing} for `hardware` |
| `p()`, `p` an unannotated `Fn` parameter | ∅; the call that passed the closure adds its effects (row above) |
| `g()`, `g` a closure the analysis cannot trace: returned by a function, taken out of an enum, or bound to either through an unannotated `Fn` type | every effect |
| `?`, `Ok`, `Err`, enum constructors, `!`, binary operators | the effects of their operands |
| `match`, `if` | the union over the scrutinee and every arm or branch, taken or not |
| `for` | the effects of its body |
| a block `d₁ ... dₙ` | the union of E(dᵢ), where `let x = fn ...` in dᵢ puts `x` in Γ for dᵢ₊₁ ... dₙ, `let x = y` puts `x` in Γ as whatever `y` is, and `let x: Fn(...) -> T confirm k = e` puts `x` in Γ with ceiling `k` |

A recursive call contributes ∅ to its own component, which gives the least
fixed point: a recursive function that deletes is still Destructive, and a
pure one stays Pure.

**Confirmation.** Each operation that needs confirmation carries it on its
own line: `confirm physical_key` on a `delete`, `confirm hardware` on
`synthesise` and `sequence` (or `confirm synthesis` on `synthesise` and
`confirm sequencing` on `sequence`). A call is confirmed when every operation its
effects come from is. `store` needs no confirmation; the checker warns
about real (not simulated) stores instead.

**Ceilings.** Binding a closure `c` into a slot with ceiling `k` (passing
it to an annotated parameter, or `let`-binding it with an annotated type)
requires E(c) ⊆ allowed(k), where allowed(`hardware`) = {Synthesis,
Sequencing}, allowed(`synthesis`) = {Synthesis}, allowed(`sequencing`) =
{Sequencing} and allowed(`physical_key`) = {Destructive}, and requires every
operation in `c` to be confirmed. Here a call through an unannotated `Fn`
parameter counts as every effect, not ∅: whatever closure a caller passes
for it would run under `k`, so `c` fits only if it calls none. The checker
reports `E-FN-EFFECT-ARG-MISMATCH` otherwise. Calling the parameter can
then be trusted to stay within `k` without seeing the closure, because
every closure that can reach it was checked where it was bound.

The rules live in `nucle_lang/src/effects.rs` (`expr_effects`,
`block_effect`, `EffectSet`) and in `typeck::check_fn_effect_compatibility`.

## The value rules

Only some types have runtime values; the rest exist only while checking.

| Type | Runtime value |
|---|---|
| `Result<T, Str>` | `Ok(v)` with `v` a `T`, or `Err(message)` |
| `Void` | nothing (a `delete`'s `Ok` carries what it removed, which counts) |
| `Str` | a string |
| `File`, `DnaFile` | a file name, or a stored file |
| `Pool<...>` | a declared pool, or nothing for a pool that exists only at compile time |
| `Fn(T₁, ..., Tₙ) -> T` | a closure taking `n` arguments |
| `enum E` | an instance of `E` |
| `Strand`, `Sequence`, `Recovery` | compile time only |

`store` has type `Result<DnaFile, Str>`, `delete` `Result<Void, Str>`. `?`
applies only to a `Result`, and only inside a function returning one. A
`match` covers every variant of its scrutinee or has a wildcard. A function
declared `returns Void` returns nothing, whatever its last binding holds.

## How the promises are checked

`execute_program` records what a program does while it runs
(`codegen::RunTrace`, read with `take_run_trace`):

- every store and delete that reaches the VFS, with its effect, whether its
  source carried its confirmation, and which top-level declaration was
  running;
- a fault whenever a value fails its declared type (`Value::conforms_to`)
  at a binding, argument or return, including a declared pool whose profile
  is not the one a `Pool<Profile>` names, and whenever `?` or `match` meets
  a value it should not;
- a fault whenever an operation runs inside a closure call whose ceiling it
  exceeds. A closure takes on the ceiling of every slot it is bound into,
  and the interpreter keeps the ceilings of the closure calls in progress.

`nucle_lang/tests/soundness.rs` runs programs and checks each trace against
the promises: no unconfirmed delete ran, no declaration exceeded its
reported effect, and no fault was recorded. It runs two sets of programs:

- every shipped example in `docs/examples` that compiles: 30 of the 31,
  which ran 48 stores and deletes between them;
- 300 programs generated from seeds 0 to 299 in CI, from a generator
  (`Gen` in the same file) that writes:
  - functions, closures held in `let`s, and higher-order functions with
    and without ceilings, passed closures inline or by name that call other
    functions;
  - closures returned from functions, and closures passed on under a
    second name, both called directly and handed to higher-order
    functions;
  - functions generic over a pool's profile, functions taking a
    `Pool<Illumina>` that are also called with a Twist pool, and functions
    that recurse until the call-depth limit stops them;
  - `Result` functions using `?` and `match`, `if`, `for`, pipelines and
    stores with `retain` requirements;
  - an enum whose `match` picks which function runs, an enum that carries
    a closure (called from its arm, or handed to a higher-order function
    whose ceiling it cannot meet), a function imported from the
    `nuclescript/presets` package, and `test` blocks, one in ten of which
    asserts a bound that does not hold;
  - loops over the files a pool holds while the program runs, deleting
    each one or copying it into a pool.

  About one delete in twenty-five drops its confirmation, and about one
  closure in twenty is given a ceiling its body does not fit, so the
  checker's rejections are exercised too. Some functions take a closure
  without a ceiling and pass it on to one that has a ceiling, by name or
  inside a closure of their own. Of the 300, 209 were accepted; they ran
  2,637 stores and 369 deletes.

A longer run of 5,000 programs (`NSL_SOUNDNESS_PROGRAMS=5000`) on 26
November 2026 accepted 3,384, ran 44,988 stores and 5,325 deletes, and broke
no promise. The reports are also close to exact: of the 42,711 top-level
declarations that ran an operation, 322 were reported with a worse effect
than they performed, most of them calls through a closure the analysis
cannot trace. The rules allow that: every arm and branch counts
whether it runs or not, and a run that stops early never reaches its later
operations.

## What it found

The monitor's first run, before any of the fixes below, failed on five
shipped examples and 159 of the 300 generated programs. Each defect is now
fixed and pinned by a test in `nucle_lang/tests/soundness_regressions.rs`;
all five tests fail on the code before the fixes.

- **Pipelines were reported Pure.** A pipeline with a `store into` step
  writes for real, but `nucle explain` called it Pure. It is Synthesis now.
- **Closures bound inside a body were invisible.** The effect walk resolved
  a call to a `let`-bound closure only at the point where the checker was
  standing, not inside a function it called or later at top level, so a
  function that deleted through its own closure was reported Pure. The
  walk now carries the closures in scope through every block.
- **A delete could pass a `confirm hardware` ceiling** through such a
  function, since the ceiling check used the same walk.
- **A store could pass a `confirm physical_key` ceiling** when the same
  closure also deleted, because the check compared only the worst effect.
  Ceilings now compare the whole set.
- **A `Void` function returned its last binding.** A function declared
  `returns Void` whose body ended on a `Result` binding handed that
  `Result` to its caller at run time. It returns nothing now.

Three more turned up later, while writing down the core calculus for the
proof below. Two came from seeing that a closure can leave the scope the
analysis watches; the generator was then extended to make such programs,
and on the code before those fixes it flagged 70 of its 300. The third is
a case the calculus's ceiling rule has to settle: what a parameter without
a ceiling may do inside one.

- **A closure returned from a function could do anything unnoticed.** A
  call through a closure the analysis could not trace (one returned by a
  function, or bound through an unannotated `Fn` type to one) counted as
  Pure. `nucle explain` reported such calls as Pure, and the closure fit
  every ceiling, so a function could hand back a deleting closure that
  then ran inside a `confirm hardware` slot. Such a call now has every
  effect and fits no ceiling unless its `Fn` type names one. Another name
  for a known closure resolves to that closure, and calls to imported
  functions resolve to their bodies.
- **A closure under a second name was missing at run time.** At top
  level, `let again: Fn() -> Void = wipe` (or `let name: File =
  "a.txt"`) was never evaluated, so calling `again()` found nothing to
  call. It is evaluated now.
- **A parameter without a ceiling could be passed into one.** Inside `fn
  relay(action: Fn() -> Void)`, handing `action` to a `confirm hardware`
  parameter, or calling it inside a closure handed there, was accepted,
  because a call through `action` counted as ∅. A caller could then give
  `relay` a deleting closure, and it ran under the hardware ceiling. Where
  a ceiling is checked, such a parameter now counts as every effect; the
  generator, once it learned to write `relay`, found one program of its
  300 that broke the ceiling on the code before the fix.
- **A closure handed to a parameter was never counted.** Inside `fn
  apply(g: Fn(Fn() -> Void) -> Void)`, a call `g(wipe)` added nothing for
  `wipe`. The closure `g` holds counts its own parameter's calls as ∅ and
  leaves them to the call sites that pass it a closure, but `wipe` is built
  inside `apply`, so no call site saw it. `nucle explain` called
  `apply(fn(k: Fn() -> Void) -> Void { k() })` Pure while it deleted, and
  the delete ran inside a closure accepted under `confirm hardware`. A call
  through a parameter, or through a closure the checker cannot see into
  (one returned by a function, taken out of an enum, or bound to one),
  now counts every closure passed to it. This one was found by writing the
  Lean rules for closures with arguments of their own, which said the call
  must count them.
- **A closure bound by `let` that called its argument fit a ceiling.** Held
  to a ceiling, a closure literal's own parameters without one count as
  every effect, since whatever it is passed runs under the ceiling. Bound
  to a name first, `let relay: Fn(Fn() -> Void) -> Void = fn(k: Fn() ->
  Void) -> Void { k() }`, the same closure had them counted as ∅ and fit
  `confirm sequencing`, so a function holding it there could hand it a
  delete. Both paths now count them. The comparison with the Lean rules
  found this one (below).

Each is pinned in `soundness_regressions.rs` too, and those tests also
fail on the code before the fixes.

## The machine-checked core

[proofs/NucleScriptCore.lean](../proofs/NucleScriptCore.lean) writes the
effect rules above down in Lean 4 for a core of the language and proves
that a program they accept keeps all four promises there. The core has:

- operations, each with its effect and whether its confirmation was
  written;
- sequencing, and branches of which either side may run (`if`, `match`);
- closures bound by `let`, with or without a ceiling, taking closures as
  arguments of their own, each with or without a ceiling of its own, and
  other names for them;
- closures the rules cannot see into, such as one returned by a function,
  with or without a ceiling on the type they come through;
- calls to closures, passing them closures, and named functions that take
  closures as arguments, with or without ceilings, recursion included;
- loops over a pool's files, which run their body zero or more times;
- an early return through `?`, which ends the function or closure it is in,
  so nothing after it there runs;
- declared pools, passed to functions that take a pool of one profile or
  are generic over it (`fn keep<P>(source: Pool<P>)`);
- the `Result` a `store` or `delete` gives back, or a function returning
  one, unwrapped by `?`;
- enum values, whose variants carry a plain value such as a `Str` or
  nothing;
- `match` on an enum or a `Result`, with an arm for each variant or a
  wildcard arm.

Running a statement gives the list of operations it performed, each with
its effect, whether it carried its confirmation, and the ceilings of the
closure calls in progress: what the monitor's trace records. The rules give
each statement two things: R, the effects it may have itself, and U, the
parameters without a ceiling whose closures it may call. R is what `nucle
explain` reports. A call adds the effects of the closures it passes for U,
and a closure fits a ceiling only when its U is empty and its R is allowed,
the rule the parameter defect above broke. A closure's own parameters are
in its U when it calls them, so one that calls its arguments fits no
ceiling. A call to a closure adds what the closures it passes do when the
closure may call them: one with parameters of its own, or a parameter's
closure, may; a closure held to a ceiling may not. A closure's own
parameter with a ceiling works like a function's: what it is passed must
fit, a call through it may do what the ceiling allows, and a closure that
calls it can still fit a ceiling that allows as much. A call to a closure
the rules can see counts only what it passes to parameters without a
ceiling; a call through a parameter counts everything it passes, as the
Rust checker does. A closure's parameter may reuse any name in scope and
then hides it; a closure in scope that
runs the outer parameter of that name counts, inside the body, as doing
anything, since the name there means the closure's own parameter. The
Rust checker resolves such a call to the inner parameter instead, and
still counts the outer parameter's closure at the call that passed it in.
The generator below reuses names only where no closure in scope runs the
outer one, so the two checkers agree there; elsewhere the core is the
stricter of the two. A function's effect may be any set
its body's R fits in; the least fixed point the checker computes is one.

| Theorem | Promise |
|---|---|
| `program_confirmed` | 1: every operation that ran carried its confirmation |
| `program_effects_sound` | 2: every operation's effect is in R |
| `program_ceilings_sound` | 3: every operation is allowed by the ceiling of each closure call it ran inside |
| `program_shapes_sound` | 4: no run reaches a call it cannot make, a value that breaks its declared type, or a `?` or `match` that meets a value it cannot take |

A value's shape is its type: for a closure, the ceilings of the closures
it takes; for a pool, its profile; for a `Result` or an enum value, the
variants it may be. A program is shaped when every call passes values of
the types its parameters want, every `let` gives its name the type of its
value, `?` meets only a `Result`, and `match` meets only an enum or a
`Result`, with an arm for each variant it may be (an arm that did not
match rules its variant out for the arms after it). In the core a run
checks values at a call's arguments and at the name `?` binds, and there
a generic `Pool<P>` takes any pool, since a running program no longer
knows `P`. `program_shapes_sound` proves that a shaped program
never reaches a call to a name with no value or to something that is not
a closure, arguments that do not conform to their parameters' types, an
unknown function, a value that does not conform to the type `?` binds it
to, a `?` on something other than a `Result`, or a `match` none of whose
arms fits. A generic function's body is checked once, with `P` abstract,
and the proof covers it for every profile a call gives `P`: the call
carries the profiles the checker inferred.

The effect rules see a function's parameter whose type is not a closure's
as doing nothing. That is sound because a run checks what each call
passes, so such a parameter never holds a closure. What `?` or a `match`
arm takes out of a value counts as doing anything, as a closure the rules
cannot see into does; in the core it is a plain value, so it never runs.

The proof needs two invariants the rules do not state. A closure value
does at most what the binding holding it says, given what the enclosing
function's parameters hold and, when it is called, what it is passed, and
calls none of its arguments if its binding says so (`ValOk` and `EnvOk`).
And every name holds a value of its type, with each type parameter
standing for the profile the call gave it (`Has` and `EnvShaped`).
Lean checks the file in a few seconds, the theorems rest on Lean's
standard axioms `propext` and `Quot.sound` (function extensionality), and
CI rebuilds the proof and fails if any step rests on `sorry`.
[proofs/README.md](../proofs/README.md) says how to check it.

### Checking the Rust checker against the rules

The Lean rules were written by hand from the Rust code, so the two could
part without anyone noticing. `proofs/Checker.lean` makes the rules
something Lean can run: `check` computes a statement's R and U, or rejects
it, and `check_sound` proves that its answer is a derivation of the rules,
so the three theorems hold for every program it accepts
(`accept_effects_sound`, `accept_ceilings_sound`, `accept_confirmed`).
`nsl-check`, built from `proofs/Main.lean`, runs it on programs written one
per line.

`nucle_lang/tests/lean_differential.rs` generates programs in the core
(functions taking closures with and without each ceiling, some of them
closures that are passed a closure, closures with and without arguments
of their own, those arguments with and without ceilings, second names
for them, closures returned from functions, branches, loops over a pool's
files, two pools of different profiles passed to functions that take one
profile or are generic over it, the `Result`s of stores, deletes and
functions returning one, unwrapped by `?` in functions that return a
`Result`, values of an enum, `match`es on those and on `Result`s with an
arm for each variant or a wildcard, and every operation with and without
its confirmation; now and then a call passes one closure too many or a
pool of the wrong profile, a `?` declares the wrong type, or a `match`
leaves an arm out), writes each as NucleScript and for `nsl-check`, which
also checks shapes, and fails if the two checkers differ on whether a
program is accepted or on its worst effect. CI runs 1,000 on every push.
A run of 20,000 on 26 November 2026 accepted 10,580 and found no
disagreement. Of the 20,000, 4,540 match on an enum or a `Result` (2,153
accepted), 447 use `?` (212), 756 call a generic function (279) and 2,449
take a pool as a parameter (889). Against the Rust checker as it was at
v0.1.85, before the two closure fixes above, the generator as it was then
gave 3 disagreements in 20,000, each a program that checker accepted and
the rules reject.

Its first run found 40 disagreements, all programs the Lean rules accept
and the Rust checker rejected. Beyond mistakes in how the test wrote
programs, they came down to two places where `typeck.rs` had drifted from
the rules and from `effects.rs`, neither of them unsound:

- **A `Fn` parameter could not be given a second name.** `let again:
  Fn() -> Void = action` reported `action` as an undeclared variable,
  because only `let`s were looked up. It is accepted now, and the second
  name must repeat the parameter's type, ceiling included.
- **A second name declared with a ceiling lost what its closure does.**
  `let again: Fn() -> Void confirm hardware = keep` was held to the whole
  ceiling, so a store-only `keep` no longer fit `confirm synthesis` under
  its second name. It now stands for exactly the closure it names, as it
  always did in `effects.rs`.

Once the generator wrote closures that take arguments, it found two more,
one on each side:

- **A closure bound by `let` that called its argument fit a ceiling**, the
  unsound acceptance above.
- **The Lean rule counted too much.** It added the closures passed to a
  call through a slot with a ceiling, which cannot run them, since a
  closure that calls its arguments fits no ceiling. The Rust checker had
  this right, and the rule now matches it.

Writing the rules for pools, `Result`s and enums, and generating programs
that use them, found three more:

- **A `let` annotated with a type parameter broke every call.** A generic
  function's body is checked again with each call's profile, to report
  what only a concrete profile raises, and that check compared `let held:
  Pool<P> = source` with the annotation as written, so every call to the
  function was rejected. It now reads `P` as the call's profile.
- **An enum parameter could not be given a second name.** `let again: Job
  = job` reported `job` as undeclared when it was a parameter, as a `Fn`
  parameter once did. 151 of the 20,000 programs hit it before the fix.
- **The Lean rule took a pool parameter for a closure.** All of a
  function's parameters were bound as closure parameters, so passing a
  pool parameter on from inside a closure with a ceiling counted as
  calling a closure no one had seen, and the closure fit no ceiling. 3 of
  20,000 programs hit it. The Rust checker had this right; the rule now
  sees a parameter whose type is not a closure's as doing nothing.

## What this does not show

- **The proof covers a core, not the implementation.** Its rules were
  written by hand from `effects.rs` and `typeck.rs`. The two are tied by
  testing, not by proof: the differential test checks that the Rust
  checker gives the rules' verdict on generated core programs, which
  catches a slip only where the generator reaches. The core's functions
  return nothing: what a call to a function returning a `Result` gives
  back is taken to be a `Result` of its declared type, so the shape of a
  returned value rests on testing. It also leaves out `Ok(...)` and
  `Err(...)` written out, enum variants carrying anything but a plain
  value (a closure carried by an enum is still a closure the rules cannot
  see into), closures that take a pool, generic closures (`fn<P>(...)`),
  a second name for a pool or a `Result`, and `Strand`, `Sequence` and
  `File` values.
- **The generator leaves pool expressions out** (`consensus_vote`,
  `synthesise`, `sequence`): they never run, existing only while
  checking, so the monitor has nothing to watch. Their rules are covered
  by the ordinary tests. `test` blocks are generated, but they never run
  either; the monitor only shows that the checker handles them alongside
  everything else.
- **Error-rate types come from measurement, and the monitor cannot check
  them.** A pool's type carries an error rate (`Pool<Illumina, 0.35%>`):
  the profile's published rate, and after `consensus_vote` that rate times
  the share of error the consensus decoder left on real reads at that
  coverage ([architecture.md](architecture.md#measured-consensus-curve)).
  The runtime has no pool values to compare them with. The consensus curve
  is pinned by `nucle_lang/tests/consensus_curve.rs` instead, which reruns
  the decoder on the MinION sample in the repository.
- **The monitor checks shapes and pool profiles, not every type.** A
  declared pool is checked against a concrete `Pool<Profile>`, but a
  generic `Pool<P>` is erased by the time a program runs, and `Strand`,
  `Sequence` and `Recovery` have no values at all.
- **`store` needs no confirmation**, by design (see
  [effects.md](effects.md)); the checker warns about real stores
  instead.
- **The VFS only.** Hardware submission (`nucle hardware export`) has its
  own confirmation gates, tested separately; see
  [hardware-providers.md](hardware-providers.md).
