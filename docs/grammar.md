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

FunctionDecl        ::= 'fn' Identifier '(' FnParamList? ')' ( '->' | 'returns' ) TypeExpr '{' Declaration* '}'
                      // the return type is mandatory — a function with no
                      // meaningful return value still writes `returns Void`
                      // rather than omitting it; the parser rejects a
                      // missing '->'/'returns' clause instead of defaulting
FnParamList         ::= FnParam ( ',' FnParam )* ','?
FnParam             ::= Identifier ':' TypeExpr

TypeExpr            ::= 'Pool' '<' PoolState ( ',' PercentLiteral )? '>'
                      | 'Strand' | 'Sequence' | 'File' | 'DnaFile' | 'Recovery' | 'Void'
                      | 'Result' '<' TypeExpr ',' TypeExpr '>'
                      | 'Str'
                      // 'Result<T, E>' is the one generic type NucleScript
                      // has -- no general 'Type<...>' mechanism exists;
                      // 'Pool<Illumina>' above is its own hardcoded parse
                      // path, unrelated to this. 'Str' is meaningful only
                      // as 'Result<_, Str>''s error slot: every VFS
                      // failure is a plain message string, and there is no
                      // string arithmetic or any other place 'Str' is
                      // expected. See "Result / Error Propagation" below.
PoolState           ::= 'Illumina' | 'Nanopore' | 'Twist' | 'Amplified' | 'Recovered'

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
- **No `match`/`if let` on `Result`** -- a caught `Err` can only be
  produced and propagated, not branched on from within the same program.
  Building on a caught error (retrying a different pool, logging why)
  needs a second, independent function call from the caller, not
  in-language conditional logic. This is a deliberate scope boundary, not
  an oversight: NucleScript still has no pattern matching or general
  boolean branching over runtime values, only over the same compile-time
  `Condition` grammar `if`/`assert` already use.

See [docs/errors.md](errors.md) for the six new `E-TRY-*`/`E-BINDING-
RESULT-*`/`E-RETURN-TYPE-*` codes, and
[`docs/examples/result_fallback_store.nsl`](examples/result_fallback_store.nsl)
for a complete, runnable example.

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
