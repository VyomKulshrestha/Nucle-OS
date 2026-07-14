# Nucle-OS Architecture

## Design Philosophy

Nucle-OS follows the same bottom-up layered architecture as FerrumOS(https://github.com/VyomKulshrestha/Ferrum-OS). Each layer is a separate Rust crate with well-defined responsibilities and clean interfaces to adjacent layers. Dependencies flow strictly upward — lower layers never depend on higher layers.

## Layer Dependency Graph

```
nucle_cli
    ├── nucle_agent
    │       └── nucle_vfs
    │               └── nucle_index
    │                       └── nucle_ecc
    │                               ├── nucle_codec
    │                               └── nucle_synth
    │                                       └── nucle_codec
    ├── nucle_hardware
    │       └── nucle_lang (below)
    └── nucle_lang
            ├── nucle_vfs
            │       └── nucle_index
            │               └── nucle_ecc
            │                       ├── nucle_codec
            │                       └── nucle_synth
            │                               └── nucle_codec
            ├── nucle_codec
            └── nucle_synth
```

Dependencies flow strictly downward. No layer ever imports from a layer above it.

## NucleScript Language Layer

`nucle_lang` is the NucleScript compiler crate. It sits above the VFS and turns
`.nsl` source files into NucleOS operations:

```text
NucleScript source (.nsl)
    → lexer
    → parser / AST
    → semantic + biological constraint checks, including
      compile-time if/for desugaring (typeck::check_and_desugar)
    → bio-aware MIR
    → redundancy/profile optimizer
    → VFS backend or simulation backend
    → NucleOS syscalls or no-hardware plan
```

The compiler currently supports declarative pool definitions, store/retrieve
operations, simulation options, pipeline programs, DNA-native `Sequence`
literals such as `seq"ATCGATCG-GCTAGCTA"`, probabilistic pool annotations
such as `Pool<Illumina, 0.35%>`, compile-time `if`/`for` control flow
with comparison/boolean operators (`==`, `!=`, `<`, `>`, `<=`, `>=`, `&&`,
`||`, `!`), and `Result<T, E>`/`?` for genuinely catchable VFS failures
(see below). Sequence literals are validated at compile time for DNA
alphabet, GC balance, homopolymer length, and hairpin-prone palindromes.
Probabilistic pool bindings are checked for profile/state compatibility and
propagate an error budget through consensus recovery. Effect checking
classifies operations as `Pure`, `Synthesis`, `Sequencing`, or
`Destructive`; hardware effects require `confirm hardware`, and destructive
effects require `confirm physical_key`. The MIR optimizer raises insufficient
redundancy for the selected profile and coverage before either executable VFS
lowering or no-hardware simulation planning.

`if`/`for` are resolved entirely inside `typeck::check_and_desugar`, before
any of that MIR/optimizer/backend machinery runs — before `Result<T, E>`/`?`
(below) existed, NucleScript's execution model was "compile a static plan, then run it,"
with no runtime branch or loop anywhere in a compiled program.
`check_and_desugar` evaluates each `if` condition once and keeps only the
taken branch (the untaken branch is never type-checked, closer to
`#[cfg(...)]` than a real conditional), and unrolls each `for` by
substituting the loop binding with every item in its literal list. The
result is a plain, control-flow-free `Program` — `middle` never sees an
`if`/`for` node at all. See
[docs/grammar.md](grammar.md#control-flow-if--for) for the full semantics
and a worked example.

### Structured error handling (`Result<T, E>` / `?`)

The one place NucleScript actually has a runtime, as opposed to a
compile-time-resolved plan. Before this, `store`/`retrieve`/`delete`
either succeeded or aborted the entire program via a bare `?` inside
`codegen::execute_program` — there was no way for a NucleScript program to
observe, inspect, or recover from an operation failure, and a function
call was purely a compile-time signature lookup (`typeck::TypeChecker::
infer_expr`'s `FunctionCall` arm), never something that actually ran.

`store`/`delete` can now also appear in *expression* position
(`Expr::StoreExpr`/`DeleteExpr`, reusing the exact `StoreOp`/`DeleteOp`
structs the statement form already carries — one grammar, two surface
positions), producing a `Result<T, Str>` a postfix `?` (`Expr::Try`) can
unwrap or propagate to the enclosing function's own `Result` return type.
`middle`/`MirOp` is deliberately untouched by this and stays that way —
it still has zero notion of control flow or function bodies. Instead,
`codegen.rs` gained a small, real interpreter (`eval_expr`/
`exec_function_body`/`call_user_function`, sharing a minimal `value::
{Value, EvalOutcome}` runtime representation with a lighter-weight
narrating counterpart in `sim_backend.rs`) that runs directly off the
already-desugared `Program`, executing a called function's body for the
first time in this compiler's history — Rust's own call stack serves as
NucleScript's; no bytecode VM was introduced. A function's own internal
`?` short-circuit resolves entirely within that function's call: the
caller always sees an ordinary, already-wrapped `Result` value at the
call site, never an automatic propagation of its own.

Backward compatibility here is a build-time guarantee, not a review-time
one: every `Expr`/`Declaration` match this touches has no wildcard arm,
so the new AST variants were a compile error in each of `effects.rs`,
`typeck.rs`, `middle.rs`, `docgen.rs`, and `nucle_lsp/src/backend.rs`
until explicitly handled. A golden-file regression test
(`nucle_lang/tests/result_backward_compat.rs`) additionally pins every
pre-existing example's execution output to what it was on the commit
before this feature landed. See the "Result / Error Propagation" section
of [docs/grammar.md](grammar.md) for the full semantics, plus "Pattern
Matching" below for `match`/`Ok`/`Err`.

**A real, pre-existing gap in this interpreter, closed only later**:
`exec_function_body` originally only ever processed
`Declaration::Let` — a bare statement-form `store`/`retrieve`/`delete`
inside *any* function body (present in the source, type-checked, never
producing the function's own return value) was silently skipped, never
reaching the real VFS, regardless of whether closures existed yet. Fixed
by adding a `Declaration::Operation` arm that reuses the same
VFS-executing helpers the top-level path already had; a statement-form
failure inside a function aborts that function unconditionally (the same
all-or-nothing contract the top-level form always had), which a caller
catches through the ordinary `?`/`match` machinery. Paired with making
`Expr::StringLiteral` a real, unconditional `Value::Str` (previously an
inert placeholder except inside `Err(...)`) — what makes a `File`/`Str`-
typed *parameter*'s argument become a real, bound value inside the
callee's own `env`, which `store <ident> into <pool>`'s existing "file
variable" syntax (accepted since it was written, never actually resolved)
now resolves through via a new `resolve_file_arg` helper.

The language layer now exposes ecosystem-facing integration points:

- `import { ... } from "nuclescript/presets"` validates built-in presets with
  the same resolver shape a package registry can extend.
- `analyze_source` returns serializable diagnostics, optimizer notes, simulation
  steps, and VFS call counts for browser playgrounds.
- `collect_hardware_requests` extracts synthesis, sequencing, and destructive
  requests from effectful MIR so a hardware bridge can submit them without
  changing NucleScript source syntax.

Every declaration/operation in the AST (`PoolDecl`, `LetDecl`, `StoreOp`, and
so on) carries a `Span { line, column, end_line, end_column }`, threaded from
the lexer's token positions through the parser and into every diagnostic
`typeck::check_program` produces. Every diagnostic also carries a stable
`code` (see [docs/errors.md](errors.md)) and, for the most common
undeclared-name mistakes, a "did you mean X?" suggestion via edit-distance
matching against names actually in scope. `nucle check` renders this as a
rustc-style snippet — `file:line:column`, the offending source line, and a
`^^^` underline — and the playground API exposes the same structured data
as JSON, so both a CLI and a future editor integration read from one
diagnostic shape, not two that can drift.

> See [docs/grammar.md](grammar.md) for the full formal syntax reference,
> [docs/effects.md](effects.md) for the effect model — including how effects
> propagate through function calls, not just literal operations — and
> [docs/stdlib.md](stdlib.md) for `consensus_vote`/`protect`, NucleScript's
> two built-in functions.

`consensus_vote` and `protect` are ordinary `FunctionTable` entries
(`stdlib::builtin_functions`), not separate AST nodes with their own
hardcoded type-checking/effect logic. The parser still accepts their
existing keyword-sugar surface syntax (`consensus_vote(source, coverage:
N)`, `protect data for guarantee`) but desugars both directly to
`Expr::FunctionCall` at parse time, so arity checking, effect propagation,
and "did you mean X?" suggestions all flow through the exact same path a
call to a user-defined `fn` does — `effects::function_table` seeds a
program's function table from the built-ins before overlaying its own `fn`
declarations, and `typeck::TypeChecker::lookup_function` does the same for
type-checking. `consensus_vote`'s actual return type still needs one
narrow, explicit intrinsic-recognition branch
(`TypeChecker::infer_consensus_vote`), because its result genuinely
depends on its *argument values* (the source binding's inferred error
rate, the requested coverage) — a real compiler-level computation no
fixed declared signature could express, not something the shared
function-call machinery skipped by accident.
`simulate`/`synthesize`/`sequence` stay as dedicated grammar forms rather
than joining the stdlib, since their `confirm hardware` effect-
confirmation semantics are load-bearing enough to want a real grammar
production of their own.

A TextMate grammar for `.nsl` files lives at
`editors/vscode/nuclescript/syntaxes/nuclescript.tmLanguage.json`, derived
from `docs/grammar.md`/the actual keyword sets in `lexer.rs`/`parser.rs`
(not invented independently), and is snapshot-tested against every file in
`docs/examples/` (`vscode-tmgrammar-snap`) so grammar/compiler drift shows
up as a diff. This is purely presentational; live diagnostics, hover, and
navigation come from the language server below.

### Generics (`fn name<T>(...)`)

Resolves entirely at type-check time, with no runtime representation and
no per-instantiation re-checking of a function's body — neither classic
monomorphization (generating a fresh copy of code per concrete type) nor
dynamic dispatch, because this codebase's actual constraints make both
unnecessary: `Profile` (`Illumina`/`Nanopore`/`Twist`) is a closed, flat
enum with no subtyping anywhere in the language, so variance never comes
up, and effect classification (`effects.rs`) never inspects a pool's
profile at all — every operation already behaves identically across all
three. Concretely: `PoolState` (the state slot inside `Pool<...>`) gains
a fourth variant, `Var(String)`, an unbound type parameter that a
generic function's body is type-checked against exactly once, opaquely —
the *existing* fallback typeck already uses when it needs a concrete
`Profile` but only has a non-`Profile` state (`Amplified`/`Recovered`)
absorbs `Var` for free, with zero new code, since it was already a
wildcard. At each call site, `infer_expr`'s `FunctionCall` arm unifies
every `Pool<T>`-typed argument against the parameter's `Var`, building a
substitution used only to (a) catch the same type parameter being bound
to two different profiles in one call (`E-TYPE-PARAM-CONFLICT`) and (b)
resolve a return type that mentions the type parameter to the concrete
type for that specific call. `codegen.rs`'s interpreter (the one added
for `Result<T,E>`/`?` above) needs zero changes — it resolves a function
call purely by name and executes whatever concrete `Value`s were passed,
with no notion of "generic" at any point. See the "Generics" section of
[docs/grammar.md](grammar.md) for the full semantics, including the one
honest limitation (a handful of profile-specific typeck warnings can't
fire while checking a generic body against an abstract type parameter).

**Explicit type-argument syntax, `name::<Illumina>(...)`**:
seeds the call's substitution map from the explicit arguments (zipped
against the function's own declared `<T, U>` list) *before* the
pre-existing per-argument unification loop runs, reusing the same
`E-TYPE-PARAM-CONFLICT` an ordinary disagreeing argument already reports,
plus a new `E-TYPE-PARAM-ARITY` for the wrong explicit-argument count.
This closes the one real gap the original design left: a type parameter
that appears *only* inside a `Fn(...)`-typed parameter's own signature,
never as a directly `Pool<P>`-shaped argument itself, has nothing for the
named-function argument loop to unify it from (that loop only inspects
`TypeExpr::Pool`-shaped arguments; a `Fn(...)`-typed argument is validated
by a separate helper, `check_fn_typed_arg`, that doesn't feed the outer
substitution map at all) — an explicit type argument is the only way to
call it.

### Enums (`enum` declarations) and general pattern matching

Two related gaps stood open from earlier in the language's development,
named explicitly at the time: a general pattern-matching/exhaustiveness
engine (needing user-defined enums, which didn't exist at all) and
accurate effect analysis through an arbitrary closure call (deferred
separately, still open). This closes the first.

`ast::EnumDecl { name, variants: Vec<EnumVariant>, span, doc }` is a new
top-level declaration, each variant at most one payload
(`Variant`/`Variant(Type)`, mirroring `Ok(T)`/`Err(E)`'s own shape — no
tuple/struct variants). `EnumName::Variant`/`EnumName::Variant(payload)`
(`Expr::EnumConstruct`) reuses the `::` token turbofish already uses,
disambiguated by one token of lookahead (turbofish's `::` is
always followed by `<`; a variant reference's is always followed by an
identifier). `TypeExpr::Enum(String)` is a new type-expression case;
concretely, `parse_type_expr`'s previously-hard-error fallback branch now
treats *any* unrecognized identifier as presumably naming a user enum,
deferring the real validation to typeck (`check_enum`/`enums` lookup)
rather than teaching the parser NucleScript's set of declared names.

**Result unification happens at exactly one layer: matching, not
construction or runtime storage.** The user-facing ask was for `Result<T,
E>`'s existing `match`/`Ok`/`Err` machinery to run through the same
general engine as a real user `enum` — not for `Result` to become
literally an instance of the `enum` mechanism. `Expr::Ok`/`Expr::Err`
stay their own dedicated, unprefixed `Expr` variants, untouched:
`Err`'s payload has a bespoke string-literal-only restriction that
doesn't generalize to arbitrary enum variants, and folding construction
into one path would mean either a magic `enum_name == "Result"` special
case or silently dropping that restriction. `Value::Result` similarly
stays a distinct runtime representation; a new `Value::EnumInstance {
enum_name, variant, payload }` is a *sibling*, not a replacement — the
dozens of existing `codegen.rs` sites pattern-matching `Value::Result(Ok
(_)/Err(_))` directly in the hottest, most safety-critical control-flow
path (`?`'s short-circuit, `call_user_function`'s auto-wrap) stay exactly
as they were, compiler-checked and exhaustive, rather than becoming
stringly-typed `variant == "Ok"` comparisons for zero behavioral gain.

The unification lives entirely in `typeck::check_match`/`codegen.rs`'s
`eval_expr` Match arm. `check_match` first resolves what's being matched
via a new `ScrutineeKind` (`Result { ok_ty, err_ty }`, whose two variants
are always exactly `Ok`/`Err`; or `UserEnum { decl }`, a real `EnumDecl`
looked up by name) — through a new `infer_scrutinee_kind`, which tries
the existing, completely unchanged `infer_result_expr` first, and only
falls back to resolving an enum name (from an `EnumConstruct`, an
enum-typed variable, a closure/function call's declared return type, or
a nested `match`) if that fails. Either way, the same four-pass check
runs: shape-check every arm against the scrutinee's own
`declared_variants: &[VariantSignature]` (unknown variant name, wildcard
not last, duplicate arm), exhaustiveness (one arm per declared variant,
or a trailing wildcard), per-arm type-check via
`check_match_arm_general`, then unify every arm's resolved type. `eval_
expr`'s runtime side is unified the same way: a new shared `run_match_
arm` helper extracts `(variant_name, payload)` uniformly from either a
`Value::Result` or a `Value::EnumInstance`, finds the arm whose `variant`
matches (falling back to a wildcard), and evaluates its body in a cloned
environment — bit-for-bit the same outcome for any existing 2-arm Result
match, the core of the zero-regression proof (`result_backward_compat.
rs` re-run completely unmodified, still green).

**Arm order is now free, checked by variant name rather than
position** — a deliberate, confirmed behavior change from the fixed
Ok-then-Err order originally required (`match r { Err(e) => ...,
Ok(v) => ... }` is now exactly as valid as the reverse). `Expr::Match`'s
shape changed accordingly, from four hardcoded fields
(`ok_pattern`/`ok_body`/`err_pattern`/`err_body`) to `{ scrutinee, arms:
Vec<MatchArm> }`, `MatchArm { variant: Option<String>, binding:
Option<String>, body, span }` (`variant: None` is the wildcard) — a real,
compiler-enforced breaking change that turned every existing `Expr::
Match` construction/destructuring site across the workspace into a
compile error until updated, the same forcing function every prior
feature relied on.

**The old cross-arm-context trick, correctly generalized.**
Re-reading the old "hand the `Err` arm the `Ok` arm's own resolved value
type" logic closely revealed it never actually needed a sibling arm's
*resolved* value at all — only the scrutinee's own *declared* Ok-side
type, already fully known statically before any arm is checked. So the
N-arm generalization turned out simpler than an accumulator-based
"resolved so far" threading: `check_match_arm_general` receives the full
`declared_variants` list as static context up front, and `Ok(pattern) =>
Ok(pattern)`/`Err(pattern) => Err(pattern)` resolve by looking up the
other of `Ok`/`Err` inside it directly. The general `EnumName::Variant
(pattern) => EnumName::Variant(pattern)` re-wrap needs no "other side" at
all — just its own variant's declared payload type.

**A real, user-flagged limitation fixed properly, not worked around.**
`check_match_arm_general`'s dispatch for "a nested `match` as an arm's
body" originally only accepted an inner match whose *own* resolved type
was Result-shaped — an inner match whose every arm already used `?`
(producing a bare, unwrapped value) was rejected with
`E-MATCH-ARM-UNTYPABLE`, even though the outer match's own dispatch
handles a bare bound name fine. Generalized to delegate directly to
`self.check_match(body, span)` and accept whatever it resolves to —
Result-shaped, enum-shaped, Pool-shaped, or already-bare — a strictly
more general and correct fix, verified not to break any existing test.

**Two real gaps surfaced by hands-on testing of the shipped example, one
fixed, one accepted as a genuine, narrower limitation.** (1) Enum-typed
function/closure parameters were invisible to scope — no `LetDecl`
registers a parameter, so the existing `pool_bindings`/`result_bindings`
convention had no equivalent for an enum-typed parameter until a new
`enum_bindings: HashMap<String, String>` field was added, populated in
`check_let`'s new `Expr::EnumConstruct` branch and both param-seeding
loops (`check_function`/`check_closure_expr`). (2) `Err(...)`'s payload
must always be a literal string, even inside a user-enum match arm — a
bound `Str` variable (a user enum's own payload, or an outer arm's
binding) can never be re-wrapped into a fresh `Err(...)`, since the only
mechanism for resolving `Err`'s missing `Ok` side is the sibling-arm
trick within the *same* `Result` match, which has no equivalent for a
user-enum match. Not fixed (would need new syntax/mechanism, out of
scope here) — the shipped example is instead designed so every arm
produces its `Result` through a real, self-contained fallible expression.

`effects.rs`/`middle.rs`/`sim_backend.rs` all generalized their `Expr::
Match` handling from two hardcoded arms to a fold over `arms` (effect =
join across scrutinee + every arm; confirmation = AND across all of them
— the same conservative "every declaration in this join counts" rule
`if`/`?` already follow); `sim_backend.rs`'s `narrate_result_expr` in
particular has a real, not-compiler-enforced wildcard `_` arm (same as
before this step), so its generalization was written and verified
explicitly rather than relying on the compiler to force it.
`formatter.rs` needed *zero* changes — its existing token-based rules for
braces/commas/keywords were confirmed, by direct testing, to already
generalize correctly to `enum` declarations, `EnumName::Variant`
construction, and N-arm `match` bodies. See the "Enums"/"Pattern
Matching" sections of [docs/grammar.md](grammar.md) for the full surface
semantics, and
[`docs/examples/recovery_plan.nsl`](examples/recovery_plan.nsl) for a
complete, runnable example exercising a user enum, a nested match, and
exhaustiveness via a trailing wildcard.

### Closures (`Fn(...)` / `fn(params) -> T { body }`)

The capture-semantics question this was originally deferred over ("by
value or by reference? what does capturing a mutated-in-place pool even
mean?") turned out to have a structural, not chosen, answer: every `let`
binding in NucleScript is single-assignment, so there is no "later
mutation" a by-value/by-reference distinction could ever observe --
capture-by-snapshot is simply correct here, not a design compromise.
Separately, pools are never mutated *as bindings* -- `store`/`delete`
reference a `pool` *declaration* by name (always globally visible),
never a captured probabilistic binding -- and have no runtime `Value`
representation at all, so "capturing a pool" was never a coherent
runtime operation to begin with. Capture in `codegen.rs`'s `eval_expr`
is therefore just one `env.clone()` at the point a closure literal is
evaluated (`Value::Closure`'s `captured_env` field) -- nothing to
filter, since `env` already only ever holds `Value`-representable
things (a captured `Pool`/`Strand`/`Sequence` binding, if present at
all, is an inert `Value::Unit` placeholder, exactly as harmless to
capture as it already is to pass around anywhere else).

Calling a closure reuses the *existing* `name(args)` surface syntax
(`Expr::FunctionCall` is completely unchanged) -- typeck's `self.closures`
map and the runtime's `env`-before-`funcs` lookup both resolve a
closure-bound name *before* falling back to the global function table,
so a call site looks identical whether `name` is a top-level `fn` or a
local closure.

**Self-recursion — the one genuine runtime bug found in this
whole language's development, not a design gap.** `codegen.rs`'s
`call_closure` always started a call's own `env` fresh from
`captured_env`, the snapshot `Expr::Closure` takes at evaluation time --
*before* its enclosing `let` finishes binding, so `captured_env` never
contains the closure's own name. A self-recursive call therefore actually
resolved to "internal error: undeclared function," silently wrapped into
a `Value::Result(Err(...))` with no VFS step to show for it -- close
enough to a real, caught failure that a first hand-verification pass
misread it as the self-recursion actually working (one real failure plus
one silently-swallowed internal error looks identical to two real
failures unless you check the count). The real bug surfaced two ways: an
automated test asserting the *count* of real steps failed outright, and
once fixed, the fix immediately produced a stack overflow on the existing
example -- a closure retrying the exact same failing operation with no
changing state now genuinely recursed, forever, since nothing ever made a
later attempt behave differently. Fixed by having `call_closure`
re-insert the closure under the name it was just called through, into
its own fresh `env`, before running its own body -- on *every* call, not
just the first, which is what makes arbitrarily deep recursion work
rather than one level. The shipped example was redesigned alongside the
fix to retry into a genuinely different fallback target on the second
attempt, so the recursion is real *and* terminates.

**Generic closures, `fn<T>(...)`**: `Expr::Closure` gained its
own `type_params: Vec<String>`, mirroring `FunctionDecl`. Because a fresh
type-parameter name is only recognized inside a closure literal's own
declared `<...>` list at parse time (not inferred from surrounding
context), a generic closure is only expressible nested inside an
already-generic enclosing function/closure sharing the same
type-parameter name -- the parser's `type_params_in_scope` now *merges*
with the outer scope in both `parse_function_decl` and the new
`parse_closure_expr`, instead of clobbering it, since closures can nest
this way. `check_closure_call_args` was rewritten to perform the same
`PoolState::Var` unification a named function's call-site checking
already does, rather than a flat equality comparison -- surfacing and
fixing a real, latent false-positive `E-ARG-TYPE-MISMATCH` along the way
(a `Pool<Var>`-typed expected parameter was being compared for flat
equality against any concrete argument, which is always unequal).

**`nucle plan`/`nucle explain` narration through a `let`-bound closure's
own call**: `sim_backend.rs`'s `narrate_result_expr` now threads
a `closures: &mut HashMap<String, Vec<Declaration>>` map (name → body),
built incrementally by the same two declaration-walking loops that
already existed, checked *before* the named-function lookup -- the exact
priority `effects.rs`'s own earlier fix already established. A closure
received as a `Fn(...)`-typed *parameter*, or an inline closure literal
passed directly as a call argument, remains unnarratable, since neither's
real body is known at that call site, only at runtime -- this closes only
the `let`-bound case.

This is also why no cycle-guard machinery was needed for *mutual*
recursion between closures: capture only ever sees bindings from *before*
the literal's own position, so two independently-`let`-bound closures can
never see each other -- `codegen.rs`'s existing `calling`/`func.name`
guard (built for named function recursion) stays completely untouched.

`effects.rs` needed a real signature change, not just an additive match
arm -- the one place this feature couldn't stay purely additive. It's a
free-function pass over `&FunctionTable` with no visibility into
typeck's per-scope state, so `expr_effect`/`expr_has_required_
confirmation`/`function_call_effect` all gained a second `closures:
&FunctionTable` parameter: a synthetic `FunctionDecl` per `let`-bound
closure (real params/return_type/body, so its actual effect can be
computed by recursing into it, exactly like a named function already
can) that `typeck::TypeChecker::check_let` populates and passes through
at its own confirmation-gating call site -- the one place that actually
gates compilation. A `Fn(...)`-typed *parameter*'s call was left as a
real, deliberately open gap at the time (its real body isn't knowable
until runtime, so it was optimistically treated as `Pure`/confirmed, no
worse than the pre-existing "can't resolve, assume Pure" fallback for
any unresolvable name) -- closed by effect-annotated function types,
below. `sim_backend.rs`'s narrator has the identical, symmetric gap for
the same reason (it resolves callees by static name and has no runtime
environment to consult) and remains genuinely open -- see the "Closures"
section of [docs/grammar.md](grammar.md) for the full semantics.

### Effect-annotated function types

The other real gap `Fn(...)`-typed parameters left open: their call
inside a function body couldn't have its effect analyzed at all, since
the concrete closure a caller passes isn't knowable until runtime.
`TypeExpr::Fn` gained a third field, `Option<FnEffectAnnotation>`
(`Hardware`/`PhysicalKey`, mirroring the *only* two confirmation
keywords the language already has -- `confirm hardware` for
`Synthesis`/`Sequencing`, `confirm physical_key` for `Destructive`) --
`None` for every `Fn(...)` type written before this feature, preserving
today's exact behavior unconditionally. Surface syntax reuses the exact
existing `confirm`/`hardware`/`physical_key` tokens
(`Fn(...) -> T confirm hardware`), so no new lexer tokens were needed.

**The soundness argument, and the hole an architecture-review pass found
in the first draft.** The only place a *concrete* closure value is ever
created is an `Expr::Closure` literal -- a parameter, a `let`-bound name,
or a captured name inside a nested closure is always just an alias to a
closure created somewhere else. So the actual invariant needed is: a
call to any `Fn(...)`-typed expression is sound to trust as
`(declared_effect, confirmed=true)` as long as every concrete closure
ever bound into that annotated slot was checked against the ceiling
*once*, wherever it was concretely bound -- not re-derived at the
parameter's own call site. The first draft's design checked this only
for the explicit-argument-passing case; a dedicated review pass found
that a *capture* (an inner closure calling an annotated *outer*
parameter, never passed as an explicit call argument at all) was never
covered, which would have made the ceiling untrustworthy for that case.
The fix, adopted in the final design: run the *same* effect computation
uniformly everywhere a concrete closure is bound into an annotated slot
(`typeck::TypeChecker::check_fn_effect_compatibility`, called from both
`check_fn_typed_arg` and `check_let`'s existing closure-registration
flow), always using the *current* scope's real `fn_param_effects` --
this naturally resolves the capture case for free (the enclosing
function's own annotated parameters are already in scope when the inner
closure's effect gets computed) and the *forwarding* case too (an
annotated parameter passed straight through as another function's
compatibly-annotated parameter resolves via its own trusted ceiling,
no real body needed) with no special-casing for either.

Concretely: `effects.rs` gained `fn_param_effects: &HashMap<String,
Effect>` as a new parameter threaded through `expr_effect`/`expr_has_
required_confirmation`/`function_call_effect`/`decl_effect_info` (mapping
a `Fn(...)`-typed parameter's name to the `Effect` its annotation stands
for) -- `function_call_effect`'s existing "unresolvable name" fallback
now checks this table before defaulting to `(Pure, true)`. A new
`effects::scoped_fn_param_effects` builds the table a callable's *own*
body should be resolved against: its own annotated parameters (always
authoritative -- a name collision could only type-check if the callable
itself also declares that name) layered over whatever the enclosing
scope already had (meaningful only for a closure, which really does
capture lexically). `function_call_effect`'s own body-joining loop was
extracted into a new `effects::body_effect`, reused by `typeck::
TypeChecker::check_fn_effect_compatibility` to compute a closure
*literal*'s effect directly (it has no name of its own to resolve a call
against). `typeck::TypeChecker` gained a parallel `fn_param_effects:
HashMap<String, Effect>` field, populated in `check_function`'s and
`check_closure_expr`'s existing parameter-seeding loops and cloned into
child checkers the same way `closures`/`closure_decls` already are.

**Deliberately not attempted**: distinguishing `Synthesis` from
`Sequencing` at the annotation level -- the language's own `confirm
hardware` doesn't distinguish them either, and `nucle_hardware`'s
`collect_hardware_requests`/`middle::lower_program` are already strictly
top-level-scoped, never walking into any function or closure body at all
(verified directly: `Declaration::Function` is an explicit no-op there),
so this analysis's output was never going to reach that pipeline either
way, annotated or not. `sim_backend.rs`'s narrator remains genuinely
unable to see through an annotated parameter's call -- a declared
ceiling is not a real body to synthesize a concrete VFS step from; only
whole-program flow analysis (the option not taken) could close that
specific, narrower gap.

**A second, unrelated real bug found while building the verification
example.** `codegen.rs`'s `is_result_producing` (gating whether a
top-level `let` is routed through the real interpreter at all) only ever
returned `true` for a call to a `Result<_, _>`-returning function --
correct before statement-form execution existed, wrong afterward: a
top-level `let result: Void = do_thing()` where `do_thing`'s body has a
real statement-form `store`/`delete` silently never ran it, with no
error and no diagnostic, discovered because the effect-annotated-`Fn
(...)`-types example is specifically a top-level call chain into a
genuinely `Destructive` closure. Fixed by routing any function call
through `eval_expr` except one returning a compile-time-only type
(`Pool<...>`/`Strand`/`Sequence`/`File`/`Recovery`, none of which
`value::Value` has a runtime representation for at all, so `eval_expr`
has nothing to produce for them anyway).

See the "Effect-Annotated Function Types" section of
[docs/grammar.md](grammar.md) for the full surface semantics, and
[`docs/examples/effect_annotated_closure.nsl`](examples/effect_annotated_closure.nsl)
for a complete, runnable example -- verified directly with `nucle doc`,
which now shows both functions in the example correctly reporting
`Destructive (confirmed)` rather than the previous, silently wrong
`Pure`.

### Language Server (`nucle_lsp`)

`nucle_lsp` is a thin LSP protocol adapter, not a second compiler: every
answer it gives comes from `nucle_lang::analyze` (`typeck::
check_program_with_symbols` under the hood), the exact same pass `nucle
check` and the playground already run. `typeck::TypeChecker`'s own
scope-tracking (`pools`, `functions`, `strands`, `sequences`, `bindings`)
is exposed as a `SymbolTable` — top-level declaration name/span pairs —
specifically so the language server doesn't re-derive a second, possibly
divergent notion of "what's declared where." `nucle_lsp/src/backend.rs`
implements:

- `textDocument/didOpen`/`didChange` → `publishDiagnostics`, using the
  same `Span`/error `code` every CLI diagnostic already carries.
- `textDocument/hover` — finds the identifier at the cursor by slicing the
  open document's text (not a second AST — the server keeps document text
  in memory and re-parses on every request, since NucleScript programs are
  small and re-parsing is cheap), then looks it up in the `SymbolTable`.
- `textDocument/definition` — same lookup, returning the declaration's
  span as the jump target.
- `textDocument/documentSymbol` — the whole `SymbolTable` as an outline.

Deliberately out of scope for this first pass: completion, rename, and
semantic tokens (already covered by the TextMate grammar). Verified with
an in-memory duplex-pipe integration test
(`nucle_lsp/tests/diagnostics.rs`) that speaks the real Content-Length-
framed JSON-RPC protocol — not just unit tests of the internal Rust
functions — and cross-checks the published diagnostics against
`nucle_lang::check_source`'s own output for the identical source, so the
server can't silently drift from what the CLI reports.

The VS Code extension (`editors/vscode/nuclescript/src/extension.ts`)
spawns the `nucle-lsp` binary over stdio via `vscode-languageclient` — the
one place in this stack that's TypeScript rather than Rust, kept to
"spawn and connect" with no logic of its own. `src/serverDownload.ts`
resolves which binary to spawn: an explicit `nuclescript.serverPath`
setting, then `nucle-lsp` on `PATH` (local development), then a prebuilt
binary for the current OS/architecture downloaded once from
`.github/workflows/release-vscode-extension.yml`'s GitHub Release output
and cached in the extension's global storage — the path a marketplace
install (no local Rust toolchain) needs.

`src/formatProvider.ts` (Format Document) and `src/runProvider.ts`
(`NucleScript: Run File`) both shell out to a separate `nucle-cli`
binary rather than `nucle-lsp` — different executable, same three-tier
resolution, via `src/cliDownload.ts`. It shares the actual HTTPS
fetch-and-cache logic with `serverDownload.ts` (factored into
`src/download.ts`) but downloads from the main NucleOS repo's own `v*`
release tags instead of `nucle-lsp-v*`, since `nucle-cli` ships as part
of NucleOS itself and is versioned independently of the extension —
`cliDownload.ts` pins a specific tag (`CLI_RELEASE_TAG`) rather than
deriving one from the extension's version. `runProvider.ts` runs
`nucle-cli run <file>` as a VS Code `Task` (`ShellExecution`) rather than
a raw child process, so its output shows up in an integrated terminal
the way a real "Run" command would, and argument quoting is handled by
the Tasks API instead of by hand.

The extension is published on the Marketplace under the `nuclescript`
publisher; see the extension's own
[CONTRIBUTING.md](../editors/vscode/nuclescript/CONTRIBUTING.md) for how
to ship an update (a manual `.vsix` upload, or the CI workflow if
`VSCE_PAT` is configured as a repository secret).

### Formatter (`nucle fmt`)

One canonical, zero-configuration style, `gofmt`-style: there is exactly
one way `nucle fmt` writes a given program, so a diff never contains
unrelated whitespace churn. `nucle_lang::formatter::format_source`
deliberately does **not** print from the AST — the AST drops comments and
normalizes literal spellings by design (see `ast.rs`), so printing from it
would silently delete every `//` comment. Instead it re-renders the real
token stream (`lexer::Lexer`, which already carries each token's
line/column from the lexer's own span-tracking work) plus a small dedicated scan for
comments (the one thing tokenizing discards), consulting the parsed
`Program` for exactly one thing: each top-level declaration's start line,
used to force exactly one blank line between top-level declarations
without splitting a leading doc comment away from the declaration it
documents.

Concretely, formatting keeps every line-break the input already has (no
line-wrapping heuristics for this first cut, matching how `gofmt` leaves
most multi-line constructs alone), recomputes indentation from bracket-
nesting depth, recomputes inter-token spacing from a small rule table, and
collapses blank-line runs to at most one everywhere. Because every rule is
a pure function of (tokens, comments, which line-breaks exist), running
the formatter on its own output is a no-op by construction, verified by
`nucle_lang/tests/formatter.rs` sweeping every file under `docs/examples/`
for both idempotence and "formatting never changes the parsed program."
`nucle_cli`'s `Fmt` command exposes `--check` (CI, exits non-zero if not
already formatted) and `--write` (rewrite in place); `nucle fmt -` reads
the buffer to format from stdin instead of a file, which is what the VS
Code extension's `Format Document`/format-on-save provider
(`src/formatProvider.ts`) shells out to, so there's exactly one formatting
implementation rather than a second one duplicated in TypeScript or the
language server.

### Test runner (`nucle test`)

`test "description" { ... }` (`ast::TestDecl`) is a named block of
declarations; `assert <condition>` (`ast::AssertOp`, valid anywhere a
declaration is, not just inside a test) is evaluated by the *exact same*
`typeck::TypeChecker::eval_condition` an `if` condition uses — there's no
separate assertion DSL, reusing `if`'s own existing
comparison operators. That's also why an assertion is checked at
type-check time rather than deferred to some later "runtime" phase:
NucleScript's probabilistic properties (a pool binding's inferred error
rate) are deterministic formulas computed at compile time already, not
something measured empirically, so there's nothing for an assertion to
wait for. A false assertion is reported as an ordinary `E-ASSERTION-FAILED`
diagnostic at its own span — `nucle check` surfaces one anywhere in a
program as a real bug on its own, with no dependency on `test_runner.rs`
at all.

`nucle_lang::test_runner::run_tests` is what turns that into pass/fail per
test: it runs `typeck::check_and_desugar` once over the whole program (a
*real* compile error anywhere — anything other than a failed assertion —
aborts the entire run before any test executes, the same way a type error
in a Rust test file stops `cargo test` from running anything), then for
each `TestDecl` in the desugared output, groups every `E-ASSERTION-FAILED`
diagnostic whose span falls within that test's line range into its
result. Independently, it builds a small "virtual program" per test (the
file's own non-test top-level declarations — pools, lets, functions — plus
that one test's body) and runs it through the exact same
`codegen::compile_program`/`execute_program` path `nucle run` uses,
against a fresh `NucleOS` instance per test for isolation — so a test can
also catch a genuine VFS failure (a `retrieve`/`delete` erroring out), not
just a failed assertion. `nucle_cli`'s `Test` command reports `cargo
test`-style pass/fail output (or `--json`).

Not implemented: wiring `nucle test --json` into VS Code's native Test
Explorer API, an optional stretch goal the plan itself flagged as likely
to slip — the CLI command was the stated acceptance bar.

### Doc comments and `nucle doc`

`///` is a real, distinct token (`lexer::TokenKind::DocComment`), not a
plain `//` comment with an extra slash discarded during tokenizing.
`parser::Parser::consume_doc_comment` accumulates every consecutive `///`
line immediately before a declaration into one `\n`-joined string, and
attaches it to a `doc: Option<String>` field -- present only on
`PoolDecl`/`StrandDecl`/`SequenceDecl`/`FunctionDecl`/`PipelineDecl`, the
five kinds of declaration with a real name and signature worth looking up
in generated docs. A `///` before anything else (a `let`, an operation,
`if`/`for`/`test`) is rejected with a parse error naming the offending
keyword (`reject_doc_comment`) rather than silently discarded — there's
no field to attach it to, so it's always a mistake worth surfacing.

`docgen::generate_docs` walks a `Program`'s top-level declarations of
those five kinds and renders one Markdown document, grouped by kind: each
entry gets its doc text (if any), its real NucleScript signature (not a
paraphrase), and its effect (via the same `effects::decl_effect_info` the
playground/`nucle explain` use, so a function that ends up `Synthesis`
because it calls `store` shows that, not just its declared return type).
An *un*documented declaration still gets an entry — the output is meant
to be a complete reference, not just whatever a program's author
remembered to comment. `nucle doc <source> [--output <file>]` exposes this;
default is stdout.

### Project scaffolding (`nucle new`)

`nucle new <name>` creates a directory with `main.nsl` (a self-contained
probabilistic-pool program that needs no external sample file, so `nucle
check`/`nucle run` succeed against it completely unmodified), a
`README.md` with the basic command reference, and an empty `nucle.lock`
(`lockfile::LockFile::default()`, serialized the same way `nucle package
lock` writes a populated one) — ready for `nucle package install` to
start populating once the project actually depends on something.

## NucleScript Playground

The interactive playground has three tabs, each backed by the real engine rather than reimplemented or mocked logic, and ships two ways from the same source:

- **`nucle_wasm`** compiles the compiler/codec/ECC/noise engine to `wasm32-unknown-unknown` via `wasm-bindgen` and runs it entirely client-side — no server at all. Live at [nuclescript.github.io/playground](https://nuclescript.github.io/playground/), rebuilt and redeployed to GitHub Pages by `Nuclescript/playground`'s `.github/workflows/pages.yml` on every push.
- **`nucle_playground`** is a thin `tiny_http` HTTP server exposing the same three operations over `POST /analyze`, `/benchmark`, `/pipeline-demo`, for anyone who wants a native binary instead of a browser tab.

Both frontends call into the exact same logic — `nucle_wasm` and `nucle_playground` both depend on `nucle_demo_core` (a pure, I/O-free crate holding the benchmark and pipeline-visualizer implementations) plus `nucle_lang::playground::analyze_source` for the Write & Run tab, so there is one implementation to keep correct, not two that can drift.

- **Write & Run**: returns `PlaygroundReport` as JSON containing compiler diagnostics, simulator steps, and optimizer notes — the same `analyze_source` API `nucle check --json` uses.
- **Benchmark Explorer**: accepts `{ codec, profile, redundancy, data }` and returns density, GC distribution, homopolymer violations, and an estimated cost — all from `nucle_codec::benchmark` — plus a `recovery_probability` computed by actually running Reed-Solomon parity + `NoiseEngine` simulation + decode across 20 trials. The frontend debounces control changes (codec/profile/redundancy sliders) and re-runs live.
- **Pipeline Visualizer**: encodes real input via `TernaryCodec`, adds RS parity, runs it through `NoiseEngine`, and returns per-strand before/after sequences plus drop/corruption flags so the frontend can animate encode → noise → recovery. Recovery is attempted for real (RS-decode using surviving strands as input, then codec-decode) — a failure shown in the UI is a genuine failure of the current pipeline at that redundancy/profile, not a scripted outcome.
- **Frontend**: A single glassmorphic dark-themed page with tab navigation between the three modes; plain HTML/JS, no build tooling. The WASM build's copy (`nucle_wasm/www/index.html`) calls straight into WASM functions instead of `fetch()`-ing a server, but is otherwise the same UI.
- **`wasm32` portability note**: `std::time::Instant`/`SystemTime` panic unconditionally on `wasm32-unknown-unknown` ("time not implemented on this platform"). `nucle_codec::benchmark` uses the [`web-time`](https://docs.rs/web-time) crate instead — a drop-in replacement that re-exports `std::time` unchanged on every other target and backs it with `Performance.now()` in the browser — so the same timing code works natively and in WASM.
- **Published standalone**: A self-contained snapshot of this workspace (verified to build independently from a fresh clone) is published at [github.com/Nuclescript/playground](https://github.com/Nuclescript/playground). For zero setup, prebuilt Linux/Windows/macOS binaries of `nucle_playground` (frontend embedded via `include_str!`, no external files needed) are published on that repo's [Releases page](https://github.com/Nuclescript/playground/releases) via a tag-triggered GitHub Actions workflow — free to run and host, since public-repo Actions minutes, Pages hosting, and Release storage all have no cost.

## Biological Constraints

All encoding must satisfy hard constraints imposed by DNA chemistry:

| Constraint | Value | Reason |
|-----------|-------|--------|
| GC Content | 40–60% | Synthesis fidelity, PCR amplification balance |
| Homopolymer max | 3 bases | Sequencing accuracy (especially Nanopore) |
| Secondary structure | No palindromes > 6 nt | Prevents hairpin formation during PCR |
| Strand length | 150–200 nt typical | Synthesis yield vs. data density tradeoff |

## Error Channel Model

DNA storage has a unique error profile unlike any digital channel:

| Error Type | Synthesis (Twist) | Illumina Seq | Nanopore Seq |
|-----------|-------------------|-------------|-------------|
| Substitution | ~0.01% | ~0.1% | ~3-5% |
| Insertion | ~0.005% | ~0.01% | ~2-5% |
| Deletion | ~0.02% | ~0.01% | ~2-5% |
| Strand dropout | 0.5-5% | — | — |

## Codec Strategies

### Ternary Rotating Cipher (Goldman et al., 2013)
- Converts binary → base-3 (ternary)
- Rotating mapping rule eliminates all homopolymers by construction
- Overlapping segments provide natural redundancy
- Effective density: ~1.58 bits/nucleotide

### DNA Fountain (Erlich & Zielinski, 2017)
- Luby Transform (LT) codes applied to DNA storage
- Rateless: can generate unlimited encoded strands
- Built-in screening rejects constraint-violating strands
- Near-optimal density: ~1.57 bits/nucleotide
- Natural erasure resilience — any sufficient subset of strands reconstructs data

### Yin-Yang Codec (Ping et al., 2022)
- Two complementary mapping rules achieve GC balance by construction
- Yang rule: `0 → {A,T}`, `1 → {C,G}` — structural 50% GC guarantee
- Yin rule: context-dependent (previous base) mapping reduces homopolymers
- Highest density: 2.0 bits/nucleotide theoretical, ~1.85 effective
- Best suited for real-world data with natural bit entropy

## Error Correction Architecture

Two-layer coding scheme (industry standard):

1. **Inner code** (per-strand): Handles substitutions, insertions, deletions within individual strands
2. **Outer code** (cross-strand): Handles strand dropouts and residual errors

```
Data → [Outer RS/Fountain] → [Segmentation] → [Inner encoding] → [Constraint screening] → DNA
DNA → [Basecalling] → [Clustering] → [Consensus] → [Inner decode] → [Outer decode] → Data
```

### Current status

The outer code (RS strand-level erasure recovery) and the consensus stage are both implemented and wired together in `dna_read`. The ternary decoder is still strict — it rejects a noise-corrupted strand rather than attempting soft decoding, and RS alone only recovers a strand that's entirely missing, never one that survived corrupted — but neither of those strands ever reaches the decoder directly anymore: `dna_write` records which stored strands are coverage copies of the same logical strand (via `PoolEntry::source_index`), and `dna_read` groups them and runs `nucle_ecc::consensus::build_consensus` per group before RS decode, correcting substitution errors regardless of which individual copy has them. A logical strand with zero surviving copies still becomes an erasure for RS, same as before. This requires actual sequencing coverage (`coverage_depth > 1` in `SimulationConfig`) to have multiple independent reads to vote across — a single read has nothing to vote against.

**Illumina works. Nanopore still doesn't, and the diagnosis moved twice as we kept digging — worth documenting precisely rather than leaving an earlier explanation stale.** First diagnosis: `build_consensus` voted by raw position, which breaks under indels. Fixed by aligning each read to the reference before voting (Needleman-Wunsch). Second, bigger diagnosis: `nucle_index::primer::PrimerPair::{matches_forward, untag_strand}` matched primers by exact position, so a single indel landing inside a primer — routine at Nanopore's ~4%/base indel rate — made CRISPR retrieval drop the whole strand before it ever reached consensus; this, not the voting algorithm, was the dominant blocker. Fixed via bounded edit-distance boundary search (`nucle_index::primer::tests::test_untag_tolerates_*`).

Third: pairwise realignment against one arbitrarily-picked noisy reference read turned out to have a hard ceiling once a single read carries several simultaneous indels (the realistic Nanopore regime for a 150+nt strand) — the reference's own errors and each read's individual drift compounded into wrong votes scattered across the strand. Fixed by replacing pairwise realignment with genuine **partial-order alignment (POA)**: `nucle_ecc::consensus::PoaGraph` folds every read into one shared DAG instead of comparing each to a single anchor, with edge weights (not just node visitation) so a majority "don't insert here" can correctly outvote a minority stray insertion at any position, including the very first or last base (`nucle_ecc::consensus::tests::test_boundary_insertion_outvoted_by_clean_majority`, `test_consensus_corrects_frame_shifting_indels`). Getting this right took real trial and error: a scoring tie let plain substitution runs get spuriously realigned, "maximize total nodes visited" always preferred a detour over stopping short of it, and node identity based on predecessor-set equality could alias two genuinely different reference positions together once enough reads had passed through — each is now a dedicated regression test. The graph is also fuzz-tested against realistic Nanopore error rates at 50x coverage for crash-safety (`test_high_coverage_realistic_nanopore_fuzz_does_not_crash`): a self-loop could otherwise form via the exact-base-match fast path, which the sibling-reuse cycle checks didn't cover, so the cycle check now lives in the single choke point every edge passes through (`PoaGraph::add_pred_if_missing`) instead of being re-derived per call site.

With all of that fixed, `build_consensus` also runs multi-round polishing now, not just a single POA pass: after the first pass, it reseeds a fresh graph from that pass's own (already-corrected) result — unweighted, so the backbone doesn't get double-counted as an extra vote — and re-folds every read, repeating to a fixed point, the same iterative approach real long-read polishers (Racon, Medaka) use. Getting this to *actually* be safe took a real caught-and-fixed regression of its own: the first attempt at it briefly broke the working Illumina case, from double-counting a read's vote once via the unweighted-seed omission being missing and once via the fold — fixed by `PoaGraph::seed_unweighted`, which is exactly the fix, not a workaround to avoid polishing. Polishing is verified safe (full workspace suite green, including Illumina) and does measurably help.

A synthetic 30-read stress test (3-6 edits each over a 43nt sequence — a higher combined edit rate than Nanopore's own ~7%) still landed 1 base off even after polishing converged, and the *first* diagnosis for that — "column identity occasionally fragmenting near a compounding cluster of edits" — turned out to be wrong once actually tested. The real cause: sequential POA construction is fold-order dependent. Whichever read gets folded into the graph first shapes which alternate nodes exist at all, and a later read can "snap" onto an early alternate for loosely-related reasons even when its own content doesn't really support it, letting an accidental fold-order majority snowball around the wrong interpretation at a position where several reads' unrelated edits happen to cluster. Polishing alone can't fix this, because every round reuses the same fold order — confirmed directly by folding the exact same reads in reverse order and getting the exactly correct answer instead, with no other change. `build_consensus` now re-runs the pipeline with a second (reversed) and, if needed, a third (rotated) fold order, taking whichever result at least two of three orderings agree on. This fully resolves the stress test (exact match, not just "close"; see `nucle_ecc::consensus::tests::test_consensus_exactly_recovers_original_under_many_simultaneous_indels_per_read`), and it's gated on the primary pass's own weakest per-position confidence (`< 90%`) so the extra orderings only run when there's a real reason to doubt the first answer — a clean, high-agreement result at realistic Illumina-grade noise returns after one pass, unchanged in cost.

Nanopore's own full end-to-end recovery still failed at realistic settings even with all of that, and chasing why surfaced a bug that wasn't in the consensus algorithm at all: `nucle_codec::ternary::TernaryCodec` pads unused strand length with a *constant* trit, and its 4-byte length header has several leading zero bytes for any file under 16MB. The rotating cipher maps a constant trit to a constant rotation relative to whatever base precedes it, so a long run of one trit value degenerates into a short, fixed-period base cycle once it goes through the cipher — a run of trit `0` became a literal `TATATATATATATATATATATAT...` repeat, dozens of bases long, at the start of essentially every encoded file. That self-inflicted tandem repeat — nothing to do with noise or the aligner — is exactly the kind of region that's hardest for any consensus scheme to recover under indel noise, and was the real cause of several residual errors that looked, at first glance, like a fundamental POA limitation. Fixed with `TernaryCodec::whiten_segment`: every strand's trits (including its padding) are XOR-added (mod 3) with a deterministic, position-addressable pseudo-random stream before the cipher sees them, and un-whitened per-strand at decode using that strand's own known index — position-addressable rather than a sequential-state PRNG specifically because overlapping segments see the same absolute trit position more than once, and decode doesn't know a strand's real-data/padding boundary until after decoding its length header, so unwhitening has to work without needing that boundary.

Verified: the ground-truth encoded strands for the `nucle_vfs` Nanopore tests no longer contain any long repeats (previously up to 140+ characters of a 2-base alternation), and residual consensus errors under real Nanopore noise are now small, localized 1-2-base insertions rather than sprawling repeat-region corruption. At real Nanopore noise density nearly every position is genuinely contested, so the confidence gate rarely skips the extra fold orderings — a real, non-hidden compute cost that buys real correctness where it's needed.

That narrowed the remaining gap to "Reed-Solomon can't correct a strand that survives consensus wrong-but-present rather than missing" — but Reed-Solomon itself turned out to have two real, previously undiscovered bugs of its own, unrelated to consensus or the codec. First: parity symbols are arbitrary GF(256) values spanning the full 0-255 range, but `dna_write` packed each one into DNA via the same 2-bit `Nucleotide::from_bits` used for data strand bytes (which are always pre-restricted to 0-3) — any parity byte outside that range was silently dropped, destroying the overwhelming majority of every parity strand's content without ever raising an error. Second: `consensus_then_rs_decode`'s parity conversion collapsed a strand that failed consensus out of the array entirely (`filter_map`) rather than leaving a gap at its position, which shifted every later parity strand onto the wrong evaluation point (`x = k + j` used the strand's position in the surviving subset, not its true configured index) and corrupted the whole stripe regardless of how many strands were actually wrong. Fixed by packing each parity byte into 4 bases (`Nucleotide::byte_to_bases`/`bases_to_byte`, `DnaStrand::from_packed_bytes`/`unpack_bytes`) and representing every erasure — data or parity — as `Option`-per-slot end to end, so a missing strand's true codeword position is never lost (`reed_solomon::tests::test_rs_parity_reindexing_does_not_corrupt_decode`).

With those two bugs fixed, `ReedSolomon::decode_stripe` was rewritten from plain erasure-only Lagrange interpolation to genuine combined error-and-erasure decoding via the Berlekamp-Welch algorithm (`GF256::solve_linear_system`, `GF256::poly_divmod`, `ReedSolomon::try_welch_decode`): it can now blindly correct up to `parity_count / 2` strands that come back from consensus wrong-but-present, without ever being told which ones, in addition to reconstructing up to `parity_count` known-missing strands — combinable per the standard `2*errors + erasures <= parity_count` bound. Verified directly with dedicated unit tests (`test_rs_corrects_silent_error_without_knowing_position`, `test_rs_combines_erasure_and_blind_error`), and the full workspace suite — including the 50x-coverage Nanopore fuzz test — passes with zero regressions.

Nanopore's full end-to-end recovery still fails at realistic settings even with all of this (`nucle_vfs::tests::test_nanopore_still_fails_at_realistic_indel_density_despite_alignment_fixes`, verified at 50x coverage and 12 parity strands; `nucle benchmark -p nanopore -r 4` and `-r 12` both still report FAIL at the CLI's real defaults, ~14% combined error rate). But this time the diagnosis is pinned down by direct ablation rather than inferred: comparing `-r 0` (consensus only, no Reed-Solomon at all) through `-r 50` (25-strand blind-error tolerance) on the identical noisy input produces the *exact same* decode failure at every redundancy level — proof that Reed-Solomon was never in the critical path for this specific failure, because consensus itself does not reliably converge to the correct sequence at Oxford Nanopore's real per-base error rate, before Reed-Solomon ever gets a chance to help. No amount of outer-code redundancy can fix an error that happens upstream of it. Closing this needs a better consensus/alignment algorithm for extreme indel density — an active research problem, not a parameter to tune — and remains open.

## Retrieval Architecture

```
Query → [Vector Index] → [Primer Resolution] → [CRISPR-sim Amplification] → [Strand Retrieval]
```

- Each file tagged with unique PCR primer pair
- Vector index enables content-addressable lookup
- CRISPR simulation models selective amplification
- Cross-talk modeling accounts for non-specific amplification

## VFS Abstraction

The VFS layer presents DNA storage as a device:

```rust
// Core syscall-style interface
fn dna_write(name: &str, data: &[u8], redundancy: u32) -> Result<FileHandle>;
fn dna_read(query: &str) -> Result<Vec<u8>>;
fn dna_stat(pool: &DnaPool) -> Result<PoolStats>;
fn dna_delete(name: &str) -> Result<()>;
```

### Durable, cross-process persistence

`NucleOS::new(max_files)` is still purely in-memory (used for short-lived/scratch instances — benchmarks, `nucle doctor`'s roundtrip probe). For a real, durable pool, `NucleOS::open(pool_dir, max_files)` loads whatever a prior `NucleOS::persist(pool_dir)` call wrote to `pool_dir/state.json` (or initializes fresh state if the directory is new), and `persist` writes back atomically (a temp file, then `fs::rename`d over the real path, so a process killed mid-write never corrupts the last good state). Only `pool`/`catalog`/the primer-index counter are actually persisted — `primers` regenerates identically from the same fixed seed given the same `max_files`, and `search` is rebuilt by replaying every file already in the restored catalog, so neither needs its own on-disk copy. `nucle_cli` resolves where a pool lives via (in priority order) an explicit `--pool-dir` flag, `NUCLEOS_POOL_DIR`, or a project-local `.nucleos/` directory created on first use (see `resolve_pool_dir`/`open_pool`/`persist_pool` in `nucle_cli/src/main.rs`). This is what makes `nucle store` in one process visible to a `nucle retrieve` in a later, separate one.

### Explicitly out of scope

Beyond durable persistence (above), the following remain deliberate non-goals for the current design:

- **Encryption** — data is stored as plaintext DNA. A production system would add an encryption layer between the VFS and the codec, but that's orthogonal to the storage stack.
- **Access control / permissions** — no user model, no file ownership. Every caller has full read/write to the pool.
- **Concurrent writes** — the pool is single-writer per process; two processes writing to the same `pool_dir` at once can race (last `persist()` wins). Concurrent access requires external synchronisation.
- **POSIX semantics** — no real directory tree, no symlinks, no `seek()`. The catalog is still a flat key-value map (name → blob), but names are ordinary strings that can look like paths (`"docs/report.txt"`) — `nucle store docs/readme.txt`/`nucle store downloads/readme.txt` from a relative path don't collide, and `Catalog::list_prefixed`/`nucle list <prefix>` filters by that prefix. This is prefix filtering over a flat map, not real tree traversal (no rename-a-directory, no recursive delete) — an absolute source path still strips to its bare leaf name, since that structure is local-filesystem noise, not an intentional namespace.

These boundaries are intentional. The VFS owns the question "how do I store and retrieve a named blob in DNA, durably?" — everything else belongs to layers above it.

## Hardware Bridge and Provider Boundaries

The hardware boundary separates the high-level compiler planner from the physical/simulation hardware execution:

```
[NucleScript compilation] → [HardwareRequest batches] → [Provider implementation]
                                                            ├── MockProvider (simulates, instant)
                                                            ├── DelayedMockProvider (simulates real latency on a std::thread)
                                                            └── FileExportProvider (JSON export)
```

- **HardwareRequest**: Models a typed transaction representing a physical operation. `RequestType` has five kinds: `Synthesis`/`Sequencing`/`Destructive` (cost-bearing or destructive, always `confirmation: "hardware"`/`"physical_key"`, gated by `--confirm`) and two read-only kinds requiring no confirmation — `Qc { file_name, checks }`, derived from a `pipeline { ..., verify roundtrip }` stage, and `Recovery { binding_name, consensus_method }`, derived from every `consensus_vote(...)` call (which always produces a `PoolState::Recovered` binding). Both are `effect: Effect::Pure` with an empty `confirmation` string — a deliberate design choice (not a default), since neither touches real synthesis/sequencing hardware or destroys data. Lives in `nucle_lang::hardware` — that module only ever defines and collects request *types*; it does not implement an execution trait itself. (An earlier `HardwareBridge` trait duplicated that concern with zero implementations and was removed in favor of `Provider` below, so there is exactly one execution-side trait, not two unrelated ones.)
- **Provider Trait**: The sole execution boundary, defined in `nucle_hardware::provider`. `submit(&self, batch) -> Box<dyn JobHandle>` is the required method — it returns immediately with a handle to poll (`status()`) or block on (`wait()`), rather than blocking until the batch finishes. `execute_batch` is now a default method built on `submit(...).wait()`, so every existing single-batch caller is unaffected. `JobStatus` is `Pending`/`Running`/`Complete(String)`/`Failed(String)`. `MockProvider`/`FileExportProvider` wrap their instant logic in an `ImmediateJobHandle` (already-terminal the moment it's created); `DelayedMockProvider` spawns a real `std::thread` per submission (sleeping a configurable delay before completing) to demonstrate genuine concurrent hardware submission — multiple `submit()` calls run in parallel rather than blocking each other, closing the gap the project's own action plan named as deferred ("no concurrent hardware submission model yet"). No new crate dependencies were added — this is `std::thread`/`std::sync` only, matching the hardware/CLI layer's dependency-light style (`nucle_lsp` separately depends on `tokio` via `tower-lsp`, unrelated to this path). Real vendor adapters (Twist, IDT, Illumina, Oxford Nanopore) would implement `Provider` in their own module under `nucle_hardware/src/`, once the request model has been exercised for a while.
- **`nucle hardware export`**: The CLI entry point. It first runs the compiler's own effect/confirmation check (`nucle_lang::typeck::check_program`) — a `.nsl` program missing `confirm hardware`/`confirm physical_key` in source is rejected before its requests are ever collected. It then requires an explicit `--confirm` flag whenever the collected batch contains any non-`Pure` effect, as a second, operator-level acknowledgment distinct from the language-level one. `--provider` selects `file-export` (default, writes to `--output`), `mock` (dry run, nothing persisted), or `mock-delayed` (dry run with a configurable `--simulated-delay-ms`, default 500, on a background thread); an unrecognized name (e.g. a vendor like `twist`) is accepted but falls back to `file-export` with a printed notice, since no vendor-specific adapter exists yet. `export` accepts one or more source files: a single source behaves exactly as before (a flat JSON success object); two or more sources submit all their batches concurrently — every `compile_source` compile step and the `--confirm` gate are checked for *every* source upfront (all-or-nothing: nothing is submitted if any source would be rejected), then every batch is submitted back-to-back and waited on together, so N sources never block on each other even against `mock-delayed`'s real per-job latency. Each source's `file-export` output path gets an inserted `_<index>` before the extension (`batch.json` → `batch_1.json`, `batch_2.json`, ...) so concurrent writes never collide; JSON output for 2+ sources is an array of per-source objects tagged with `"source"`, a new shape that only appears in the multi-source case.

## `nucle doctor`

Environment sanity check, run from the workspace root, so a confusing bug
report can first be ruled out as "the environment isn't what we think it is."
Each check reports pass/fail/skipped independently rather than a single
opaque status:

- **Workspace crate versions** — reads every crate's `Cargo.toml` and checks
  it inherits `version.workspace = true` rather than a hardcoded override
  (the actual mechanism that keeps workspace versions consistent, not a
  runtime comparison of values Cargo already guarantees are equal).
- **Presets package manifest** — runs the same manifest validation
  `nucle package verify` uses (non-empty name/exports, known export kinds).
- **Standard fixtures present** — checks `docs/examples/fixtures/` has the
  expected text/binary/FASTA/image files and the `project_tree/` directory.
- **Example programs parse** — actually lexes and parses every `.nsl` file
  under `docs/examples/` (excluding `failures/`), not just checking they exist.
- **Failure-mode examples parse** — same, but for `docs/examples/failures/`:
  those programs are supposed to fail *type checking* by design, so this
  only asserts they're still syntactically valid NucleScript.
- **VFS write/read roundtrip** — runs a real `dna_write`/`dna_read`
  roundtrip against a scratch, ephemeral in-memory instance (`NucleOS::new`,
  deliberately not the real persistent pool) as the VFS pipeline's
  equivalent of a scratch read/write check, without touching or polluting
  real stored data with its probe file.

A check that can't run at all from the current directory (e.g. a directory
genuinely doesn't exist) is reported `skipped`, not `failed` — it degrades
gracefully rather than treating "couldn't check" the same as "checked and
it's broken."
