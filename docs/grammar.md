# NucleScript Grammar Reference

This document defines the formal syntax and grammar of NucleScript (`.nsl`), the domain-specific language of NucleOS.

---

## EBNF Grammar

```ebnf
Program             ::= ( Declaration | ',' )*

Declaration         ::= ImportDecl
                      | PoolDecl
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
                      // annotation or a function parameter's type. See
                      // "Result / Error Propagation", "Generics", and
                      // "Closures" below.
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
                      // (as opposed to only under 'Operation' below) is
                      // Step 9's one new capability: the exact same
                      // grammar 'store'/'retrieve'/'delete' already have
                      // as *statements*, now also usable in *expression*
                      // position (e.g. the right-hand side of a 'let') --
                      // one struct, two surface positions, per "Result /
                      // Error Propagation" below. The statement forms
                      // under 'Operation' are unaffected: the parser only
                      // ever produces this expression form after 'let x =',
                      // never at the top of a declaration.

MatchExpr           ::= 'match' Expr '{' 'Ok' '(' Identifier ')' '=>' Expr ','
                                        'Err' '(' Identifier ')' '=>' Expr ','? '}'
                      // Destructures a Result<T, E>-shaped Expr. Arm order
                      // is fixed ('Ok' then 'Err') -- Result is a closed,
                      // two-variant type with no general/reorderable arm
                      // machinery. See "Pattern Matching" below.

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

Before Step 9 (`Result<T, E>`/`?`, below), NucleScript had no runtime at
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
in this language. Before Step 9, every `store`/`retrieve`/`delete`
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
- **`match` (Step 11) now lets a caught `Err` be inspected directly** --
  see "Pattern Matching" below. Before that, a caught `Err` could only be
  produced and propagated, never branched on from within the same
  program; building on it (retrying a different pool, logging why) needed
  a second, independent function call from the caller.

See [docs/errors.md](errors.md) for the six new `E-TRY-*`/`E-BINDING-
RESULT-*`/`E-RETURN-TYPE-*` codes, and
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
  `Result`/anything else. There's no explicit type-argument syntax
  (`foo::<Illumina>()`); a type parameter that no argument binds is a
  real error (`E-TYPE-PARAM-UNRESOLVED`), not silently left generic.
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
UNRESOLVED`, and
[`docs/examples/generic_pool_recovery.nsl`](examples/generic_pool_recovery.nsl)
for a complete, runnable example.

---

## Pattern Matching (`match` / `Ok` / `Err`)

The gap Step 9 itself named as "not implemented": a caught `Err` could
only be inspected by a second, independent function call from the
caller, never branched on from within the same function. `match`
destructures a `Result<T, E>`-shaped expression directly, binding each
arm's payload to a name visible only within that arm:

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

- **`Result` is the only sum type in the language, and it's closed to
  exactly two variants** -- so `match` needs no general pattern-matching
  engine or exhaustiveness algorithm, just a fixed two-arm form. Arm
  order is fixed (`Ok` always first, `Err` always second, with an
  optional trailing comma) -- reordering them is a parse error, not a
  semantic one. A real multi-arm/reorderable `match` only becomes worth
  building once user-defined enums exist (still deferred).
- **Both arms must unify to one type**, which becomes the whole `match`
  expression's type -- usable directly as a `let`'s RHS, exactly like a
  function call or any other expression. If both arms happen to still be
  `Result`-shaped (neither used `?`), the match's own value is itself a
  still-wrapped `Result` a caller can `?`/re-match later.
- **No `Ok(...)`/`Err(...)` *constructor* syntax.** This feature is about
  destructuring an existing `Result`, not building new ones -- a
  function still only ever produces a `Result` the way it already did
  before `match` existed (an unwrapped `?`-tail auto-wraps at the call
  boundary, or a still-wrapped `store`/`delete`/`Result`-returning call
  flows through as-is). An arm's body is therefore one of exactly four
  shapes: the pattern's own bound name (`Ok(file) => file`), `?` applied
  to a fallible expression (checked against the *enclosing function's*
  return type, exactly like `?` anywhere else), a still-wrapped
  `Result`-shaped expression, or a `Pool<...>`-shaped expression.
- **No composability with `?`, nested `match`, or function-call
  arguments.** A `match`'s scrutinee must be one of the shapes the
  checker already recognizes as `Result`-shaped (a variable bound to one,
  a `store`/`delete` expression, or a call to a `Result`-returning
  function) -- `match (match a {...}) {...}` and `(match a {...})?`
  aren't supported. The core case (`match` directly over a `let`-bound
  `Result`) is what actually closes Step 9's gap; composability is a
  real but narrow follow-on gap, named here rather than silently broken.
- **Arm bodies are a single expression, not a block** -- matching the
  language's existing convention that there's no bare-block-as-expression
  anywhere (`?` is the model: it wraps exactly one inner expression). An
  arm that needs a fallback operation writes it directly as its one
  expression, as `(store ... into ...)?` does above; it doesn't get its
  own local `let` sequence the way a function body does.
- **Effect analysis joins the scrutinee and both arms unconditionally**,
  the same conservative "every declaration in this join counts" rule
  `if`/`?` already follow -- a `Destructive` operation in only the `Err`
  arm still requires confirmation, since this analysis has never modeled
  "this branch might not run."

See [docs/errors.md](errors.md) for `E-MATCH-NOT-RESULT`/`E-MATCH-ARM-
TYPE-MISMATCH`/`E-MATCH-ARM-UNTYPABLE`, and
[`docs/examples/match_result_fallback.nsl`](examples/match_result_fallback.nsl)
for a complete, runnable example.

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
- **No generic closures** (`fn<T>(...)`) — a closure's signature is
  always fixed and concrete.
- **No self-recursion.** A closure literal has no name to reference
  inside its own body — it can call an *earlier*-defined closure/
  function but never itself, and two distinct closures can never be
  mutually recursive either (each only ever sees what was already bound
  *before* its own literal). There is no cycle here to guard against.
- **A real, honest limitation: `nucle plan`/`nucle explain` can't see
  through an indirect call.** Both resolve a callee by static name; a
  closure held in a variable has no name to look up, so a closure call
  is invisible to them — only `nucle run`'s real execution reflects it.
  The same applies to effect analysis for a `Fn(...)`-typed
  *parameter*'s call specifically (its real body isn't knowable until
  runtime, so it's optimistically treated as `Pure`) — but a *`let`-
  bound* closure's real effect *is* resolved correctly at its call site,
  since its actual body is right there in the source.

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
