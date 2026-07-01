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

Expr                ::= 'simulate' Identifier 'under' ProfileLiteral
                      | ( 'synthesise' | 'synthesize' ) Identifier 'via' ProfileLiteral ( 'confirm' 'hardware' )?
                      | 'sequence' Identifier 'via' ProfileLiteral ( 'confirm' 'hardware' )?
                      | 'consensus_vote' '(' Identifier ',' 'coverage' ':' MultiplierLiteral ')'
                      | 'protect' Identifier 'for' Identifier
                      | Identifier '(' ExprList? ')'
                      | Identifier
                      | StringLiteral
ExprList            ::= Expr ( ',' Expr )* ','?

Operation           ::= StoreOp
                      | RetrieveOp
                      | DeleteOp

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

## Formatting Conventions

1. **Comments**: Start with `//` and extend to the end of the line.
2. **Trailing Commas**: Allowed and encouraged in parameter lists, import lists, option sets, and query lists.
3. **Identifiers**: Case-sensitive for symbols, but keywords and codec/profile options are parsed case-insensitively.
4. **Multiplier/Percent Suffixes**: Suffixes `x`/`X` and `%` must immediately follow the numeric value without whitespace (e.g. `3x`, `0.35%`).
