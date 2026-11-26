import Checker

/-!
# `nsl-check`: the Lean checker as a program

Reads one core program per line from standard input and prints, for each,
`accept <effect>` with the worst effect the rules report for it, or
`reject`. `nucle_lang/tests/lean_differential.rs` writes the programs and
compares these verdicts with the Rust checker's.

A line is a list of functions followed by the program's statement, in
prefix form:

    fn <name> <n> <param> <ceiling> ... <stmt>   (repeated)
    main <stmt>

    <ceiling> ::= none | hw | syn | sqn | key
    <stmt>    ::= op <store | sim | get | del1 | del0 | syn1 | syn0 | sqn1 | sqn0>
                | seq <stmt> <stmt> | alt <stmt> <stmt>
                | clo <x> <ceiling> <n> <param> ... <stmt> <stmt> | alias <x> <y> <stmt>
                | opq <x> <ceiling> <n> <param> ... <stmt> <stmt>
                | call <x> <n> <arg> ... | callf <f> <n> <arg> ...
                | loop <stmt> | stop

`del0` is a delete without its confirmation, `syn1`/`sqn1` a synthesis or
sequencing with one, and so on.
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

partial def parseStmt : List String → Option (Stmt × List String)
  | "op" :: k :: rest => (parseOp k).map fun o => (.op o, rest)
  | "seq" :: rest => do
    let (a, r₁) ← parseStmt rest
    let (b, r₂) ← parseStmt r₁
    pure (.seq a b, r₂)
  | "alt" :: rest => do
    let (a, r₁) ← parseStmt rest
    let (b, r₂) ← parseStmt r₁
    pure (.choice a b, r₂)
  | "clo" :: x :: c :: n :: rest => do
    let k ← parseCeiling c
    let (ps, r₀) ← takeN (← n.toNat?) rest
    let (body, r₁) ← parseStmt r₀
    let (s, r₂) ← parseStmt r₁
    pure (.letClosure x k ps body s, r₂)
  | "alias" :: x :: y :: rest => do
    let (s, r) ← parseStmt rest
    pure (.letAlias x y s, r)
  | "opq" :: x :: c :: n :: rest => do
    let k ← parseCeiling c
    let (ps, r₀) ← takeN (← n.toNat?) rest
    let (body, r₁) ← parseStmt r₀
    let (s, r₂) ← parseStmt r₁
    pure (.letOpaque x k ps body s, r₂)
  | "call" :: x :: n :: rest => do
    let (args, r) ← takeN (← n.toNat?) rest
    pure (.callClosure x args, r)
  | "stop" :: rest => some (.stop, rest)
  | "loop" :: rest => do
    let (body, r) ← parseStmt rest
    pure (.loop body, r)
  | "callf" :: f :: n :: rest => do
    let (args, r) ← takeN (← n.toNat?) rest
    pure (.callFun f args, r)
  | _ => none

def parseParams : Nat → List String → Option (List (Name × Option Ceiling) × List String)
  | 0, rest => some ([], rest)
  | n + 1, p :: c :: rest => do
    let k ← parseCeiling c
    let (ps, r) ← parseParams n rest
    pure ((p, k) :: ps, r)
  | _ + 1, _ => none

partial def parseProgram (fs : List (Name × FunDecl)) : List String → Option (List (Name × FunDecl) × Stmt)
  | "fn" :: f :: n :: rest => do
    let (params, r₁) ← parseParams (← n.toNat?) rest
    let (body, r₂) ← parseStmt r₁
    parseProgram (fs ++ [(f, { params, body })]) r₂
  | "main" :: rest => do
    let (s, r) ← parseStmt rest
    if r.isEmpty then pure (fs, s) else none
  | _ => none

def worst (R : BEffs) : String :=
  if R.des then "Destructive" else if R.syn then "Synthesis" else if R.seq then "Sequencing" else "Pure"

def verdict (fs : List (Name × FunDecl)) (s : Stmt) : String :=
  let P : Program := fun f => (fs.find? (·.1 == f)).map (·.2)
  let names := fs.map (·.1)
  let Φ := signature P names
  if programOkB P Φ names && programConfirmedB P names && confirmedB s then
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
    match parseProgram [] tokens with
    | some (fs, s) => stdout.putStrLn (verdict fs s)
    | none => stdout.putStrLn "unparsed"
  loop stdin stdout

def main : IO Unit := do
  loop (← IO.getStdin) (← IO.getStdout)
