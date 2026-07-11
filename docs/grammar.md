# NucleScript Grammar Reference

This document defines the formal syntax and grammar of NucleScript (`.nsl`), the domain-specific language of NucleOS.

---

## EBNF Grammar

```ebnf
Program             ::= ( Declaration | ',' )*

Declaration         ::= ImportDecl
                      | PoolDecl
                      | EnumDecl
                      | StrandDecl
                      | SequenceDecl
                      | LetDecl
                      | FunctionDecl
                      | Operation
                      | PipelineDecl
                      | IfDecl
                      | ForDecl
                      | TestDecl

ImportDecl          ::= 'import' '{' ImportItemList '}' 'from' StringLiteral
ImportItemList      ::= ImportItem ( ',' ImportItem )* ','?
ImportItem          ::= Identifier ( 'as' Identifier )?

PoolDecl            ::= 'pool' Identifier ':' 'DnaPool' '{' PoolPropertyList '}'
PoolPropertyList    ::= PoolProperty ( ',' PoolProperty )* ','?
PoolProperty        ::= 'codec' ':' CodecLiteral
                      | 'redundancy' ':' MultiplierLiteral
                      | 'profile' ':' ProfileLiteral

CodecLiteral        ::= 'YinYang' | 'yin-yang' | 'Ternary' | 'ternary-rotating' | 'ternary-rotating-cipher' | 'Fountain' | 'dna-fountain'
ProfileLiteral      ::= 'Illumina' | 'Nanopore' | 'oxfordnanopore' | 'oxford-nanopore' | 'Twist' | 'twistbioscience' | 'twist-bioscience'

// A user-defined sum type -- at most one payload per variant
// (never a tuple of several), mirroring 'Ok(T)'/'Err(E)''s own shape.
// 'Result' is reserved: 'enum Result { ... }' is E-ENUM-RESERVED-NAME,
// since Result is a built-in, privileged type, not an instance of this
// general mechanism. See "Enums" below.
EnumDecl            ::= 'enum' Identifier '{' EnumVariantList '}'
EnumVariantList     ::= EnumVariant ( ',' EnumVariant )* ','?
EnumVariant         ::= Identifier ( '(' TypeExpr ')' )?

StrandDecl          ::= 'strand' Identifier ':' 'Strand' '=' StringLiteral
SequenceDecl        ::= 'seq' Identifier ':' 'Sequence' '=' StringLiteral

LetDecl             ::= 'let' Identifier ( ':' TypeExpr )? '=' Expr
                      | 'let' Identifier ':' 'Sequence' '=' 'seq' StringLiteral
                      | 'let' Identifier '=' 'seq' StringLiteral

FunctionDecl        ::= 'fn' Identifier TypeParamList? '(' FnParamList? ')' ( '->' | 'returns' ) TypeExpr '{' Declaration* '}'
                      // the return type is mandatory — a function with no
                      // meaningful return value still writes `returns Void`
                      // rather than omitting it; the parser rejects a
                      // missing '->'/'returns' clause instead of defaulting
TypeParamList       ::= '<' Identifier ( ',' Identifier )* '>'
                      // usable only as PoolState's 'Var' case below, in
                      // this function's own params/return type/body --
                      // see "Generics" below.
FnParamList         ::= FnParam ( ',' FnParam )* ','?
FnParam             ::= Identifier ':' TypeExpr

TypeExpr            ::= 'Pool' '<' PoolState ( ',' PercentLiteral )? '>'
                      | 'Strand' | 'Sequence' | 'File' | 'DnaFile' | 'Recovery' | 'Void'
                      | 'Result' '<' TypeExpr ',' TypeExpr '>'
                      | 'Str'
                      | 'Fn' '(' ( TypeExpr ( ',' TypeExpr )* )? ')' ( '->' | 'returns' ) TypeExpr
                      | Identifier
                      // 'Result<T, E>' and generic 'Pool<T>' (via PoolState's
                      // 'Var' case below) are the only two generic
                      // mechanisms NucleScript has -- no general
                      // 'Type<...>' system exists; each is its own
                      // hardcoded parse path. 'Str' is meaningful only
                      // as 'Result<_, Str>''s error slot: every VFS
                      // failure is a plain message string, and there is no
                      // string arithmetic or any other place 'Str' is
                      // expected. 'Fn(...)' is a closure/function's own
                      // type -- always non-generic, usable as a 'let'
                      // annotation or a function parameter's type. The
                      // bare 'Identifier' case is presumed to
                      // name a user 'enum' -- the parser accepts any
                      // identifier here unconditionally, and typeck
                      // resolves/validates it against the declared 'enum'
                      // table, reporting an ordinary unresolved-name
                      // failure if it isn't one. See "Result / Error
                      // Propagation", "Generics", "Closures", and "Enums"
                      // below.
PoolState           ::= 'Illumina' | 'Nanopore' | 'Twist' | 'Amplified' | 'Recovered' | Identifier
                      // the bare Identifier case is only ever a name
                      // already declared in the enclosing FunctionDecl's
                      // TypeParamList -- any other identifier here is a
                      // parse error ("unknown pool profile or state"),
                      // unchanged from before generics existed.

// `if`/`for` are resolved at COMPILE TIME, not runtime: NucleScript's
// execution model is "compile a static plan, then run it," so there is no
// runtime branch or loop anywhere in a compiled program. The type checker
// evaluates `Condition` once, keeps only the taken branch (the untaken
// branch is never type-checked, similar to `#[cfg(...)]`), and unrolls a
// `for` by textual substitution of `Binding` with each item -- the
// compiled output never contains an `IfDecl`/`ForDecl` node. See
// `nucle_lang::ast::IfDecl`/`ForDecl` for the full rationale.
IfDecl              ::= 'if' Condition '{' Declaration* '}' ( 'else' '{' Declaration* '}' )?
ForDecl             ::= 'for' Identifier 'in' IdentOrStringList '{' Declaration* '}'
IdentOrStringList   ::= ( Identifier | StringLiteral )
                      | '[' ( Identifier | StringLiteral ) ( ',' ( Identifier | StringLiteral ) )* ','? ']'

// Condition must reduce entirely to comparisons/booleans over numbers and
// probabilistic pool bindings -- there is no runtime to defer evaluation
// to. A bare `Identifier` naming a probabilistic pool binding resolves, in
// this numeric context only, to that binding's inferred error-rate percent
// (e.g. `if noisy > 0.5 { ... }` compares `noisy`'s observed error rate
// against `0.5`) -- this is the one deliberate coercion the language
// defines, in place of general field-access syntax.
Condition           ::= Condition '||' AndCondition | AndCondition
AndCondition        ::= AndCondition '&&' NotCondition | NotCondition
NotCondition        ::= '!' NotCondition | Comparison
Comparison          ::= NumericExpr ( '==' | '!=' | '<' | '>' | '<=' | '>=' ) NumericExpr
                      | '(' Condition ')'
NumericExpr         ::= NumberLiteral | Identifier

Expr                ::= 'simulate' Identifier 'under' ProfileLiteral
                      | ( 'synthesise' | 'synthesize' ) Identifier 'via' ProfileLiteral ( 'confirm' 'hardware' )?
                      | 'sequence' Identifier 'via' ProfileLiteral ( 'confirm' 'hardware' )?
                      | 'consensus_vote' '(' Identifier ',' 'coverage' ':' MultiplierLiteral ')'
                      | 'protect' Identifier 'for' Identifier
                      | StoreOp | RetrieveOp | DeleteOp
                      | Identifier '(' ExprList? ')'
                      | Identifier
                      | StringLiteral
                      | NumberLiteral
                      | '(' Expr ')'
                      | Expr ( '==' | '!=' | '<' | '>' | '<=' | '>=' ) Expr
                      | Expr '&&' Expr
                      | Expr '||' Expr
                      | '!' Expr
                      | Expr '?'
                      | MatchExpr
                      | ClosureExpr
                      | EnumConstructExpr
ExprList            ::= Expr ( ',' Expr )* ','?
                      // The boolean/comparison operators above bind exactly
                      // as in `Condition`: '||' loosest, then '&&', then
                      // unary '!', then a single non-chaining comparison,
                      // then a primary expression. There is no arithmetic
                      // ('+'/'-'/'*'/'/') -- literal numbers and a pool
                      // binding's inferred error rate are only ever compared,
                      // never combined. '?' binds tighter than comparison
                      // (like Rust: 'x? == y' means '(x?) == y') -- see
                      // "Result / Error Propagation" below.
                      //
                      // 'StoreOp'/'RetrieveOp'/'DeleteOp' appearing here
                      // (as opposed to only under 'Operation' below) reuses
                      // the exact same
                      // grammar 'store'/'retrieve'/'delete' already have
                      // as *statements*, now also usable in *expression*
                      // position (e.g. the right-hand side of a 'let') --
                      // one struct, two surface positions, per "Result /
                      // Error Propagation" below. The statement forms
                      // under 'Operation' are unaffected: the parser only
                      // ever produces this expression form after 'let x =',
                      // never at the top of a declaration.

// Destructures either a Result<T, E>-shaped Expr (Ok/Err are always its
// two variants) or a user 'enum'-shaped Expr, through one general N-arm
// engine -- arm order is free (checked by variant name, not
// position), and exhaustiveness requires either one arm per declared
// variant or a trailing '_' wildcard covering the rest. See "Pattern
// Matching" below.
MatchExpr           ::= 'match' Expr '{' MatchArmList '}'
MatchArmList        ::= MatchArm ( ',' MatchArm )* ','?
MatchArm            ::= MatchPattern '=>' Expr
MatchPattern        ::= Identifier ( '(' Identifier ')' )?
                      | '_'
                      // 'Identifier' names a variant ('Ok'/'Err' for a
                      // Result scrutinee, or one of a user enum's declared
                      // variant names) with an optional payload binding;
                      // '_' is the wildcard, valid only as the last arm.

// Constructs a user enum value directly -- 'EnumName::Variant' for a unit
// variant, 'EnumName::Variant(payload)' for a payload-carrying one. See
// "Enums" below.
EnumConstructExpr   ::= Identifier '::' Identifier ( '(' Expr ')' )?

ClosureExpr         ::= 'fn' '(' FnParamList? ')' ( '->' | 'returns' ) TypeExpr '{' Declaration* '}'
                      // An anonymous function literal -- same params/
                      // return-type/body grammar as FunctionDecl, minus
                      // the name and TypeParamList (a closure is always
                      // non-generic). Its own type is TypeExpr's 'Fn(...)'
                      // case above. See "Closures" below.

Operation           ::= StoreOp
                      | RetrieveOp
                      | DeleteOp
                      | AssertOp

StoreOp             ::= 'store' ( StringLiteral | Identifier ) 'into' Identifier StoreOptions?
                      | 'simulate' 'store' ( StringLiteral | Identifier ) 'into' Identifier StoreOptions?
StoreOptions        ::= '{' StoreOptionList '}'
StoreOptionList     ::= StoreOption ( ',' StoreOption )* ','?
StoreOption         ::= 'redundancy' ':' MultiplierLiteral
                      | 'coverage' ':' MultiplierLiteral
                      | 'tag' ':' StringList
                      | 'tags' ':' StringList
                      | 'expect' 'recovery' '>' PercentLiteral

StringList          ::= StringLiteral
                      | '[' StringLiteral ( ',' StringLiteral )* ','? ']'

RetrieveOp          ::= 'retrieve' 'from' Identifier ( 'where' '{' QueryPredicateList '}' )?
QueryPredicateList  ::= QueryPredicate ( ',' QueryPredicate )* ','?
QueryPredicate      ::= Identifier QueryOp QueryValue

QueryOp             ::= 'contains' | '=' | '>' | '<'
QueryValue          ::= StringLiteral
                      | Identifier
                      | DateLiteral
                      | SizeBytesLiteral
                      | NumberLiteral

DeleteOp            ::= 'delete' ( StringLiteral | Identifier ) 'from' Identifier ( 'confirm' 'physical_key' )?

// Evaluated at compile time with the same `Condition` grammar an `if`
// uses (see the "Control Flow" section below) -- there is no runtime to
// defer an assertion to. Valid anywhere a declaration is (not just inside
// `test { ... }`), but `nucle test` only groups the ones lexically inside
// a `TestDecl` into that test's pass/fail result; `nucle check` surfaces
// an always-false assertion anywhere as a real diagnostic regardless.
AssertOp            ::= 'assert' Condition ( ',' StringLiteral )?

// A named block run by `nucle test` against a fresh, isolated VFS
// instance per test -- see docs/stdlib.md's sibling doc, docs/errors.md's
// `E-ASSERTION-FAILED`, and the "Control Flow" section below for why
// `assert`'s condition is a compile-time evaluation, not a runtime one.
TestDecl            ::= 'test' StringLiteral '{' Declaration* '}'

PipelineDecl        ::= 'pipeline' Identifier '{' PipelineStepList '}'
PipelineStepList    ::= PipelineStep ( ',' PipelineStep )* ','?
PipelineStep        ::= 'encode' StringLiteral 'using' CodecLiteral
                      | 'protect' 'with' 'redundancy' MultiplierLiteral
                      | 'store' 'into' Identifier
                      | 'verify' 'roundtrip'

Identifier          ::= [a-zA-Z] [a-zA-Z0-9_-]*
StringLiteral       ::= '"' [^"]* '"'
NumberLiteral       ::= [0-9]+ ( '.' [0-9]+ )?
MultiplierLiteral   ::= [0-9]+ [xX]
PercentLiteral      ::= [0-9]+ ( '.' [0-9]+ )? '%'
DateLiteral         ::= [0-9]{4} '-' [0-9]{2} '-' [0-9]{2}
SizeBytesLiteral    ::= [0-9]+ ( 'mb' | 'MB' | 'kb' | 'KB' )
```

---

## Control Flow (`if` / `for`)

Before `Result<T, E>`/`?` (below) existed, NucleScript had no runtime at
all: a program compiled to a static plan (a fixed list of pool schemas,
probabilistic bindings, and store/retrieve/delete calls) which was then
executed as-is. `if` and `for` predate that feature and still fit this
model exactly -- they are **compile-time** constructs, not true runtime
branching or looping:

```nsl
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina

// `noisy` resolves to its inferred error-rate percent (0.35) in this
// numeric comparison -- the type checker evaluates the condition once,
// at compile time, and keeps only the taken branch.
if noisy > 0.1 {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
} else {
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
}

for target in [archive] {
    store "genome.fasta" into target { redundancy: 4x }
}
```

- **`if`** only ever keeps one branch. The untaken branch is never
  type-checked at all (so, unlike a real `if`, a type error in the branch
  that's never taken will not be reported) -- this mirrors `#[cfg(...)]`
  more than a conditional statement.
- **`for`** always iterates a literal, statically-known list of identifiers
  and/or string literals -- never an open-ended `while` or a runtime
  collection. Each iteration is unrolled by substituting the loop binding
  with that iteration's value in a fresh copy of the body, and each copy is
  type-checked independently.
- Both are fully resolved away during type checking; `codegen`/the
  simulation backend only ever see a plain, control-flow-free program.

---

## Result / Error Propagation (`Result<T, E>` / `?`)

Unlike `if`/`for` above, this genuinely is runtime behavior -- the first
in this language. Before `Result<T, E>`/`?` existed, every `store`/`retrieve`/`delete`
either succeeded or aborted the entire program; there was no way for a
NucleScript program to observe, inspect, or recover from an operation
failure. `store`/`delete` (not `retrieve` -- see below) can now also
appear in *expression* position, producing a `Result<T, Str>` a `?`
can unwrap or propagate:

```nsl
pool primary: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
pool backup: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }

fn archive_with_fallback() returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = store "genome.fasta" into primary
    let saved: DnaFile = attempt?
}
```

- **`store`/`delete` in expression position** reuse the exact same
  `StoreOp`/`DeleteOp` grammar the *statement* form already has -- one
  struct, two surface positions. The statement form (a bare `store ...
  into ...`/`delete ... from ...` declaration) is completely unaffected
  and keeps its original all-or-nothing abort-on-failure behavior; only
  the new expression form produces a `Result`.
- **`retrieve` is never `Result`-shaped**, even in expression position --
  it already soft-fails today (an empty match list, never a real error),
  so there is no genuine failure for a `Result` to carry.
- **`?`** unwraps a `Result<T, E>`-typed expression to `T` on success, or
  short-circuits the *enclosing function* with that `Err(E)` on failure.
  It's only valid inside a function whose own declared return type is
  `Result<_, E>` with a matching `E` (`E-TRY-OUTSIDE-RESULT-FN`/
  `E-TRY-ERROR-TYPE-MISMATCH` otherwise) -- there is no top-level `?`, and
  no coercion between different `Err` types.
- **A function's tail `let` is its implicit return**, same convention as
  a `Pool<...>`-returning function. Two valid shapes: still-wrapped (the
  tail's own annotation is `Result<T, E>`, matching the function's
  declared return type exactly) or already-unwrapped via `?` (the tail's
  annotation is just `T`) -- a successful unwrapped tail is automatically
  re-wrapped into `Ok(T)` at the call boundary, so a caller always sees
  an ordinary `Result<T, E>` value regardless of which shape the callee
  used internally.
- **`Str`** is the one new primitive type this adds, meaningful only as
  `Result<_, Str>`'s error slot -- every VFS failure is a plain message
  string (`nucle_vfs`'s own `Result<T, String>`, unchanged), and nothing
  else in the language expects a `Str`.
- **Effect analysis treats a `?` exactly like an `if`'s untaken branch**:
  conservatively, always. A function's effect is the join across *every*
  declaration in its body, whether or not an earlier `?` might
  short-circuit before a later one runs -- a `Destructive` operation
  after a `?` still requires confirmation, since effect analysis is
  static and never models which declarations actually execute at
  runtime.
- **`match` lets a caught `Err` be inspected directly** --
  see "Pattern Matching" below. Before that existed, a caught `Err` could only be
  produced and propagated, never branched on from within the same
  program; building on it (retrying a different pool, logging why) needed
  a second, independent function call from the caller.
- **`Ok(<expr>)`/`Err(<string literal>)` construct a `Result`
  directly**, rather than only ever receiving one from `store`/`delete`/a
  `Result`-returning call. `Ok(...)`'s payload is any expression already
  resolvable to a concrete type -- a bound variable of any shape, `?`, a
  `Result`-shaped expression, or a `Pool`-shaped one; its missing `Err`
  side defaults to `Str` (the only error type anywhere in the language).
  `Err(...)`'s payload is restricted to a string literal -- the only way
  to author a *new* `Str` value -- and its missing `Ok` side has no
  default, coming instead from the enclosing `let`'s annotation, a
  sibling `Ok` match arm's already-resolved type, or the enclosing
  function/closure's declared return type; a bare `Err(...)` with none of
  those reports `E-ERR-CONSTRUCTOR-AMBIGUOUS`:
  ```nsl
  fn archival_disabled() returns Result<DnaFile, Str> {
      let disabled: Result<DnaFile, Str> = Err("archival is temporarily disabled by policy")
  }
  ```
- **A statement-form `store`/`retrieve`/`delete` inside a function body
  now actually executes**, not just at the top level. Before
  this fix, a bare `store "x.txt" into pool` declaration (no `let`, not
  producing the function's return value) inside any function body was
  silently skipped by the interpreter -- present in the source, type-
  checked, but never touching the real VFS. A statement-form failure
  aborts the enclosing function unconditionally (same all-or-nothing
  contract the top-level statement form always had), which a caller
  catches via the ordinary `?`/`match` machinery if that function itself
  returns `Result<_, _>`.
- **A `File`/`Str`-typed parameter now carries its real argument value
  at runtime**, rather than being an inert, type-checked-only
  label. `store <identifier> into <pool>`'s "file variable" syntax (an
  identifier instead of a string literal) resolves that identifier
  through the callee's own bound parameters, so a function like
  `fn archive_named(name: File) returns Result<DnaFile, Str> { store
  name into archive ... }` called as `archive_named("genome.fasta")`
  really does store `genome.fasta`, not a file literally named `name`.

See [docs/errors.md](errors.md) for the six new `E-TRY-*`/`E-BINDING-
RESULT-*`/`E-RETURN-TYPE-*` codes plus `E-OK-CONSTRUCTOR-INVALID`/
`E-ERR-CONSTRUCTOR-INVALID`/`E-ERR-CONSTRUCTOR-AMBIGUOUS`, and
[`docs/examples/result_fallback_store.nsl`](examples/result_fallback_store.nsl)
for a complete, runnable example.

---

## Generics (`fn name<T>(...)`)

A function generic over `Pool<T>`'s profile — the actual motivating pain
point: without this, the same logic needs one hardcoded copy per profile
(`Illumina`/`Nanopore`/`Twist`), since `Pool<Illumina>` and
`Pool<Nanopore>` are different concrete types with no shared supertype.

```nsl
pool illumina_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
pool nanopore_archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Nanopore }

fn recover_from<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {
    let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
}

let noisy_illumina: Pool<Illumina, 0.35%> = simulate illumina_archive under Illumina
let recovered_a: Pool<Recovered> = recover_from(noisy_illumina)

let noisy_nanopore: Pool<Nanopore, 5%> = simulate nanopore_archive under Nanopore
let recovered_b: Pool<Recovered> = recover_from(noisy_nanopore)
```

`recover_from` is declared and type-checked **once**, treating `P` as an
opaque, unresolved placeholder (`PoolState::Var("P")`) — not once per
concrete profile, and not through any runtime mechanism (`P` never
exists once type-checking finishes; the interpreter that runs
`Result<T,E>`/`?`'s function calls has no notion of "generic" at all).
At each call site, `P` is **unified** against that call's real argument:
the first `Pool<T>`-typed argument in a call binds `T`, and any later
argument using the same `T` must agree or the call is rejected
(`E-TYPE-PARAM-CONFLICT`). The resolved binding is then substituted into
the return type for that specific call, so `recover_from(noisy_illumina)`
and `recover_from(noisy_nanopore)` above both type-check as
`Pool<Recovered>`, from the exact same declaration.

- **Scope is deliberately narrow**: a type parameter is usable only as
  the state slot inside `Pool<T>` (parameters, the return type, and
  `let` annotations inside the body) — not on `Strand`/`Sequence`/
  `Result`/anything else. A type parameter that no argument binds is a
  real error (`E-TYPE-PARAM-UNRESOLVED`), not silently left generic —
  unless it's resolved explicitly (see below).
- **Explicit type-argument syntax, `name::<Illumina>(...)`**,
  for the one shape inference alone can't resolve: a type parameter that
  appears *only* inside a `Fn(...)`-typed parameter's own signature,
  never as a directly `Pool<P>`-shaped argument. An explicit argument
  that later disagrees with what an ordinary argument would infer
  reports the same `E-TYPE-PARAM-CONFLICT` a conflicting pair of ordinary
  arguments already does; the wrong number of explicit arguments reports
  `E-TYPE-PARAM-ARITY`:
  ```nsl
  fn recover_generically<P>(source: Pool<Illumina, 0.35%>, recover_fn: Fn(Pool<P, 0.35%>) -> Pool<Recovered>) returns Pool<Recovered> {
      let recovered: Pool<Recovered> = recover_fn(source)
  }
  let recovered: Pool<Recovered> = recover_generically::<Illumina>(noisy_illumina, fn<P>(source: Pool<P, 0.35%>) -> Pool<Recovered> {
      let recovered: Pool<Recovered> = consensus_vote(source, coverage: 10x)
  })
  ```
- **No trait-bound-style constraints exist**, because nothing in the
  type system needs one — every operation a `Pool<T>`-typed value can be
  used for already works identically across all three profiles (effect
  classification never depends on profile at all).
- **A real, honest limitation**: a handful of profile-*specific* typeck
  warnings (e.g. `E-STORE-UNSATISFIABLE-RECOVERY`, which only fires for
  `Nanopore`) can't fire while checking a generic body against an
  abstract `P` — there's no concrete profile yet to check against. They
  still fire correctly for the equivalent non-generic code, and would
  still fire on a *non-generic* argument passed through a generic
  parameter's own call-site checks to the extent those inspect the
  argument's real type. This is the direct language-level analog of
  `consensus_vote`/`protect`'s existing "type system honesty" precedent
  (see [docs/stdlib.md](stdlib.md)) — a documented gap, not a silent one.

See [docs/errors.md](errors.md) for `E-TYPE-PARAM-CONFLICT`/`E-TYPE-PARAM-
UNRESOLVED`/`E-TYPE-PARAM-ARITY`, and
[`docs/examples/generic_pool_recovery.nsl`](examples/generic_pool_recovery.nsl)
(a generic closure nested inside a generic function) and
[`docs/examples/explicit_type_args_and_file_param.nsl`](examples/explicit_type_args_and_file_param.nsl)
(the explicit-type-argument-required case) for complete, runnable
examples.

---

## Enums (`enum` declarations)

NucleScript's only sum type used to be `Result<T, E>`, closed to exactly
two built-in variants, so `match` never needed a general exhaustiveness
algorithm. `enum` adds a real user-defined sum type, and `match` is one
general engine that handles both `Result` (as a built-in, privileged
2-variant pseudo-enum) and a real user `enum` uniformly:

```nsl
enum RecoveryPlan {
    Retry,
    Fallback,
    GiveUp(Str),
}

let plan: RecoveryPlan = RecoveryPlan::Fallback
```

- **At most one payload per variant** (`Variant` or `Variant(Type)`,
  never a tuple of several) -- mirroring `Ok(T)`/`Err(E)`'s own existing
  shape. Tuple/struct variants don't exist.
- **`Result` is a reserved name** -- `enum Result { ... }` is
  `E-ENUM-RESERVED-NAME`, since `Result` stays a distinct, privileged
  built-in (see "Deliberate non-unification," below), not an instance of
  this general mechanism.
- **`EnumName::Variant`/`EnumName::Variant(payload)` construct a value.**
  The `::` token is the same one turbofish uses
  (`name::<Illumina>(...)`); the two are unambiguous by one token of
  lookahead — turbofish's `::` is always followed by `<`, a variant
  reference's `::` is always followed by an identifier. A payload's
  presence/absence and type must match the variant's own declaration
  exactly (`E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH` otherwise).
- **Deliberate non-unification with `Result`/`Ok`/`Err`**: `enum` is a
  parallel mechanism, not `Result`'s generalization. `Ok(...)`/`Err(...)`
  keep their own dedicated, unprefixed construction syntax and
  `Value::Result`'s own runtime representation, untouched — `Err`'s
  string-literal-only payload restriction doesn't generalize to a real
  `enum`'s variants, and every existing `.nsl` file's bare `Ok(x)`/
  `Err("msg")` needed to keep working with zero parser changes. The
  unification the user-facing behavior actually gets is entirely at the
  **matching** layer (below) — construction and runtime storage for
  `Result` and a user `enum` stay genuinely distinct.

See [docs/errors.md](errors.md) for `E-ENUM-DUPLICATE`/
`E-ENUM-RESERVED-NAME`/`E-ENUM-EMPTY`/`E-ENUM-VARIANT-DUPLICATE`/
`E-ENUM-CONSTRUCT-UNKNOWN-ENUM`/`E-ENUM-CONSTRUCT-UNKNOWN-VARIANT`/
`E-ENUM-CONSTRUCT-PAYLOAD-MISMATCH`.

---

## Pattern Matching (`match` / `Ok` / `Err` / user enums)

Before `match` existed, a caught `Err` could
only be inspected by a second, independent function call from the
caller, never branched on from within the same function. `match`
destructures a `Result<T, E>`-shaped expression, or a real
user-`enum`-shaped one, binding each arm's payload to a name visible only
within that arm:

```nsl
pool primary: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }
pool secondary: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }

fn archive_with_fallback() returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
    let saved: DnaFile = match attempt {
        Ok(file) => file,
        Err(reason) => (store "sample_b.txt" into secondary)?
    }
}
```

- **One general engine handles both scrutinee kinds** --
  `check_match` resolves what's being matched to either the built-in
  `Result` pseudo-enum (its declared variants are always exactly `Ok(T)`/
  `Err(E)`) or a real, looked-up `EnumDecl`, then runs the same
  exhaustiveness/dispatch logic either way. A scrutinee that resolves to
  neither (e.g. a bare `Pool<...>` binding) is
  `E-MATCH-UNRECOGNIZED-SCRUTINEE` (renamed from the earlier
  `E-MATCH-NOT-RESULT`, since "not `Result`" is no longer the whole
  story).
- **Arm order is free, checked by variant name, not position** -- a
  deliberate behavior change from an earlier version of the language,
  when `Ok` had to come
  first and `Err` second. `match r { Err(e) => ..., Ok(v) => ... }` is
  now exactly as valid as the reverse.
- **Exhaustiveness**: every declared variant needs exactly one matching
  arm, or a trailing wildcard `_` (which must be last) covers whatever
  isn't named — `E-MATCH-NON-EXHAUSTIVE`/`E-MATCH-ARM-AFTER-WILDCARD`/
  `E-MATCH-DUPLICATE-ARM`/`E-MATCH-UNKNOWN-VARIANT` cover the ways this
  can go wrong. For a 2-variant `Result` match this is unchanged from
  before (both `Ok` and `Err` still need covering, one way or another);
  for a 3+-variant user `enum` it's the first place NucleScript actually
  enforces exhaustiveness.
- **Every present arm must unify to one type**, which becomes the whole
  `match` expression's type -- usable directly as a `let`'s RHS, exactly
  like a function call or any other expression
  (`E-MATCH-ARM-TYPE-MISMATCH` otherwise). If every arm of a `Result`
  match happens to still be `Result`-shaped (none used `?`), the match's
  own value is itself a still-wrapped `Result` a caller can `?`/re-match
  later.
- **An arm's body is one of a fixed, closed set of shapes** (never a bare
  literal, the same restriction `Ok(...)`'s own payload has --
  `E-MATCH-ARM-UNTYPABLE` otherwise): the pattern's own bound name
  (`Ok(file) => file`, or `Info(msg) => msg` for a user enum whose
  variant shares its payload type); `Ok(<pattern>)`/`Err(<pattern>)`
  re-wrapping the arm's own bound value (using the scrutinee's *declared*
  variant list for the missing side's type -- no dependency on a
  sibling arm's own resolved value); the general
  `EnumName::Variant(<pattern>)` re-wrap for a user enum; `?` applied to
  a fallible expression (checked against the *enclosing function's*
  return type); **a nested `match`** --
  accepted whatever `check_match` itself resolves the inner match to,
  Result-shaped, enum-shaped, Pool-shaped, or already bare/unwrapped, not
  just Result-shaped as originally built; a still-wrapped
  `Result`-shaped expression; an enum-shaped expression; or a
  `Pool<...>`-shaped expression.
- **Composability with `?` and nested `match`**: a `match`'s scrutinee
  can itself be another `match` expression (`match (match a {...}) {...}`
  ), and `?` applies directly to a `match` expression's own result
  (`(match a {...})?`) -- both fall out of `infer_result_expr`/
  `infer_scrutinee_kind` recognizing a nested `match` as potentially
  Result-/enum-shaped, benefiting every caller (`check_try`, `check_
  match`'s own scrutinee check, `check_match_arm_general`) automatically:
  ```nsl
  enum RecoveryPlan { Retry, Fallback, GiveUp(Str) }

  fn archive_with_plan(plan: RecoveryPlan) returns Result<DnaFile, Str> {
      let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
      let saved: DnaFile = match attempt {
          Ok(file) => file,
          Err(reason) => match plan {
              Retry => (store "sample_a.txt" into primary)?,
              _ => (store "sample_b.txt" into secondary)?,
          }
      }
  }
  ```
- **Arm bodies are a single expression, not a block** -- matching the
  language's existing convention that there's no bare-block-as-expression
  anywhere (`?` is the model: it wraps exactly one inner expression). An
  arm that needs a fallback operation writes it directly as its one
  expression, as `(store ... into ...)?` does above; it doesn't get its
  own local `let` sequence the way a function body does.
- **A genuine, narrower limitation remains**: `Err(...)`'s payload must
  always be a literal string, even inside a user-enum match arm -- a
  bound `Str` variable (a user enum's own payload, or an outer match
  arm's own binding) can never be re-wrapped into a fresh `Err(...)`,
  since the only mechanism for resolving `Err`'s missing `Ok` side is the
  `Ok`/`Err` sibling-arm trick within the *same* `Result` match, which
  doesn't apply to a user-enum match at all. Writing around this means
  producing the `Err` through a real, self-contained fallible expression
  (e.g. `(store ... into ...)?`) rather than hand-crafting the message.
- **Effect analysis joins the scrutinee and every arm unconditionally**,
  the same conservative "every declaration in this join counts" rule
  `if`/`?` already follow -- a `Destructive` operation in only one arm
  still requires confirmation, since this analysis has never modeled
  "this branch might not run."

See [docs/errors.md](errors.md) for `E-MATCH-UNRECOGNIZED-SCRUTINEE`/
`E-MATCH-UNKNOWN-VARIANT`/`E-MATCH-NON-EXHAUSTIVE`/`E-MATCH-DUPLICATE-
ARM`/`E-MATCH-ARM-AFTER-WILDCARD`/`E-MATCH-ARM-TYPE-MISMATCH`/
`E-MATCH-ARM-UNTYPABLE`, and
[`docs/examples/match_result_fallback.nsl`](examples/match_result_fallback.nsl)/
[`docs/examples/recovery_plan.nsl`](examples/recovery_plan.nsl) for
complete, runnable examples (the latter exercising a user `enum`, a
nested match, and exhaustiveness via a trailing wildcard).

---

## Closures (`Fn(...)` / `fn(params) -> T { body }`)

`fn name<T>(...)`-style declarations are still named, top-level, and
fixed-signature — there was previously no way to write an anonymous
function, bind one to a variable, or pass one as an argument. `Fn(...)`
is a closure's own type; `fn(params) -> T { body }` in expression
position is the closure literal itself:

```nsl
pool primary: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }

fn retry_once(attempt_fn: Fn() -> Result<DnaFile, Str>) returns Result<DnaFile, Str> {
    let attempt: Result<DnaFile, Str> = attempt_fn()
    let saved: DnaFile = match attempt {
        Ok(file) => file,
        Err(reason) => attempt_fn()?
    }
}

fn archive_with_retry() returns Result<DnaFile, Str> {
    let result: Result<DnaFile, Str> = retry_once(fn() -> Result<DnaFile, Str> {
        let attempt: Result<DnaFile, Str> = store "sample_a.txt" into primary
    })
}
```

- **Capture is real, lexical, and by snapshot** — a closure's body can
  reference any name already bound in its enclosing scope at the exact
  point of the literal (params, `let` bindings of any shape, other
  already-defined closures). This is the direct answer to the capture-
  semantics question closures were originally deferred over: "by value
  or by reference?" is moot here, because every `let` binding in
  NucleScript is single-assignment — there is no `let mut`, no
  reassignment syntax anywhere in the grammar — so there is no "later
  mutation" a by-value/by-reference distinction could ever observe.
  Capturing a `Pool<...>`/`Strand`/`Sequence` binding is likewise
  harmless-but-inert: those have no runtime representation at all (they
  stay purely compile-time-inferred), the same as passing one to an
  ordinary function parameter already is.
- **Calling a closure reuses the existing `name(args)` syntax** —
  whether `name` is a top-level `fn` or a local closure/`Fn(...)`-typed
  parameter, the call site looks identical; typeck/the runtime both
  resolve closures *before* falling back to the global function table.
- **Generic closures, `fn<T>(...)`** — a closure literal can
  declare its own type-parameter list, exactly mirroring a named
  function's `fn name<T>(...)`. Since a fresh type-parameter name is only
  recognized inside a closure literal's *own* declared `<...>` list (not
  invented from thin air), a generic closure is only expressible nested
  inside an already-generic enclosing function/closure that shares the
  same type-parameter name — the closure's own call-site unification
  (`check_closure_call_args`) works exactly like a named generic
  function's:
  ```nsl
  fn recover_via_closure<P>(source: Pool<P, 0.35%>) returns Pool<Recovered> {
      let recover: Fn(Pool<P, 0.35%>) -> Pool<Recovered> = fn<P>(inner: Pool<P, 0.35%>) -> Pool<Recovered> {
          let recovered: Pool<Recovered> = consensus_vote(inner, coverage: 10x)
      }
      let recovered: Pool<Recovered> = recover(source)
  }
  ```
- **Self-recursion.** A `let`-bound closure can now call itself
  by its own bound name — resolved at runtime by `call_closure`
  re-inserting the closure under the name it was just invoked through
  before running its own body, since the closure's own `captured_env`
  snapshot (taken *before* its enclosing `let` finishes binding) never
  contains itself on its own. **Mutual recursion between two
  independently-`let`-bound closures is still impossible** — `let` is
  single-assignment and forward-reference-only capture (B must exist
  before A can capture it, and A must exist before B can capture it
  back), and there's no forward-declaration syntax to break that cycle.
  A self-recursive closure that always retries the exact same failing
  operation with no changing state recurses forever, exactly like a
  self-recursive named function with no base case would — the recursive
  call needs to do something different each time (retry a different
  target, in the example below) to actually terminate.
- **`nucle plan`/`nucle explain` narration through a `let`-bound
  closure's own call** — the narrator now walks into a
  `let`-bound closure's real body the same way it already walks a named
  function's, checked with the same priority effect analysis
  established (closures before named functions). **A real, honest
  limitation remains**: a closure received as a `Fn(...)`-typed
  *parameter*, or an inline closure literal passed directly as a call
  argument, is still unnarratable — neither's real body is known at that
  call site, only at runtime. The same applies to effect analysis for a
  `Fn(...)`-typed *parameter*'s call specifically (its real body isn't
  knowable until runtime, so it's optimistically treated as `Pure`) — but
  a *`let`-bound* closure's real effect *is* resolved correctly at its
  call site, since its actual body is right there in the source.

See [docs/errors.md](errors.md) for `E-CLOSURE-RETURN-TYPE-MISMATCH`, and
[`docs/examples/closure_retry.nsl`](examples/closure_retry.nsl) for a
complete, runnable example.

---

## Testing (`test` / `assert`)

```nsl
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }

test "consensus voting reduces the inferred error rate" {
    let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
    let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
    assert recovered < noisy, "consensus_vote should reduce the error budget"
}
```

`nucle test` runs every `test { ... }` block against a fresh, isolated VFS
instance per test (real `store`/`retrieve`/`delete` operations inside a
test body execute for real). `assert`'s condition uses the exact same
`Condition` grammar an `if` does, evaluated the same way -- at compile
time, not deferred to a "runtime" phase, since NucleScript's probabilistic
properties are deterministic formulas computed at compile time already,
not something measured empirically. A test fails if any assertion inside
it evaluates false, or if a real VFS operation inside it errors at
execution time; a test whose body has a genuine compile error (anything
other than a failed assertion) aborts the whole run and is reported the
same way `nucle check` would report it, not folded into that one test's
result. See [docs/errors.md](errors.md) for `E-ASSERTION-FAILED` and the
shared `E-CONDITION-*` codes.

---

## Documentation (`///`)

```nsl
/// Archives a file with the given recovery guarantee.
fn archive(data: File, target: Pool<Illumina>, guarantee: Recovery) returns DnaFile {
    let plan: DnaFile = protect data for guarantee
    store plan into target
}
```

A `///` line immediately preceding a `pool`/`strand`/`seq`/`fn`/`pipeline`
declaration attaches as that declaration's documentation (consecutive
`///` lines join into one `\n`-separated string); `nucle doc` renders
every declaration of those five kinds to Markdown, with or without a
`///` comment -- undocumented ones just get a signature and effect, no
description paragraph. A `///` comment immediately before anything else
(a `let`, an operation, an `if`/`for`/`test`) is a parse error naming the
offending keyword, not a silent no-op -- there's no field on those
declarations to attach it to, so writing one there is always a mistake
worth surfacing rather than documentation that quietly went nowhere. See
[docs/architecture.md](architecture.md) for how `nucle doc` is
implemented (`docgen.rs`).

---

## Formatting Conventions

1. **Comments**: Start with `//` and extend to the end of the line. A
   `///` comment (see "Documentation" above) is a distinct doc comment,
   not a regular comment with an extra slash.
2. **Trailing Commas**: Allowed and encouraged in parameter lists, import lists, option sets, and query lists.
3. **Identifiers**: Case-sensitive for symbols, but keywords and codec/profile options are parsed case-insensitively.
4. **Multiplier/Percent Suffixes**: Suffixes `x`/`X` and `%` must immediately follow the numeric value without whitespace (e.g. `3x`, `0.35%`).
