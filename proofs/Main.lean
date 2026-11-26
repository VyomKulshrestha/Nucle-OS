import Checker

/-!
# `nsl-check`: the Lean checker as a program

Reads one core program per line from standard input and prints, for each,
`accept <effect>` with the worst effect the rules report for it, or
`reject`. `nucle_lang/tests/lean_differential.rs` writes the programs and
compares these verdicts with the Rust checker's.

A line is a list of enums and functions followed by the program's
statement, in prefix form:

    enum <name> <n> <tag> <base> ...                   (repeated)
    fn <name> <n> <param> <ceiling> <ty> ... <stmt>    (repeated)
    main <stmt>

    <ceiling> ::= none | hw | syn | sqn | key
    <ty>      ::= fn <m> <ceiling> ... | pool <profile> | poolv <param>
                | base <name> | res <ty> <ty> | enum <name>
    <stmt>    ::= op <store | sim | get | del1 | del0 | syn1 | syn0 | sqn1 | sqn0>
                | seq <stmt> <stmt> | alt <stmt> <stmt>
                | clo <x> <ceiling> <n> <param> <ceiling> ... <stmt> <stmt>
                | opq <x> <ceiling> <n> <param> <ceiling> ... <stmt> <stmt>
                | alias <x> <y> <stmt>
                | call <x> <n> <arg> ... <m> <ceiling> ...
                | callf <f> <n> <arg> ... <k> <param> <prof <p> | var <q>> ...
                | val <x> <pool <p> | out <base> | var <enum> <tag> <base>> <stmt>
                | try <x> <ty> <y> <stmt> | mt <y> <tag> <z> <stmt> <stmt> | nomatch <y>
                | loop <stmt> | stop

A `fn <m> <ceiling> ...` type is a closure's: the ceilings of the closures
it takes. In a call, the ceilings after the arguments are the called
closure's type, and a function call ends with the profiles the checker
gave the function's type parameters. `del0` is a delete without its
confirmation, `syn1`/`sqn1` a synthesis or sequencing with one, and so on.
-/

open NucleScript

def parseCeiling : String → Option (Option Ceiling)
  | "none" => some none
  | "hw" => some (some .hardware)
  | "syn" => some (some .synthesis)
  | "sqn" => some (some .sequencing)
  | "key" => some (some .physicalKey)
  | _ => none

def parseOp : String → Option Op
  | "store" => some .store
  | "sim" => some .simulateStore
  | "get" => some .retrieve
  | "del1" => some (.delete true)
  | "del0" => some (.delete false)
  | "syn1" => some (.synthesise true)
  | "syn0" => some (.synthesise false)
  | "sqn1" => some (.sequence true)
  | "sqn0" => some (.sequence false)
  | _ => none

def takeN : Nat → List String → Option (List String × List String)
  | 0, rest => some ([], rest)
  | n + 1, a :: rest => (takeN n rest).map fun (xs, r) => (a :: xs, r)
  | _ + 1, [] => none

def parseCeilings : Nat → List String → Option (List (Option Ceiling) × List String)
  | 0, rest => some ([], rest)
  | n + 1, c :: rest => do
    let k ← parseCeiling c
    let (ks, r) ← parseCeilings n rest
    pure (k :: ks, r)
  | _ + 1, [] => none

def parseOwn : Nat → List String → Option (List (Name × Option Ceiling) × List String)
  | 0, rest => some ([], rest)
  | n + 1, p :: c :: rest => do
    let k ← parseCeiling c
    let (ps, r) ← parseOwn n rest
    pure ((p, k) :: ps, r)
  | _ + 1, _ => none

partial def parseTy (En : Enums) : List String → Option (Ty × List String)
  | "fn" :: m :: rest => do
    let (ks, r) ← parseCeilings (← m.toNat?) rest
    pure (.fn ks, r)
  | "pool" :: p :: rest => some (.pool (.prof p), rest)
  | "poolv" :: v :: rest => some (.pool (.var v), rest)
  | "base" :: b :: rest => some (.base b, rest)
  | "res" :: rest => do
    let (t, r₁) ← parseTy En rest
    let (e, r₂) ← parseTy En r₁
    pure (Ty.res t e, r₂)
  | "enum" :: e :: rest => do
    let vs ← En e
    pure (.enum e (vs.map Prod.fst), rest)
  | _ => none

def parseInst : Nat → List String → Option (List (Name × PTy) × List String)
  | 0, rest => some ([], rest)
  | n + 1, v :: "prof" :: p :: rest => do
    let (is, r) ← parseInst n rest
    pure ((v, .prof p) :: is, r)
  | n + 1, v :: "var" :: q :: rest => do
    let (is, r) ← parseInst n rest
    pure ((v, .var q) :: is, r)
  | _ + 1, _ => none

partial def parseStmt (En : Enums) : List String → Option (Stmt × List String)
  | "op" :: k :: rest => (parseOp k).map fun o => (.op o, rest)
  | "seq" :: rest => do
    let (a, r₁) ← parseStmt En rest
    let (b, r₂) ← parseStmt En r₁
    pure (.seq a b, r₂)
  | "alt" :: rest => do
    let (a, r₁) ← parseStmt En rest
    let (b, r₂) ← parseStmt En r₁
    pure (.choice a b, r₂)
  | "clo" :: x :: c :: n :: rest => do
    let k ← parseCeiling c
    let (ps, r₀) ← parseOwn (← n.toNat?) rest
    let (body, r₁) ← parseStmt En r₀
    let (s, r₂) ← parseStmt En r₁
    pure (.letClosure x k ps body s, r₂)
  | "alias" :: x :: y :: rest => do
    let (s, r) ← parseStmt En rest
    pure (.letAlias x y s, r)
  | "opq" :: x :: c :: n :: rest => do
    let k ← parseCeiling c
    let (ps, r₀) ← parseOwn (← n.toNat?) rest
    let (body, r₁) ← parseStmt En r₀
    let (s, r₂) ← parseStmt En r₁
    pure (.letOpaque x k ps body s, r₂)
  | "call" :: x :: n :: rest => do
    let (args, r₀) ← takeN (← n.toNat?) rest
    match r₀ with
    | m :: r₁ =>
      let (pks, r) ← parseCeilings (← m.toNat?) r₁
      pure (.callClosure x args pks, r)
    | [] => none
  | "callf" :: f :: n :: rest => do
    let (args, r₀) ← takeN (← n.toNat?) rest
    match r₀ with
    | k :: r₁ =>
      let (inst, r) ← parseInst (← k.toNat?) r₁
      pure (.callFun f args inst, r)
    | [] => none
  | "val" :: x :: "pool" :: p :: rest => do
    let (s, r) ← parseStmt En rest
    pure (.letVal x (.pool p) s, r)
  | "val" :: x :: "out" :: t :: rest => do
    let (s, r) ← parseStmt En rest
    pure (.letVal x (.outcome t) s, r)
  | "val" :: x :: "var" :: e :: tag :: b :: rest => do
    let (s, r) ← parseStmt En rest
    pure (.letVal x (.variant e tag b) s, r)
  | "try" :: x :: rest => do
    let (t, r₀) ← parseTy En rest
    match r₀ with
    | y :: r₁ =>
      let (s, r) ← parseStmt En r₁
      pure (.tryOk x t y s, r)
    | [] => none
  | "mt" :: y :: tag :: z :: rest => do
    let (a, r₁) ← parseStmt En rest
    let (b, r₂) ← parseStmt En r₁
    pure (.matchTag y tag z a b, r₂)
  | "nomatch" :: y :: rest => some (.noMatch y, rest)
  | "stop" :: rest => some (.stop, rest)
  | "loop" :: rest => do
    let (body, r) ← parseStmt En rest
    pure (.loop body, r)
  | _ => none

partial def parseParams (En : Enums) : Nat → List String →
    Option (List (Name × Option Ceiling) × List Ty × List String)
  | 0, rest => some ([], [], rest)
  | n + 1, p :: c :: rest => do
    let k ← parseCeiling c
    let (t, r₀) ← parseTy En rest
    let (ps, ts, r) ← parseParams En n r₀
    pure ((p, k) :: ps, t :: ts, r)
  | _ + 1, _ => none

def parseVariants : Nat → List String → Option (List (Name × Name) × List String)
  | 0, rest => some ([], rest)
  | n + 1, tag :: b :: rest => do
    let (vs, r) ← parseVariants n rest
    pure ((tag, b) :: vs, r)
  | _ + 1, _ => none

def enumsOf (es : List (Name × List (Name × Name))) : Enums := fun e => lookup es e

partial def parseProgram (es : List (Name × List (Name × Name))) (fs : List (Name × FunDecl)) :
    List String → Option (List (Name × List (Name × Name)) × List (Name × FunDecl) × Stmt)
  | "enum" :: e :: n :: rest => do
    let (vs, r) ← parseVariants (← n.toNat?) rest
    parseProgram (es ++ [(e, vs)]) fs r
  | "fn" :: f :: n :: rest => do
    let (params, ptypes, r₁) ← parseParams (enumsOf es) (← n.toNat?) rest
    let (body, r₂) ← parseStmt (enumsOf es) r₁
    parseProgram es (fs ++ [(f, { params, body, ptypes })]) r₂
  | "main" :: rest => do
    let (s, r) ← parseStmt (enumsOf es) rest
    if r.isEmpty then pure (es, fs, s) else none
  | _ => none

def worst (R : BEffs) : String :=
  if R.des then "Destructive" else if R.syn then "Synthesis" else if R.seq then "Sequencing" else "Pure"

def verdict (es : List (Name × List (Name × Name))) (fs : List (Name × FunDecl)) (s : Stmt) : String :=
  let P : Program := fun f => (fs.find? (·.1 == f)).map (·.2)
  let En := enumsOf es
  let names := fs.map (·.1)
  let Φ := signature P names
  if programOkB P Φ names && programConfirmedB P names && confirmedB s
      && programShapedB P En names && shapedB P En [] s then
    match check P Φ [] s with
    | some (R, _) => "accept " ++ worst R
    | none => "reject"
  else
    "reject"

partial def loop (stdin : IO.FS.Stream) (stdout : IO.FS.Stream) : IO Unit := do
  let line ← stdin.getLine
  if line.isEmpty then
    return
  let tokens := ((line.replace "\r" "").replace "\n" "" |>.splitOn " ").filter (· ≠ "")
  if !tokens.isEmpty then
    match parseProgram [] [] tokens with
    | some (es, fs, s) => stdout.putStrLn (verdict es fs s)
    | none => stdout.putStrLn "unparsed"
  loop stdin stdout

def main : IO Unit := do
  loop (← IO.getStdin) (← IO.getStdout)
