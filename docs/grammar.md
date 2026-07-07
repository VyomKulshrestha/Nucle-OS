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
                      | Identifier '(' ExprList? ')'
                      | Identifier
                      | StringLiteral
                      | NumberLiteral
                      | '(' Expr ')'
                      | Expr ( '==' | '!=' | '<' | '>' | '<=' | '>=' ) Expr
                      | Expr '&&' Expr
                      | Expr '||' Expr
                      | '!' Expr
ExprList            ::= Expr ( ',' Expr )* ','?
                      // The boolean/comparison operators above bind exactly
                      // as in `Condition`: '||' loosest, then '&&', then
                      // unary '!', then a single non-chaining comparison,
                      // then a primary expression. There is no arithmetic
                      // ('+'/'-'/'*'/'/') -- literal numbers and a pool
                      // binding's inferred error rate are only ever compared,
                      // never combined.

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

NucleScript has no runtime: a program compiles to a static plan (a fixed
list of pool schemas, probabilistic bindings, and store/retrieve/delete
calls) which is then executed as-is. `if` and `for` fit into that model as
**compile-time** constructs, not true runtime branching or looping:

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
