/-!
# NucleScript's effect rules, proved for a core of the language

A core of NucleScript with the parts its effect rules are about:
operations with effects and confirmations, sequencing, branches (an `if` or
a `match`: either side may run), closures bound by `let` with or without an
effect ceiling, other names for closures, closures the checker cannot see
into (returned by a function, taken out of an enum), calls to closures,
named functions taking closure arguments, with or without ceilings, loops
over a pool's files (zero or more passes), and an early return through `?`
(`stop`), which ends the function or closure it is in. Closures take
closures as arguments of their own, with or without ceilings, and a call
passes them. Values that are not closures come in too: declared pools,
passed to functions concrete or generic over their profile, the `Result`s
operations give back, unwrapped by `?`, and enum variants, taken apart by
`match`. They add no effects of their own; the fourth theorem is about
their shapes.

The static rules give a statement two things: `R`, the effects it may have
itself, and `U`, the parameters without a ceiling whose closures it may run.
`nucle explain` reports `R` and leaves `U` to the call sites, which add the
closures they pass; an effect ceiling must also cover `U`, so it holds only
when `U` is empty and `R` is within it. These are the rules of
`effects.rs` and `typeck.rs`, the second through `effects::ceiling_view`.

The run of a statement is a list of events: each operation's effect, whether
its confirmation was written, and the ceilings of the closure calls in
progress. Three theorems, for a statement the rules accept in a program
whose functions they accept:

* `program_effects_sound`: every event's effect is in `R`;
* `program_ceilings_sound`: every event is allowed by every ceiling in force;
* `program_confirmed`: every event carried its confirmation.

A fourth, `program_shapes_sound`, says a shaped program never reaches a
call it cannot make, a value that breaks its declared type, or a `?` or
`match` meeting a value it cannot take.
-/

namespace NucleScript

/-! ## Effects and ceilings -/

inductive Eff where
  | synthesis
  | sequencing
  | destructive
  deriving DecidableEq, Repr

/-- A set of effects. -/
def Effs : Type := Eff → Prop

namespace Effs

def none : Effs := fun _ => False

def all : Effs := fun _ => True

def union (a b : Effs) : Effs := fun e => a e ∨ b e

/-- Every effect in `a` is in `b`. -/
def Sub (a b : Effs) : Prop := ∀ e, a e → b e

theorem Sub.refl (a : Effs) : Sub a a := fun _ h => h

theorem Sub.trans {a b c : Effs} (h₁ : Sub a b) (h₂ : Sub b c) : Sub a c :=
  fun e h => h₂ e (h₁ e h)

theorem sub_union_left (a b : Effs) : Sub a (union a b) := fun _ h => Or.inl h

theorem sub_union_right (a b : Effs) : Sub b (union a b) := fun _ h => Or.inr h

theorem union_sub {a b c : Effs} (h₁ : Sub a c) (h₂ : Sub b c) : Sub (union a b) c :=
  fun e h => h.elim (h₁ e) (h₂ e)

end Effs

/-- `confirm hardware` covers synthesis and sequencing, `confirm
synthesis` and `confirm sequencing` one of them each, and `confirm
physical_key` covers deletion. -/
inductive Ceiling where
  | hardware
  | synthesis
  | sequencing
  | physicalKey
  deriving DecidableEq, Repr

def Ceiling.allows : Ceiling → Effs
  | .hardware => fun e => e = .synthesis ∨ e = .sequencing
  | .synthesis => fun e => e = .synthesis
  | .sequencing => fun e => e = .sequencing
  | .physicalKey => fun e => e = .destructive

/-! ## Syntax -/

/-- Operations. `confirmed` is whether the source wrote the confirmation the
operation needs; a store needs none. -/
inductive Op where
  | store
  | simulateStore
  | retrieve
  | delete (confirmed : Bool)
  | synthesise (confirmed : Bool)
  | sequence (confirmed : Bool)

def Op.effect : Op → Option Eff
  | .store => some .synthesis
  | .simulateStore => none
  | .retrieve => none
  | .delete _ => some .destructive
  | .synthesise _ => some .synthesis
  | .sequence _ => some .sequencing

def Op.confirmed : Op → Bool
  | .delete c => c
  | .synthesise c => c
  | .sequence c => c
  | _ => true

def Op.effs (o : Op) : Effs := fun e => o.effect = some e

abbrev Name := String

/-! ## Types -/

/-- A pool's profile, or a type parameter of a generic function standing
for one. -/
inductive PTy where
  | prof (p : Name)
  | var (v : Name)
  deriving DecidableEq, Repr

/-- A value's type, as far as its shape goes. A closure's lists the
ceilings of the closures it takes. A `Result`'s and an enum's list the
variants the value may still be: all of them as declared, fewer in the
arms of a `match` after those that did not match. -/
inductive Ty where
  | fn (pks : List (Option Ceiling))
  | pool (p : PTy)
  /-- `Str`, `DnaFile`, `Void`: values with nothing inside. -/
  | base (b : Name)
  | result (t e : Ty) (rem : List Name)
  | enum (e : Name) (rem : List Name)
  deriving DecidableEq, Repr

/-- `Result<t, e>` as written. -/
def Ty.res (t e : Ty) : Ty := .result t e ["Ok", "Err"]

def Ty.isFn : Ty → Bool
  | .fn _ => true
  | _ => false

/-- A value a `let` binds that is not a closure. -/
inductive VExpr where
  /-- A declared pool of profile `p`. -/
  | pool (p : Name)
  /-- What a `store` or `delete` gives back, a `Result<t, Str>`; the
  operation itself is the statement before. -/
  | outcome (t : Name)
  /-- `E::tag`, carrying a value of base type `b` (`Void` for none). -/
  | variant (e tag b : Name)

inductive Stmt where
  | op (o : Op)
  | seq (s₁ s₂ : Stmt)
  /-- An `if` or `match`: either side may run. -/
  | choice (s₁ s₂ : Stmt)
  /-- `let x: Fn(...) -> Void [confirm k] = fn(p₁: Fn() -> Void [confirm
  k₁], ...) -> Void { body }`, then `rest`: a closure whose own parameters
  `ps` take closures, each with or without a ceiling. -/
  | letClosure (x : Name) (k : Option Ceiling) (ps : List (Name × Option Ceiling)) (body rest : Stmt)
  /-- `let x: Fn(...) -> Void = y`, then `rest`. -/
  | letAlias (x y : Name) (rest : Stmt)
  /-- A closure the checker cannot see into, such as one returned by a
  function: `body` is what it does, which the rules do not look at. -/
  | letOpaque (x : Name) (k : Option Ceiling) (ps : List (Name × Option Ceiling)) (body rest : Stmt)
  /-- `x(a₁, ...)`: a call to a closure, passing it closures. `pks` are the
  ceilings of the closure's parameters as its type gives them; a run checks
  that the closure it finds has them. -/
  | callClosure (x : Name) (args : List Name) (pks : List (Option Ceiling))
  /-- `f(a₁, ...)`: `inst` are the profiles the checker inferred for the
  function's type parameters, if it has any. -/
  | callFun (f : Name) (args : List Name) (inst : List (Name × PTy))
  /-- `for f in files of pool { body }`: the body runs zero or more times. -/
  | loop (body : Stmt)
  /-- A `?` meeting an `Err`: the enclosing function or closure returns
  here, and nothing after it in that function or closure runs. -/
  | stop
  /-- `let x: T = v`, then `rest`, for a value that is not a closure. -/
  | letVal (x : Name) (v : VExpr) (rest : Stmt)
  /-- `let x: t = y?`, then `rest`: an `Err` in `y` ends the function here. -/
  | tryOk (x : Name) (t : Ty) (y : Name) (rest : Stmt)
  /-- An arm of a `match` on `y`, `tag(z) => s₁`, and the arms after it,
  `s₂`, which run if `y` is another variant. A wildcard arm is any other
  statement. -/
  | matchTag (y tag z : Name) (s₁ s₂ : Stmt)
  /-- The end of a `match` without a wildcard arm: no arm matched. -/
  | noMatch (y : Name)

/-- A named function: its parameters, a closure parameter with or without
a ceiling, and its body. Named functions capture nothing. -/
structure FunDecl where
  params : List (Name × Option Ceiling)
  body : Stmt
  /-- Each parameter's type, in order, a closure taking nothing where the
  list stops; a pool's may name a type parameter. Only the value-shape
  theorem reads it. -/
  ptypes : List Ty := []

abbrev Program := Name → Option FunDecl

/-- Parameters with their types, a closure taking nothing where the types
stop. -/
def paramShapes : List (Name × Option Ceiling) → List Ty → List (Name × Ty)
  | [], _ => []
  | (p, _) :: ps, [] => (p, .fn []) :: paramShapes ps []
  | (p, _) :: ps, t :: ts => (p, t) :: paramShapes ps ts

/-- The types a call must pass. -/
def wantOf (ps : List (Name × Option Ceiling)) (ts : List Ty) : List Ty :=
  (paramShapes ps ts).map Prod.snd

/-- The names of a list of parameters. -/
def names (ps : List (Name × Option Ceiling)) : List Name := ps.map Prod.fst

/-! ## Running a statement -/

/-- A value. A closure has its own parameters, its body, the environment
it captured, and the ceilings of the slots it was bound into. -/
inductive Val where
  | closure (ps : List (Name × Option Ceiling)) (body : Stmt) (env : List (Name × Val)) (ceilings : List Ceiling)
  | pool (p : Name)
  | base (b : Name)
  | ok (v : Val)
  | err (v : Val)
  | variant (e tag b : Name)

abbrev Env := List (Name × Val)

def lookup {α : Type} : List (Name × α) → Name → Option α
  | [], _ => none
  | (y, a) :: rest, x => if y = x then some a else lookup rest x

/-- A value as bound into a slot: into one with a ceiling, it takes it on. -/
def Val.underCeiling (v : Val) : Option Ceiling → Val
  | none => v
  | some c => match v with
    | .closure ps body env ks => .closure ps body env (c :: ks)
    | v => v

/-- The variant a value is, with what it carries; `Ok` and `Err` for a
`Result`. -/
def Val.tagOf : Val → Option (Name × Val)
  | .ok v => some ("Ok", v)
  | .err v => some ("Err", v)
  | .variant _ tag b => some (tag, .base b)
  | _ => none

/-- What a run checks a value against where a type is declared: a
closure's parameters, a pool's profile (any pool for a type parameter,
which a run no longer knows), a `Result`'s or an enum's variant. -/
def Conforms : Ty → Val → Prop
  | .fn pks, .closure ps _ _ _ => ps.map Prod.snd = pks
  | .pool (.prof p), .pool q => p = q
  | .pool (.var _), .pool _ => True
  | .base b, .base c => b = c
  | .result t _ rem, .ok v => "Ok" ∈ rem ∧ Conforms t v
  | .result _ e rem, .err v => "Err" ∈ rem ∧ Conforms e v
  | .enum e rem, .variant e' tag _ => e = e' ∧ tag ∈ rem
  | _, _ => False

/-- Each argument's value, checked against its parameter's type. -/
inductive ArgsConform (ρ : List (Name × Val)) : List Ty → List Name → Prop
  | nil : ArgsConform ρ [] []
  | cons {t ts a as v} : lookup ρ a = some v → Conforms t v → ArgsConform ρ ts as →
      ArgsConform ρ (t :: ts) (a :: as)

/-- The values a `VExpr` may give: an operation may succeed or fail. -/
inductive EvalV : VExpr → Val → Prop
  | pool {p} : EvalV (.pool p) (.pool p)
  | outcomeOk {t} : EvalV (.outcome t) (.ok (.base t))
  | outcomeErr {t} : EvalV (.outcome t) (.err (.base "Str"))
  | variant {e tag b} : EvalV (.variant e tag b) (.variant e tag b)

/-- One operation that ran. -/
structure Event where
  effect : Eff
  confirmed : Bool
  ceilings : List Ceiling

/-- A named function's parameters bound to its arguments' values. -/
inductive BindArgs (ρ : Env) : List (Name × Option Ceiling) → List Name → Env → Prop
  | nil : BindArgs ρ [] [] []
  | cons {p k a ps as v ρf} :
      lookup ρ a = some v → BindArgs ρ ps as ρf →
      BindArgs ρ ((p, k) :: ps) (a :: as) ((p, v.underCeiling k) :: ρf)

/-- `Eval P ρ cs s t b`: in environment `ρ`, under the ceilings `cs` of the
closure calls in progress, `s` can run and perform the events `t`; `b` says
whether it stopped at a `?` (`stop`), so that what follows in the same
function or closure does not run. A call returns normally either way. -/
inductive Eval (P : Program) : Env → List Ceiling → Stmt → List Event → Bool → Prop
  | opEffect {ρ cs o e} : o.effect = some e →
      Eval P ρ cs (.op o) [⟨e, o.confirmed, cs⟩] false
  | opPure {ρ cs o} : o.effect = none → Eval P ρ cs (.op o) [] false
  | stop {ρ cs} : Eval P ρ cs .stop [] true
  | seq {ρ cs s₁ s₂ t₁ t₂ b} : Eval P ρ cs s₁ t₁ false → Eval P ρ cs s₂ t₂ b →
      Eval P ρ cs (.seq s₁ s₂) (t₁ ++ t₂) b
  | seqStop {ρ cs s₁ s₂ t₁} : Eval P ρ cs s₁ t₁ true → Eval P ρ cs (.seq s₁ s₂) t₁ true
  | choiceLeft {ρ cs s₁ s₂ t b} : Eval P ρ cs s₁ t b → Eval P ρ cs (.choice s₁ s₂) t b
  | choiceRight {ρ cs s₁ s₂ t b} : Eval P ρ cs s₂ t b → Eval P ρ cs (.choice s₁ s₂) t b
  | letClosure {ρ cs x k ps body rest t b} :
      Eval P ((x, .closure ps body ρ k.toList) :: ρ) cs rest t b →
      Eval P ρ cs (.letClosure x k ps body rest) t b
  | letAlias {ρ cs x y rest v t b} : lookup ρ y = some v →
      Eval P ((x, v) :: ρ) cs rest t b → Eval P ρ cs (.letAlias x y rest) t b
  | letOpaque {ρ cs x k ps body rest t b} :
      Eval P ((x, .closure ps body ρ k.toList) :: ρ) cs rest t b →
      Eval P ρ cs (.letOpaque x k ps body rest) t b
  /-- The closure runs in the environment it captured, with its own
  parameters bound to the arguments, each taking on its parameter's
  ceiling. -/
  | callClosure {ρ cs x args pks ps body ρ' ks ρa t b} :
      lookup ρ x = some (.closure ps body ρ' ks) → ps.map Prod.snd = pks →
      ArgsConform ρ (List.replicate args.length (.fn [])) args →
      BindArgs ρ ps args ρa → Eval P (ρa ++ ρ') (ks ++ cs) body t b →
      Eval P ρ cs (.callClosure x args pks) t false
  /-- A function's arguments are checked against its parameters' types. -/
  | callFun {ρ cs f args inst d ρf t b} : P f = some d →
      ArgsConform ρ (wantOf d.params d.ptypes) args → BindArgs ρ d.params args ρf →
      Eval P ρf cs d.body t b → Eval P ρ cs (.callFun f args inst) t false
  | loopDone {ρ cs body} : Eval P ρ cs (.loop body) [] false
  | loopStep {ρ cs body t₁ t₂ b} : Eval P ρ cs body t₁ false → Eval P ρ cs (.loop body) t₂ b →
      Eval P ρ cs (.loop body) (t₁ ++ t₂) b
  | loopStop {ρ cs body t} : Eval P ρ cs body t true → Eval P ρ cs (.loop body) t true
  | letVal {ρ cs x ex rest v t b} : EvalV ex v → Eval P ((x, v) :: ρ) cs rest t b →
      Eval P ρ cs (.letVal x ex rest) t b
  /-- `?` on an `Ok` binds what it holds, checked against the declared type. -/
  | tryOk {ρ cs x ty y rest v t b} : lookup ρ y = some (.ok v) → Conforms ty v →
      Eval P ((x, v) :: ρ) cs rest t b → Eval P ρ cs (.tryOk x ty y rest) t b
  /-- `?` on an `Err` returns from the function. -/
  | tryErr {ρ cs x ty y rest v} : lookup ρ y = some (.err v) →
      Eval P ρ cs (.tryOk x ty y rest) [] true
  | matchThen {ρ cs y tag z s₁ s₂ v pv t b} : lookup ρ y = some v → v.tagOf = some (tag, pv) →
      Eval P ((z, pv) :: ρ) cs s₁ t b → Eval P ρ cs (.matchTag y tag z s₁ s₂) t b
  /-- The arms after this one see `y` again, a value that is not `tag`. -/
  | matchElse {ρ cs y tag z s₁ s₂ v tag' pv t b} : lookup ρ y = some v →
      v.tagOf = some (tag', pv) → tag' ≠ tag →
      Eval P ((y, v) :: ρ) cs s₂ t b → Eval P ρ cs (.matchTag y tag z s₁ s₂) t b

/-! ## The rules -/

/-- What a name in scope stands for: a closure whose effects `R`, used
parameters `U` and own parameters `ps` are known (a closure literal, a slot
with a ceiling, an untraceable closure), or a parameter without a ceiling.
`U` may name the closure's own parameters without ceilings, which each call
fills; `ps` names all of its own parameters. -/
inductive Binding where
  | known (R : Effs) (U : List Name) (ps : List Name)
  | param (p : Name)

def Binding.R : Binding → Effs
  | .known R _ _ => R
  | .param _ => Effs.none

/-- Every parameter without a ceiling whose closure a call may run, the
closure's own included. -/
def Binding.U : Binding → List Name
  | .known _ U _ => U
  | .param p => [p]

def Binding.ps : Binding → List Name
  | .known _ _ ps => ps
  | .param _ => []

/-- The names in `U` that are not among `ps`. -/
def outerOf (U ps : List Name) : List Name := U.filter fun q => decide (q ∉ ps)

/-- The parameters a call may run closures of that the call does not fill:
the enclosing function's, or an enclosing closure's. -/
def Binding.outer (b : Binding) : List Name := outerOf b.U b.ps

/-- Whether the closure may call its own arguments. A parameter's closure
may. -/
def Binding.callsOwn : Binding → Bool
  | .known _ U ps => U.any fun q => decide (q ∈ ps)
  | .param _ => true

/-- Whether a call must count the closures it passes: a closure with
parameters of its own, or a parameter's closure, may run them. A slot with
a ceiling may not, since a closure that calls its arguments fits none. -/
def Binding.mayCallArgs : Binding → Bool
  | .known _ _ ps => !ps.isEmpty
  | .param _ => true

/-- Whether a call counts the closures it passes to parameters with a
ceiling too. A call through a parameter does, since it cannot see which of
them the closure the parameter holds runs; they fit their ceilings, so
they add effects only. -/
def Binding.addsAll : Binding → Bool
  | .known _ _ _ => false
  | .param _ => true

/-- What the closures passed to a call add to it. -/
def argEffs (b : Binding) (R : Effs) : Effs := if b.mayCallArgs then R else Effs.none

def argUses (b : Binding) (U : List Name) : List Name := if b.mayCallArgs then U else []

abbrev Ctx := List (Name × Binding)

/-- What the closures held by the parameters in `U` may do, if parameter `p`
holds closures doing at most `σ p`. -/
def bigU (σ : Name → Effs) (U : List Name) : Effs := fun e => ∃ p, p ∈ U ∧ σ p e

def Binding.sem (b : Binding) (σ : Name → Effs) : Effs := Effs.union b.R (bigU σ b.outer)

/-- What a closure the rules cannot see into may do: anything, unless the
type it comes through names a ceiling. -/
def opaqueEffs : Option Ceiling → Effs
  | none => Effs.all
  | some c => c.allows

/-- What an untraceable closure may call of its own arguments: any of them,
unless a ceiling says it calls none. -/
def opaqueU : Option Ceiling → List Name → List Name
  | none, ps => ps
  | some _, _ => []

/-- The own parameters an untraceable closure's binding records: none under
a ceiling, which it does not call. -/
def opaquePs : Option Ceiling → List Name → List Name
  | none, ps => ps
  | some _, _ => []

def paramBinding (p : Name) : Option Ceiling → Binding
  | none => .param p
  | some c => .known c.allows [] []

def paramCtx : List (Name × Option Ceiling) → Ctx
  | [] => []
  | (p, k) :: ps => (p, paramBinding p k) :: paramCtx ps

/-- What a name holding a pool, an operation's `Result` or an enum variant
stands for: calling it runs nothing. -/
def inert : Binding := .known Effs.none [] []

/-- What a name holding a value taken out of a `Result` or an enum stands
for: anything, calling any closure passed to it. -/
def untraced : Binding := .known Effs.all ["*"] ["*"]

/-- A named function's parameters as bindings: a closure parameter as
itself, one whose type is not a closure's as `inert`. A call checks what
it passes against those types, so such a parameter never holds a closure. -/
def fparamCtx : List (Name × Option Ceiling) → List Ty → Ctx
  | [], _ => []
  | (p, k) :: ps, [] => (p, paramBinding p k) :: fparamCtx ps []
  | (p, k) :: ps, t :: ts => (p, if t.isFn then paramBinding p k else inert) :: fparamCtx ps ts

/-- A closure with effects `R` using parameters `U` fits the ceiling `k`:
it uses no parameter without a ceiling, and every effect is allowed. -/
def Fits (k : Option Ceiling) (R : Effs) (U : List Name) : Prop :=
  ∀ c, k = some c → U = [] ∧ Effs.Sub R c.allows

/-- `A` added to `R` when `add` says so. -/
def addIf (add : Bool) (A R : Effs) : Effs := if add then Effs.union A R else R

/-- The arguments of a call, checked against the ceilings of the parameters
they fill: an argument to a parameter with a ceiling must fit it; the
others add their effects, and with `add` so do the ones that fit. -/
inductive ArgsOk (Γ : Ctx) (add : Bool) : List (Option Ceiling) → List Name → Effs → List Name → Prop
  | nil : ArgsOk Γ add [] [] Effs.none []
  | unannotated {a ks as b R U} : lookup Γ a = some b → ArgsOk Γ add ks as R U →
      ArgsOk Γ add (none :: ks) (a :: as) (Effs.union b.R R) (b.outer ++ U)
  | annotated {c a ks as b R U} : lookup Γ a = some b → b.U = [] →
      Effs.Sub b.R c.allows → ArgsOk Γ add ks as R U →
      ArgsOk Γ add (some c :: ks) (a :: as) (addIf add b.R R) U

/-- No closure the body can see by name runs a parameter named like one of
the closure's own: such a use would be taken for the closure's own. -/
def Fresh (ps : List Name) (Γ : Ctx) : Prop :=
  ∀ y b, lookup Γ y = some b → y ∉ ps → ∀ q, q ∈ b.outer → q ∉ ps

/-- A binding that may do anything, calling its own arguments if it has
any. -/
def weakB : Binding → Binding
  | .known _ _ ps => .known Effs.all ps ps
  | .param _ => .known Effs.all ["*"] ["*"]

/-- How a binding in scope is seen inside a closure whose own parameters
are `ps`: unchanged, unless it is still visible and runs an outer parameter
that a name in `ps` now hides, in which case it may do anything. -/
def weakenB (ps : List Name) (y : Name) (b : Binding) : Binding :=
  if (decide (y ∉ ps) && b.outer.any fun q => decide (q ∈ ps)) = true then weakB b else b

/-- The context a closure body is checked in, before its own parameters
are added: a closure parameter may reuse any name in scope. -/
def weaken (ps : List Name) : Ctx → Ctx
  | [] => []
  | (y, b) :: Γ => (y, weakenB ps y b) :: weaken ps Γ

/-- `Typed P Φ Γ s R U`: `s` may have the effects `R` and run the closures
of the parameters `U`. `Φ f` is the effect of calling `f`, its own body's
`R`. -/
inductive Typed (P : Program) (Φ : Name → Effs) : Ctx → Stmt → Effs → List Name → Prop
  | op {Γ o} : Typed P Φ Γ (.op o) o.effs []
  | seq {Γ s₁ s₂ R₁ R₂ U₁ U₂} : Typed P Φ Γ s₁ R₁ U₁ → Typed P Φ Γ s₂ R₂ U₂ →
      Typed P Φ Γ (.seq s₁ s₂) (Effs.union R₁ R₂) (U₁ ++ U₂)
  | choice {Γ s₁ s₂ R₁ R₂ U₁ U₂} : Typed P Φ Γ s₁ R₁ U₁ → Typed P Φ Γ s₂ R₂ U₂ →
      Typed P Φ Γ (.choice s₁ s₂) (Effs.union R₁ R₂) (U₁ ++ U₂)
  | letClosure {Γ x k ps body rest Rb Ub R U} :
      Typed P Φ (paramCtx ps ++ weaken (names ps) Γ) body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known Rb Ub (names ps)) :: Γ) rest R U →
      Typed P Φ Γ (.letClosure x k ps body rest) R U
  | letAlias {Γ x y rest b R U} : lookup Γ y = some b →
      Typed P Φ ((x, b) :: Γ) rest R U → Typed P Φ Γ (.letAlias x y rest) R U
  | letOpaque {Γ x k ps body rest Rb Ub R U} :
      Typed P Φ (paramCtx ps ++ weaken (names ps) Γ) body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known (opaqueEffs k) (opaqueU k (names ps)) (opaquePs k (names ps))) :: Γ) rest R U →
      Typed P Φ Γ (.letOpaque x k ps body rest) R U
  /-- A call may do what the closure does, and what the closures passed to
  it do if it may call them. -/
  | callClosure {Γ x args pks b R U} : lookup Γ x = some b → ArgsOk Γ b.addsAll pks args R U →
      Typed P Φ Γ (.callClosure x args pks) (Effs.union b.R (argEffs b R)) (b.outer ++ argUses b U)
  | callFun {Γ f args inst d R U} : P f = some d → ArgsOk Γ false (d.params.map Prod.snd) args R U →
      Typed P Φ Γ (.callFun f args inst) (Effs.union (Φ f) R) U
  /-- A loop may do whatever its body does, any number of times. -/
  | loop {Γ body R U} : Typed P Φ Γ body R U → Typed P Φ Γ (.loop body) R U
  | stop {Γ} : Typed P Φ Γ .stop Effs.none []
  /-- A pool, an operation's `Result` or an enum variant: binding one runs
  nothing. -/
  | letVal {Γ x ex rest R U} : Typed P Φ ((x, inert) :: Γ) rest R U →
      Typed P Φ Γ (.letVal x ex rest) R U
  | tryOk {Γ x ty y rest b R U} : lookup Γ y = some b →
      Typed P Φ ((x, untraced) :: Γ) rest R U → Typed P Φ Γ (.tryOk x ty y rest) R U
  /-- The arm or the arms after it may run. -/
  | matchTag {Γ y tag z s₁ s₂ b R₁ R₂ U₁ U₂} : lookup Γ y = some b →
      Typed P Φ ((z, untraced) :: Γ) s₁ R₁ U₁ → Typed P Φ ((y, b) :: Γ) s₂ R₂ U₂ →
      Typed P Φ Γ (.matchTag y tag z s₁ s₂) (Effs.union R₁ R₂) (U₁ ++ U₂)
  | noMatch {Γ y} : Typed P Φ Γ (.noMatch y) Effs.none []

/-- Every function's body is accepted, with its effect within its
signature. -/
def ProgramOk (P : Program) (Φ : Name → Effs) : Prop :=
  ∀ f d, P f = some d → ∃ R U, Typed P Φ (fparamCtx d.params d.ptypes) d.body R U ∧ Effs.Sub R (Φ f)

/-! ## What a value may do -/

mutual
/-- `ValOk P Φ B o v`: running `v` performs only effects in `B`, besides
those of the closures passed to it; it calls none of those unless `o`; and
under each of its ceilings it calls none of them and does only what the
ceiling allows. -/
inductive ValOk (P : Program) (Φ : Name → Effs) : Effs → Bool → Val → Prop
  | closure {B o Γ σ ps body env ks R U} :
      Fresh (names ps) Γ → Typed P Φ (paramCtx ps ++ Γ) body R U → EnvOk P Φ [] Γ env σ →
      Effs.Sub (Effs.union R (bigU σ (outerOf U (names ps)))) B →
      (o = false → ∀ q, q ∈ U → q ∉ names ps) →
      (∀ c, c ∈ ks → (∀ q, q ∈ U → q ∉ names ps) ∧ Effs.Sub (Effs.union R (bigU σ U)) c.allows) →
      ValOk P Φ B o (.closure ps body env ks)
  | pool {B o p} : ValOk P Φ B o (.pool p)
  | base {B o b} : ValOk P Φ B o (.base b)
  /-- What a `Result` holds may do anything, as long as it was made by
  the program. -/
  | ok {B o v} : ValOk P Φ Effs.all true v → ValOk P Φ B o (.ok v)
  | err {B o v} : ValOk P Φ Effs.all true v → ValOk P Φ B o (.err v)
  | variant {B o e tag b} : ValOk P Φ B o (.variant e tag b)

/-- Each name's value does at most what its binding says. A name already
in `seen` is hidden by an earlier one, so nothing is asked of it. -/
inductive EnvOk (P : Program) (Φ : Name → Effs) : List Name → Ctx → Env → (Name → Effs) → Prop
  | nil {seen σ} : EnvOk P Φ seen [] [] σ
  | cons {seen x b v Γ ρ σ} : ValOk P Φ (b.sem σ) b.callsOwn v →
      EnvOk P Φ (x :: seen) Γ ρ σ → EnvOk P Φ seen ((x, b) :: Γ) ((x, v) :: ρ) σ
  | hidden {seen x b v Γ ρ σ} : x ∈ seen →
      EnvOk P Φ (x :: seen) Γ ρ σ → EnvOk P Φ seen ((x, b) :: Γ) ((x, v) :: ρ) σ
end

/-! ## Lemmas -/

theorem bigU_nil (σ : Name → Effs) : Effs.Sub (bigU σ []) Effs.none := by
  intro e ⟨p, hp, _⟩
  cases hp

theorem bigU_append_left (σ : Name → Effs) (U₁ U₂ : List Name) :
    Effs.Sub (bigU σ U₁) (bigU σ (U₁ ++ U₂)) := by
  intro e ⟨p, hp, h⟩
  exact ⟨p, List.mem_append.mpr (Or.inl hp), h⟩

theorem bigU_append_right (σ : Name → Effs) (U₁ U₂ : List Name) :
    Effs.Sub (bigU σ U₂) (bigU σ (U₁ ++ U₂)) := by
  intro e ⟨p, hp, h⟩
  exact ⟨p, List.mem_append.mpr (Or.inr hp), h⟩

/-- Under a constant `σ`, used parameters may do at most that constant. -/
theorem bigU_const (B : Effs) (U : List Name) : Effs.Sub (bigU (fun _ => B) U) B := by
  intro e ⟨_, _, h⟩
  exact h

/-- A closure that fits a ceiling does at most what the ceiling allows,
whatever its parameters hold. -/
theorem fits_sem {R : Effs} {U : List Name} {σ : Name → Effs} {c : Ceiling}
    (hU : U = []) (hR : Effs.Sub R c.allows) : Effs.Sub (Effs.union R (bigU σ U)) c.allows := by
  intro e he
  rcases he with h₁ | h₂
  · exact hR e h₁
  · rw [hU] at h₂
    exact False.elim (bigU_nil σ e h₂)

theorem mem_toList {k : Option Ceiling} {c : Ceiling} (h : c ∈ k.toList) : k = some c := by
  cases k with
  | none => simp [Option.toList] at h
  | some c' =>
    simp [Option.toList] at h
    rw [h]

theorem mem_outerOf {q : Name} {U ps : List Name} : q ∈ outerOf U ps ↔ q ∈ U ∧ q ∉ ps := by
  simp [outerOf, List.mem_filter]

/-- With none of its own parameters used, a closure's outer uses are all
of its uses. -/
theorem bigU_sub_outerOf {σ : Name → Effs} {U ps : List Name} (h : ∀ q, q ∈ U → q ∉ ps) :
    Effs.Sub (bigU σ U) (bigU σ (outerOf U ps)) := by
  intro e ⟨p, hp, hσ⟩
  exact ⟨p, mem_outerOf.mpr ⟨hp, h p hp⟩, hσ⟩

theorem callsOwn_false {b : Binding} (h : b.callsOwn = false) : ∀ q, q ∈ b.U → q ∉ b.ps := by
  cases b with
  | known R U ps =>
    intro q hq hps
    have hany : (U.any fun q => decide (q ∈ ps)) = true :=
      List.any_eq_true.mpr ⟨q, hq, decide_eq_true hps⟩
    simp only [Binding.callsOwn] at h
    rw [h] at hany
    cases hany
  | param p => simp [Binding.callsOwn] at h

theorem callsOwn_of_U_nil {b : Binding} (h : b.U = []) : b.callsOwn = false := by
  cases b with
  | known R U ps =>
    simp only [Binding.U] at h
    subst h
    rfl
  | param p => simp [Binding.U] at h

/-- A binding with no outer uses does at most its own effects. -/
theorem sem_of_outer_nil {b : Binding} {σ : Name → Effs} (h : b.outer = []) :
    Effs.Sub (b.sem σ) b.R := by
  intro e he
  rcases he with hR | ⟨q, hq, _⟩
  · exact hR
  · rw [h] at hq
    cases hq

theorem outer_of_U_nil {b : Binding} (h : b.U = []) : b.outer = [] := by
  simp [Binding.outer, outerOf, h]

theorem param_mem_outer (p : Name) : p ∈ (Binding.param p).outer := by
  simp [Binding.outer, outerOf, Binding.U, Binding.ps]

theorem ValOk.mono {P Φ B B' o o' v} (h : ValOk P Φ B o v) (hs : Effs.Sub B B')
    (ho : o' = false → o = false) : ValOk P Φ B' o' v := by
  cases h with
  | closure hf ht henv hb hown hc =>
    exact ValOk.closure hf ht henv (Effs.Sub.trans hb hs) (fun h' => hown (ho h')) hc
  | pool => exact ValOk.pool
  | base => exact ValOk.base
  | ok hv => exact ValOk.ok hv
  | err hv => exact ValOk.err hv
  | variant => exact ValOk.variant

theorem evalV_ok {P Φ B o ex v} (h : EvalV ex v) : ValOk P Φ B o v := by
  cases h with
  | pool => exact ValOk.pool
  | outcomeOk => exact ValOk.ok ValOk.base
  | outcomeErr => exact ValOk.err ValOk.base
  | variant => exact ValOk.variant

theorem untraced_ok {P Φ σ v} (h : ValOk P Φ Effs.all true v) :
    ValOk P Φ (untraced.sem σ) untraced.callsOwn v :=
  h.mono (fun _ _ => Or.inl trivial) (fun h' => by simp [untraced, Binding.callsOwn] at h')

/-- What a variant carries was made by the program. -/
theorem ValOk.payload {P Φ B o v tag pv} (h : ValOk P Φ B o v) (ht : v.tagOf = some (tag, pv)) :
    ValOk P Φ Effs.all true pv := by
  cases v with
  | closure => simp [Val.tagOf] at ht
  | pool => simp [Val.tagOf] at ht
  | base => simp [Val.tagOf] at ht
  | ok w =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    cases h with
    | ok hw => exact hw
  | err w =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    cases h with
    | err hw => exact hw
  | variant e tag' b =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    exact ValOk.base

theorem EnvOk.find {P Φ σ x v} : ∀ {seen Γ ρ}, EnvOk P Φ seen Γ ρ σ → x ∉ seen →
    NucleScript.lookup ρ x = some v →
    ∃ b, NucleScript.lookup Γ x = some b ∧ ValOk P Φ (b.sem σ) b.callsOwn v := by
  intro seen Γ ρ h hx hl
  induction ρ generalizing Γ seen with
  | nil => simp [NucleScript.lookup] at hl
  | cons yw ρ ih =>
    obtain ⟨y, w⟩ := yw
    have hx' : y ≠ x → x ∉ y :: seen := by
      intro hyx hm
      rcases List.mem_cons.mp hm with h' | h'
      · exact hyx h'.symm
      · exact hx h'
    cases h with
    | cons hw hrest =>
      by_cases hyx : y = x
      · subst hyx
        simp [NucleScript.lookup] at hl
        subst hl
        exact ⟨_, by simp [NucleScript.lookup], hw⟩
      · simp [NucleScript.lookup, hyx] at hl
        obtain ⟨b', hb', hv'⟩ := ih hrest (hx' hyx) hl
        exact ⟨b', by simp [NucleScript.lookup, hyx, hb'], hv'⟩
    | hidden hs hrest =>
      by_cases hyx : y = x
      · subst hyx
        exact absurd hs hx
      · simp [NucleScript.lookup, hyx] at hl
        obtain ⟨b', hb', hv'⟩ := ih hrest (hx' hyx) hl
        exact ⟨b', by simp [NucleScript.lookup, hyx, hb'], hv'⟩

theorem EnvOk.mono_seen {P Φ σ} : ∀ {ρ seen seen' Γ}, EnvOk P Φ seen Γ ρ σ →
    (∀ q, q ∈ seen → q ∈ seen') → EnvOk P Φ seen' Γ ρ σ := by
  intro ρ
  induction ρ with
  | nil =>
    intro seen seen' Γ h _
    cases h
    exact EnvOk.nil
  | cons xv ρ ih =>
    intro seen seen' Γ h hs
    have hs' : ∀ x, ∀ q, q ∈ x :: seen → q ∈ x :: seen' := by
      intro x q hq
      rcases List.mem_cons.mp hq with h' | h'
      · subst h'
        simp
      · exact List.mem_cons_of_mem _ (hs q h')
    cases h with
    | cons hw hr => exact EnvOk.cons hw (ih hr (hs' _))
    | hidden hd hr => exact EnvOk.hidden (hs _ hd) (ih hr (hs' _))

theorem EnvOk.append {P Φ σ Γ₂ ρ₂} :
    ∀ {ρ₁ seen Γ₁}, EnvOk P Φ seen Γ₁ ρ₁ σ →
    EnvOk P Φ ((Γ₁.map Prod.fst).reverse ++ seen) Γ₂ ρ₂ σ →
    EnvOk P Φ seen (Γ₁ ++ Γ₂) (ρ₁ ++ ρ₂) σ := by
  intro ρ₁
  induction ρ₁ with
  | nil =>
    intro seen Γ₁ h₁ h₂
    cases h₁
    simpa using h₂
  | cons xv ρ ih =>
    intro seen Γ₁ h₁ h₂
    cases h₁ with
    | cons hw hr =>
      refine EnvOk.cons hw (ih hr ?_)
      simpa [List.map_cons, List.reverse_cons, List.append_assoc] using h₂
    | hidden hd hr =>
      refine EnvOk.hidden hd (ih hr ?_)
      simpa [List.map_cons, List.reverse_cons, List.append_assoc] using h₂

/-- An environment keeps its meaning under a new `σ` that agrees on every
parameter its visible bindings use. -/
theorem EnvOk.congr {P Φ σ σ'} : ∀ {ρ seen Γ}, EnvOk P Φ seen Γ ρ σ →
    (∀ y b, NucleScript.lookup Γ y = some b → y ∉ seen → ∀ q, q ∈ b.outer → σ' q = σ q) →
    EnvOk P Φ seen Γ ρ σ' := by
  intro ρ
  induction ρ with
  | nil =>
    intro seen Γ h _
    cases h
    exact EnvOk.nil
  | cons xv ρ ih =>
    intro seen Γ h hσ
    have tail : ∀ (x : Name) (b : Binding) (Γ' : Ctx),
        (∀ (y : Name) (b₁ : Binding), NucleScript.lookup ((x, b) :: Γ') y = some b₁ → y ∉ seen →
          ∀ q, q ∈ b₁.outer → σ' q = σ q) →
        ∀ (y : Name) (b' : Binding), NucleScript.lookup Γ' y = some b' → y ∉ x :: seen →
          ∀ q, q ∈ b'.outer → σ' q = σ q := by
      intro x b Γ' hσ y b' hyb hy q hq
      have hyx : y ≠ x := fun h => hy (by simp [h])
      have hl : NucleScript.lookup ((x, b) :: Γ') y = some b' := by
        simp [NucleScript.lookup, Ne.symm hyx, hyb]
      exact hσ y b' hl (fun h => hy (List.mem_cons_of_mem _ h)) q hq
    cases h with
    | @cons _ x b v Γ' _ _ hv hr =>
      by_cases hxs : x ∈ seen
      · exact EnvOk.hidden hxs (ih hr (tail x b Γ' hσ))
      · refine EnvOk.cons (hv.mono ?_ (fun h => h)) (ih hr (tail x b Γ' hσ))
        intro e he
        rcases he with hR | ⟨q, hq, hqe⟩
        · exact Or.inl hR
        · refine Or.inr ⟨q, hq, ?_⟩
          rw [hσ x b (by simp [NucleScript.lookup]) hxs q hq]
          exact hqe
    | @hidden _ x b v Γ' _ _ hd hr => exact EnvOk.hidden hd (ih hr (tail x b Γ' hσ))

theorem lookup_weaken (ps : List Name) (y : Name) :
    ∀ Γ : Ctx, NucleScript.lookup (weaken ps Γ) y = (NucleScript.lookup Γ y).map (weakenB ps y)
  | [] => rfl
  | (z, b) :: Γ => by
    by_cases hzy : z = y
    · subst hzy
      simp [weaken, NucleScript.lookup]
    · simp [weaken, NucleScript.lookup, hzy, lookup_weaken ps y Γ]

theorem fresh_weaken (ps : List Name) (Γ : Ctx) : Fresh ps (weaken ps Γ) := by
  intro y b hl hy q hq hqp
  rw [lookup_weaken] at hl
  cases hb : NucleScript.lookup Γ y with
  | none =>
    rw [hb] at hl
    cases hl
  | some b₀ =>
    rw [hb] at hl
    simp only [Option.map_some, Option.some.injEq] at hl
    subst hl
    unfold weakenB at hq
    by_cases hc : (decide (y ∉ ps) && b₀.outer.any fun q => decide (q ∈ ps)) = true
    · simp only [hc, ↓reduceIte] at hq
      cases b₀ with
      | known R U ps' =>
        simp only [weakB, Binding.outer, Binding.U, Binding.ps] at hq
        exact (mem_outerOf.mp hq).2 (mem_outerOf.mp hq).1
      | param p =>
        simp only [weakB, Binding.outer, Binding.U, Binding.ps] at hq
        exact (mem_outerOf.mp hq).2 (mem_outerOf.mp hq).1
    · simp only [hc] at hq
      apply hc
      simp only [Bool.and_eq_true, decide_eq_true_eq]
      exact ⟨hy, List.any_eq_true.mpr ⟨q, hq, decide_eq_true hqp⟩⟩

theorem weakenB_ok {P Φ σ ps y b v} (h : ValOk P Φ (b.sem σ) b.callsOwn v) :
    ValOk P Φ ((weakenB ps y b).sem σ) (weakenB ps y b).callsOwn v := by
  unfold weakenB
  by_cases hc : (decide (y ∉ ps) && b.outer.any fun q => decide (q ∈ ps)) = true
  · simp only [hc, ↓reduceIte]
    cases b with
    | known R U ps' =>
      refine h.mono (fun e _ => Or.inl trivial) ?_
      intro hco
      cases ps' with
      | nil => simp [Binding.callsOwn]
      | cons p ps'' => simp [weakB, Binding.callsOwn] at hco
    | param p =>
      refine h.mono (fun e _ => Or.inl trivial) ?_
      intro hco
      simp [weakB, Binding.callsOwn] at hco
  · simp only [hc]
    exact h

theorem EnvOk.weaken {P Φ σ ps} : ∀ {ρ seen Γ}, EnvOk P Φ seen Γ ρ σ →
    EnvOk P Φ seen (NucleScript.weaken ps Γ) ρ σ := by
  intro ρ
  induction ρ with
  | nil =>
    intro seen Γ h
    cases h
    exact EnvOk.nil
  | cons xv ρ ih =>
    intro seen Γ h
    cases h with
    | cons hv hr => exact EnvOk.cons (weakenB_ok hv) (ih hr)
    | hidden hd hr => exact EnvOk.hidden hd (ih hr)

theorem paramCtx_names : ∀ ps : List (Name × Option Ceiling), (paramCtx ps).map Prod.fst = names ps
  | [] => rfl
  | (p, k) :: ps => by
    have h := paramCtx_names ps
    simp only [names] at h ⊢
    simp only [paramCtx, List.map_cons, h]

theorem mem_names_of {p : Name} {k : Option Ceiling} {ps : List (Name × Option Ceiling)}
    (h : (p, k) ∈ ps) : p ∈ names ps :=
  List.mem_map.mpr ⟨(p, k), h, rfl⟩

theorem sub_addIf (add : Bool) (A R : Effs) : Effs.Sub R (addIf add A R) := by
  intro e h
  cases add with
  | false => exact h
  | true => exact Or.inr h

/-- The callee's environment does what the arguments' bindings say: an
argument to a parameter with a ceiling fits it and takes it on, and each
parameter without one holds a closure doing at most `σ'' p`, which bounds
everything the call passes. -/
theorem bindArgs_ok {P Φ Γ ρ σ σ'' A} (henv : EnvOk P Φ [] Γ ρ σ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → (∀ p, (p, none) ∈ ps → Effs.Sub A (σ'' p)) →
    ∀ {add R U seen}, ArgsOk Γ add (ps.map Prod.snd) as R U →
    Effs.Sub (Effs.union R (bigU σ U)) A →
    EnvOk P Φ seen (paramCtx ps) ρf σ'' := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _ _ _ _ _ _; exact EnvOk.nil
  | @cons p k a ps as v ρf hl hrest ih =>
    intro hps add R U seen ha hA
    have hps' : ∀ p', (p', none) ∈ ps → Effs.Sub A (σ'' p') :=
      fun p' h => hps p' (List.mem_cons_of_mem _ h)
    obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
    simp only [List.map_cons] at ha
    cases ha with
    | unannotated hlb ht =>
      rw [hlb] at hb'
      cases hb'
      have hpA := hps p (by simp)
      refine EnvOk.cons ?_ (ih hps' ht ?_)
      · refine hv.mono ?_ (fun h => by simp [paramBinding, Binding.callsOwn] at h)
        intro e he
        refine Or.inr ⟨p, param_mem_outer p, hpA e (hA e ?_)⟩
        rcases he with hR | hU
        · exact Or.inl (Or.inl hR)
        · exact Or.inr (bigU_append_left σ _ _ e hU)
      · intro e he
        apply hA e
        rcases he with hR | hU
        · exact Or.inl (Or.inr hR)
        · exact Or.inr (bigU_append_right σ _ _ e hU)
    | annotated hlb hU hR ht =>
      rw [hlb] at hb'
      cases hb'
      refine EnvOk.cons ?_ (ih hps' ht ?_)
      · -- The value takes on the ceiling, which its effects fit, and it
        -- calls none of its own arguments, since its binding uses no
        -- parameter.
        have hbR := sem_of_outer_nil (σ := σ) (outer_of_U_nil hU)
        cases hv with
        | pool => exact ValOk.pool
        | base => exact ValOk.base
        | ok hw => exact ValOk.ok hw
        | err hw => exact ValOk.err hw
        | variant => exact ValOk.variant
        | closure hf ht' henv' hb hown hc =>
          have hno := hown (callsOwn_of_U_nil hU)
          refine ValOk.closure hf ht' henv' ?_ (fun _ => hno) ?_
          · intro e he
            exact Or.inl (hR e (hbR e (hb e he)))
          · intro c' hc'
            rcases List.mem_cons.mp hc' with heq | hin
            · subst heq
              refine ⟨hno, ?_⟩
              intro e he
              refine hR e (hbR e (hb e ?_))
              rcases he with h₁ | h₂
              · exact Or.inl h₁
              · exact Or.inr (bigU_sub_outerOf hno e h₂)
            · exact hc c' hin
      · intro e he
        apply hA e
        rcases he with h₁ | h₂
        · exact Or.inl (sub_addIf _ _ _ e h₁)
        · exact Or.inr h₂

theorem Conforms.not_closure {t v} (h : Conforms t v) (ht : t.isFn = false) :
    ∀ ps body env ks, v ≠ .closure ps body env ks := by
  intro ps body env ks hv
  subst hv
  cases t with
  | fn _ => simp [Ty.isFn] at ht
  | pool p => cases p <;> simp [Conforms] at h
  | base _ => simp [Conforms] at h
  | result _ _ _ => simp [Conforms] at h
  | enum _ _ => simp [Conforms] at h

theorem Val.underCeiling_data {v : Val} (hv : ∀ ps body env ks, v ≠ .closure ps body env ks)
    (k : Option Ceiling) : v.underCeiling k = v := by
  cases k with
  | none => rfl
  | some c =>
    cases v with
    | closure ps body env ks => exact absurd rfl (hv ps body env ks)
    | pool => rfl
    | base => rfl
    | ok => rfl
    | err => rfl
    | variant => rfl

theorem ValOk.data_any {P Φ B B' o o' v} (h : ValOk P Φ B o v)
    (hv : ∀ ps body env ks, v ≠ .closure ps body env ks) : ValOk P Φ B' o' v := by
  cases h with
  | closure _ _ _ _ _ _ => exact absurd rfl (hv _ _ _ _)
  | pool => exact ValOk.pool
  | base => exact ValOk.base
  | ok hw => exact ValOk.ok hw
  | err hw => exact ValOk.err hw
  | variant => exact ValOk.variant

/-- A function's parameters whose types are not a closure's hold values
that are not closures, which do nothing whatever the binding says. -/
theorem EnvOk.fparam {P Φ ρ σ''} : ∀ {ps as ρf}, BindArgs ρ ps as ρf → ∀ {ts},
    ArgsConform ρ (wantOf ps ts) as → ∀ {seen}, EnvOk P Φ seen (paramCtx ps) ρf σ'' →
    EnvOk P Φ seen (fparamCtx ps ts) ρf σ'' := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _ _ h; exact h
  | @cons p k a ps as v ρf hl _ ih =>
    intro ts hconf seen h
    cases ts with
    | nil =>
      simp only [wantOf, paramShapes, List.map_cons] at hconf
      cases hconf with
      | cons _ _ hrest =>
        cases h with
        | cons hv hr => exact EnvOk.cons hv (ih (ts := []) hrest hr)
        | hidden hd hr => exact EnvOk.hidden hd (ih (ts := []) hrest hr)
    | cons t ts =>
      simp only [wantOf, paramShapes, List.map_cons] at hconf
      cases hconf with
      | cons hl' hc hrest =>
        rw [hl] at hl'
        cases hl'
        cases h with
        | cons hv hr =>
          refine EnvOk.cons ?_ (ih (ts := ts) hrest hr)
          cases ht : t.isFn with
          | true =>
            simp only [ite_true]
            exact hv
          | false =>
            have hd := Conforms.not_closure hc ht
            simp only [Bool.false_eq_true, ite_false]
            rw [Val.underCeiling_data hd] at hv ⊢
            exact hv.data_any hd
        | hidden hd hr => exact EnvOk.hidden hd (ih (ts := ts) hrest hr)

/-! ## Soundness -/

/-- Every event a statement performs is within its rules' effects (with the
parameters' closures bounded by `σ`) and allowed by every ceiling in force
that the statement itself did not inherit. -/
theorem eval_sound {P Φ} (hP : ProgramOk P Φ) :
    ∀ {ρ cs s t b}, Eval P ρ cs s t b →
    ∀ {Γ R U σ}, Typed P Φ Γ s R U → EnvOk P Φ [] Γ ρ σ →
    ∀ ev, ev ∈ t →
      (Effs.union R (bigU σ U)) ev.effect ∧ (∀ c, c ∈ ev.ceilings → c ∈ cs ∨ c.allows ev.effect) := by
  intro ρ cs s t b h
  induction h with
  | stop =>
    intro _ _ _ _ _ _ ev hev
    cases hev
  | seqStop _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | seq h₁ _ =>
      obtain ⟨he, hc⟩ := ih h₁ henv ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inl hR)
      · exact Or.inr (bigU_append_left σ _ _ _ hU)
  | loopDone =>
    intro _ _ _ _ _ _ ev hev
    cases hev
  | loopStep _ _ ih₁ ih₂ =>
    intro Γ R U σ ht henv ev hev
    rcases List.mem_append.mp hev with hev | hev
    · cases ht with
      | loop hb => exact ih₁ hb henv ev hev
    · exact ih₂ ht henv ev hev
  | loopStop _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | loop hb => exact ih hb henv ev hev
  | opEffect he =>
    intro Γ R U σ ht _ ev hev
    cases ht
    rw [List.mem_singleton] at hev
    subst hev
    exact ⟨Or.inl he, fun c hc => Or.inl hc⟩
  | opPure _ =>
    intro _ _ _ _ _ _ ev hev
    cases hev
  | seq _ _ ih₁ ih₂ =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | seq h₁ h₂ =>
      rcases List.mem_append.mp hev with hev | hev
      · obtain ⟨he, hc⟩ := ih₁ h₁ henv ev hev
        refine ⟨?_, hc⟩
        rcases he with hR | hU
        · exact Or.inl (Or.inl hR)
        · exact Or.inr (bigU_append_left σ _ _ _ hU)
      · obtain ⟨he, hc⟩ := ih₂ h₂ henv ev hev
        refine ⟨?_, hc⟩
        rcases he with hR | hU
        · exact Or.inl (Or.inr hR)
        · exact Or.inr (bigU_append_right σ _ _ _ hU)
  | choiceLeft _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | choice h₁ _ =>
      obtain ⟨he, hc⟩ := ih h₁ henv ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inl hR)
      · exact Or.inr (bigU_append_left σ _ _ _ hU)
  | choiceRight _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | choice _ h₂ =>
      obtain ⟨he, hc⟩ := ih h₂ henv ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inr hR)
      · exact Or.inr (bigU_append_right σ _ _ _ hU)
  | @letClosure ρ cs x k ps body rest t b _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letClosure hb hfit hr =>
      refine ih hr (EnvOk.cons ?_ (henv.mono_seen (by simp))) ev hev
      refine ValOk.closure (fresh_weaken (names ps) Γ) hb henv.weaken (Effs.Sub.refl _) (fun h => callsOwn_false h) ?_
      intro c hc
      obtain ⟨hU, hR⟩ := hfit c (mem_toList hc)
      exact ⟨fun q hq => (by rw [hU] at hq; cases hq), fits_sem hU hR⟩
  | letAlias hl _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letAlias hlb hr =>
      obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
      rw [hlb] at hb'
      cases hb'
      exact ih hr (EnvOk.cons hv (henv.mono_seen (by simp))) ev hev
  | @letOpaque ρ cs x k ps body rest t b _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letOpaque hb hfit hr =>
      refine ih hr (EnvOk.cons ?_ (henv.mono_seen (by simp))) ev hev
      refine ValOk.closure (fresh_weaken (names ps) Γ) hb henv.weaken ?_ ?_ ?_
      · intro e he
        cases k with
        | none => exact Or.inl trivial
        | some c =>
          obtain ⟨hU, hR⟩ := hfit c rfl
          refine Or.inl (hR e ?_)
          rcases he with h₁ | ⟨q, hq, _⟩
          · exact h₁
          · rw [hU] at hq
            simp [outerOf] at hq
      · intro h q hq hps
        cases k with
        | none =>
          simp only [Binding.callsOwn, opaqueU, opaquePs, List.any_eq_false] at h
          exact h q hps (decide_eq_true hps)
        | some c =>
          obtain ⟨hU, _⟩ := hfit c rfl
          rw [hU] at hq
          cases hq
      · intro c hc
        obtain ⟨hU, hR⟩ := hfit c (mem_toList hc)
        exact ⟨fun q hq => (by rw [hU] at hq; cases hq), fits_sem hU hR⟩
  | @callClosure ρ cs x args pks ps body ρ' ks ρa t b hl hpks _ hbind _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | @callClosure _ _ _ _ bx Ra Ua hlb hargs =>
      subst hpks
      obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
      rw [hlb] at hb'
      cases hb'
      cases hv with
      | @closure _ _ Γc σc _ _ _ _ Rb Ub hfresh htb henvc hbound hown hceil =>
        -- The closure's own parameters hold the arguments; everything it
        -- captured keeps its meaning, since what it can see by name runs
        -- no parameter its own hide.
        obtain ⟨σ'', hin, hout⟩ : ∃ σ'' : Name → Effs,
            (∀ q, q ∈ names ps → σ'' q = Effs.union Ra (bigU σ Ua)) ∧
            (∀ q, q ∉ names ps → σ'' q = σc q) :=
          ⟨fun q => if q ∈ names ps then Effs.union Ra (bigU σ Ua) else σc q,
            fun q h => by simp [h], fun q h => by simp [h]⟩
        have henva : EnvOk P Φ [] (paramCtx ps) ρa σ'' := by
          refine bindArgs_ok henv hbind ?_ hargs (Effs.Sub.refl _)
          intro p hp
          rw [hin p (mem_names_of hp)]
          exact Effs.Sub.refl _
        have henvc' : EnvOk P Φ (names ps).reverse Γc ρ' σ'' :=
          (henvc.mono_seen (by simp)).congr
            (fun y b' hyb hy q hq => hout q (hfresh y b' hyb (by simpa using hy) q hq))
        have henvc'' : EnvOk P Φ (((paramCtx ps).map Prod.fst).reverse ++ []) Γc ρ' σ'' := by
          rw [paramCtx_names, List.append_nil]
          exact henvc'
        obtain ⟨he, hc⟩ := ih htb (EnvOk.append henva henvc'') ev hev
        have lift : ∀ e, Effs.union Rb (bigU σc (outerOf Ub (names ps))) e →
            Effs.union (Effs.union bx.R (argEffs bx Ra)) (bigU σ (bx.outer ++ argUses bx Ua)) e := by
          intro e h'
          rcases hbound e h' with hR | hU
          · exact Or.inl (Or.inl hR)
          · exact Or.inr (bigU_append_left σ _ _ e hU)
        refine ⟨?_, ?_⟩
        · rcases he with hR | ⟨q, hq, hqe⟩
          · exact lift _ (Or.inl hR)
          · by_cases hqp : q ∈ names ps
            · -- A use of its own parameter: the callee may call its
              -- arguments, so the call counts them.
              cases hm : bx.mayCallArgs with
              | false =>
                have hco : bx.callsOwn = false := by
                  cases bx with
                  | known R' U' ps' =>
                    simp only [Binding.mayCallArgs, Bool.not_eq_false', List.isEmpty_iff] at hm
                    subst hm
                    simp [Binding.callsOwn]
                  | param p => simp [Binding.mayCallArgs] at hm
                exact absurd hqp (hown hco q hq)
              | true =>
                rw [hin q hqp] at hqe
                rcases hqe with hR | hU
                · exact Or.inl (Or.inr (by simp [argEffs, hm]; exact hR))
                · exact Or.inr (bigU_append_right σ _ _ _ (by simp only [argUses, hm, ite_true]; exact hU))
            · rw [hout q hqp] at hqe
              exact lift _ (Or.inr ⟨q, mem_outerOf.mpr ⟨hq, hqp⟩, hqe⟩)
        · intro c hcin
          rcases hc c hcin with hks | hall
          · rcases List.mem_append.mp hks with hk | hcs
            · obtain ⟨hno, hsub⟩ := hceil c hk
              refine Or.inr (hsub _ ?_)
              rcases he with hR | ⟨q, hq, hqe⟩
              · exact Or.inl hR
              · rw [hout q (hno q hq)] at hqe
                exact Or.inr ⟨q, hq, hqe⟩
            · exact Or.inl hcs
          · exact Or.inr hall
  | @callFun ρ cs f args inst d ρf t b hf hconf hbind _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | @callFun _ _ _ _ _ Ra _ hf' hargs =>
      rw [hf] at hf'
      cases hf'
      obtain ⟨Rb, Ub, htb, hsig⟩ := hP f d hf
      have henvf := EnvOk.fparam hbind hconf
        (bindArgs_ok (seen := []) (σ'' := fun _ => Effs.union Ra (bigU σ U)) henv hbind
          (fun _ _ => Effs.Sub.refl _) hargs (Effs.Sub.refl _))
      obtain ⟨he, hc⟩ := ih htb henvf ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inl (hsig _ hR))
      · rcases bigU_const _ Ub _ hU with hR' | hU'
        · exact Or.inl (Or.inr hR')
        · exact Or.inr hU'
  | @letVal ρ cs x ex rest v t b hv _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letVal hr => exact ih hr (EnvOk.cons (evalV_ok hv) (henv.mono_seen (by simp))) ev hev
  | @tryOk ρ cs x ty y rest v t b hl _ _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | tryOk _ hr =>
      obtain ⟨_, _, hv⟩ := henv.find (by simp) hl
      have hw : ValOk P Φ Effs.all true v := by
        cases hv with
        | ok hw => exact hw
      exact ih hr (EnvOk.cons (untraced_ok hw) (henv.mono_seen (by simp))) ev hev
  | tryErr _ =>
    intro _ _ _ _ _ _ ev hev
    cases hev
  | @matchThen ρ cs y tag z s₁ s₂ v pv t b hl htag _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | matchTag _ h₁ _ =>
      obtain ⟨_, _, hv⟩ := henv.find (by simp) hl
      obtain ⟨he, hc⟩ := ih h₁ (EnvOk.cons (untraced_ok (hv.payload htag)) (henv.mono_seen (by simp))) ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inl hR)
      · exact Or.inr (bigU_append_left σ _ _ _ hU)
  | @matchElse ρ cs y tag z s₁ s₂ v tag' pv t b hl _ _ _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | matchTag hlb _ h₂ =>
      obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
      rw [hlb] at hb'
      cases hb'
      obtain ⟨he, hc⟩ := ih h₂ (EnvOk.cons hv (henv.mono_seen (by simp))) ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inr hR)
      · exact Or.inr (bigU_append_right σ _ _ _ hU)

/-! ## For a whole program -/

/-- A whole program, a statement run from nothing under no ceilings,
performs only effects its rules report. -/
theorem program_effects_sound {P Φ s t b R U} (hP : ProgramOk P Φ)
    (h : Eval P [] [] s t b) (ht : Typed P Φ [] s R U) :
    ∀ ev, ev ∈ t → R ev.effect := by
  intro ev hev
  rcases (eval_sound hP h ht (EnvOk.nil (σ := fun _ => Effs.none)) ev hev).1 with hR | hU
  · exact hR
  · obtain ⟨_, _, hn⟩ := hU
    exact False.elim hn

/-- Every operation a whole program performs is allowed by every ceiling of
the closure calls it runs inside. -/
theorem program_ceilings_sound {P Φ s t b R U} (hP : ProgramOk P Φ)
    (h : Eval P [] [] s t b) (ht : Typed P Φ [] s R U) :
    ∀ ev, ev ∈ t → ∀ c, c ∈ ev.ceilings → c.allows ev.effect := by
  intro ev hev c hc
  rcases (eval_sound hP h ht (EnvOk.nil (σ := fun _ => Effs.none)) ev hev).2 c hc with hnil | hall
  · cases hnil
  · exact hall

/-! ## Confirmation -/

/-- Every operation in a statement, closure bodies included, carries the
confirmation it needs: the checker's rule for each operation. -/
inductive Confirmed : Stmt → Prop
  | op {o} : o.confirmed = true → Confirmed (.op o)
  | seq {s₁ s₂} : Confirmed s₁ → Confirmed s₂ → Confirmed (.seq s₁ s₂)
  | choice {s₁ s₂} : Confirmed s₁ → Confirmed s₂ → Confirmed (.choice s₁ s₂)
  | letClosure {x k ps body rest} : Confirmed body → Confirmed rest →
      Confirmed (.letClosure x k ps body rest)
  | letAlias {x y rest} : Confirmed rest → Confirmed (.letAlias x y rest)
  | letOpaque {x k ps body rest} : Confirmed body → Confirmed rest →
      Confirmed (.letOpaque x k ps body rest)
  | callClosure {x args pks} : Confirmed (.callClosure x args pks)
  | callFun {f args inst} : Confirmed (.callFun f args inst)
  | loop {body} : Confirmed body → Confirmed (.loop body)
  | stop : Confirmed .stop
  | letVal {x ex rest} : Confirmed rest → Confirmed (.letVal x ex rest)
  | tryOk {x ty y rest} : Confirmed rest → Confirmed (.tryOk x ty y rest)
  | matchTag {y tag z s₁ s₂} : Confirmed s₁ → Confirmed s₂ → Confirmed (.matchTag y tag z s₁ s₂)
  | noMatch {y} : Confirmed (.noMatch y)

mutual
inductive ValConfirmed : Val → Prop
  | closure {ps body env ks} : Confirmed body → EnvConfirmed env →
      ValConfirmed (.closure ps body env ks)
  | pool {p} : ValConfirmed (.pool p)
  | base {b} : ValConfirmed (.base b)
  | ok {v} : ValConfirmed v → ValConfirmed (.ok v)
  | err {v} : ValConfirmed v → ValConfirmed (.err v)
  | variant {e tag b} : ValConfirmed (.variant e tag b)

inductive EnvConfirmed : Env → Prop
  | nil : EnvConfirmed []
  | cons {x v ρ} : ValConfirmed v → EnvConfirmed ρ → EnvConfirmed ((x, v) :: ρ)
end

theorem EnvConfirmed.find {x v} : ∀ {ρ}, EnvConfirmed ρ → NucleScript.lookup ρ x = some v →
    ValConfirmed v := by
  intro ρ h hl
  induction ρ with
  | nil => simp [NucleScript.lookup] at hl
  | cons yw ρ ih =>
    obtain ⟨y, w⟩ := yw
    cases h with
    | cons hw hrest =>
      by_cases hyx : y = x
      · subst hyx
        simp [NucleScript.lookup] at hl
        subst hl
        exact hw
      · simp [NucleScript.lookup, hyx] at hl
        exact ih hrest hl

theorem EnvConfirmed.append {ρ₂} (h₂ : EnvConfirmed ρ₂) :
    ∀ {ρ₁}, EnvConfirmed ρ₁ → EnvConfirmed (ρ₁ ++ ρ₂) := by
  intro ρ₁
  induction ρ₁ with
  | nil => intro _; exact h₂
  | cons xv ρ ih =>
    intro h₁
    cases h₁ with
    | cons hv hr => exact EnvConfirmed.cons hv (ih hr)

theorem ValConfirmed.underCeiling {v} (h : ValConfirmed v) (k : Option Ceiling) :
    ValConfirmed (v.underCeiling k) := by
  cases k with
  | none => exact h
  | some c =>
    cases h with
    | closure hb he => exact ValConfirmed.closure hb he
    | pool => exact ValConfirmed.pool
    | base => exact ValConfirmed.base
    | ok hv => exact ValConfirmed.ok hv
    | err hv => exact ValConfirmed.err hv
    | variant => exact ValConfirmed.variant

theorem ValConfirmed.payload {v tag pv} (h : ValConfirmed v) (ht : v.tagOf = some (tag, pv)) :
    ValConfirmed pv := by
  cases v with
  | closure => simp [Val.tagOf] at ht
  | pool => simp [Val.tagOf] at ht
  | base => simp [Val.tagOf] at ht
  | ok w =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    cases h with
    | ok hw => exact hw
  | err w =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    cases h with
    | err hw => exact hw
  | variant e tag' b =>
    simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
    obtain ⟨_, rfl⟩ := ht
    exact ValConfirmed.base

theorem evalV_confirmed {ex v} (h : EvalV ex v) : ValConfirmed v := by
  cases h with
  | pool => exact ValConfirmed.pool
  | outcomeOk => exact ValConfirmed.ok ValConfirmed.base
  | outcomeErr => exact ValConfirmed.err ValConfirmed.base
  | variant => exact ValConfirmed.variant

theorem bindArgs_confirmed {ρ} (h : EnvConfirmed ρ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → EnvConfirmed ρf := by
  intro ps as ρf hb
  induction hb with
  | nil => exact EnvConfirmed.nil
  | @cons p k a ps as v ρf hl _ ih => exact EnvConfirmed.cons ((h.find hl).underCeiling k) ih

def ProgramConfirmed (P : Program) : Prop := ∀ f d, P f = some d → Confirmed d.body

theorem eval_confirmed {P} (hP : ProgramConfirmed P) :
    ∀ {ρ cs s t b}, Eval P ρ cs s t b → Confirmed s → EnvConfirmed ρ →
    ∀ ev, ev ∈ t → ev.confirmed = true := by
  intro ρ cs s t b h
  induction h with
  | stop => intro _ _ ev hev; cases hev
  | seqStop _ ih =>
    intro hc henv ev hev
    cases hc with
    | seq h₁ _ => exact ih h₁ henv ev hev
  | loopDone => intro _ _ ev hev; cases hev
  | loopStep _ _ ih₁ ih₂ =>
    intro hc henv ev hev
    rcases List.mem_append.mp hev with hev | hev
    · cases hc with
      | loop hb => exact ih₁ hb henv ev hev
    · exact ih₂ hc henv ev hev
  | loopStop _ ih =>
    intro hc henv ev hev
    cases hc with
    | loop hb => exact ih hb henv ev hev
  | opEffect _ =>
    intro hc _ ev hev
    rw [List.mem_singleton] at hev
    subst hev
    cases hc with
    | op h => exact h
  | opPure _ => intro _ _ ev hev; cases hev
  | seq _ _ ih₁ ih₂ =>
    intro hc henv ev hev
    cases hc with
    | seq h₁ h₂ =>
      rcases List.mem_append.mp hev with hev | hev
      · exact ih₁ h₁ henv ev hev
      · exact ih₂ h₂ henv ev hev
  | choiceLeft _ ih =>
    intro hc henv ev hev
    cases hc with
    | choice h₁ _ => exact ih h₁ henv ev hev
  | choiceRight _ ih =>
    intro hc henv ev hev
    cases hc with
    | choice _ h₂ => exact ih h₂ henv ev hev
  | letClosure _ ih =>
    intro hc henv ev hev
    cases hc with
    | letClosure hb hr =>
      exact ih hr (EnvConfirmed.cons (ValConfirmed.closure hb henv) henv) ev hev
  | letAlias hl _ ih =>
    intro hc henv ev hev
    cases hc with
    | letAlias hr => exact ih hr (EnvConfirmed.cons (henv.find hl) henv) ev hev
  | letOpaque _ ih =>
    intro hc henv ev hev
    cases hc with
    | letOpaque hb hr =>
      exact ih hr (EnvConfirmed.cons (ValConfirmed.closure hb henv) henv) ev hev
  | callClosure hl _ _ hbind _ ih =>
    intro _ henv ev hev
    have hv := henv.find hl
    cases hv with
    | closure hb he => exact ih hb (EnvConfirmed.append he (bindArgs_confirmed henv hbind)) ev hev
  | @callFun ρ cs f args inst d ρf t b hf _ hbind _ ih =>
    intro _ henv ev hev
    exact ih (hP f d hf) (bindArgs_confirmed henv hbind) ev hev
  | letVal hv _ ih =>
    intro hc henv ev hev
    cases hc with
    | letVal hr => exact ih hr (EnvConfirmed.cons (evalV_confirmed hv) henv) ev hev
  | tryOk hl _ _ ih =>
    intro hc henv ev hev
    cases hc with
    | tryOk hr =>
      have hv := henv.find hl
      cases hv with
      | ok hw => exact ih hr (EnvConfirmed.cons hw henv) ev hev
  | tryErr _ => intro _ _ ev hev; cases hev
  | matchThen hl htag _ ih =>
    intro hc henv ev hev
    cases hc with
    | matchTag h₁ _ => exact ih h₁ (EnvConfirmed.cons ((henv.find hl).payload htag) henv) ev hev
  | matchElse hl _ _ _ ih =>
    intro hc henv ev hev
    cases hc with
    | matchTag _ h₂ => exact ih h₂ (EnvConfirmed.cons (henv.find hl) henv) ev hev

/-- A whole program whose operations and function bodies all carry their
confirmations performs only confirmed operations. -/
theorem program_confirmed {P s t b} (hP : ProgramConfirmed P) (h : Eval P [] [] s t b)
    (hc : Confirmed s) : ∀ ev, ev ∈ t → ev.confirmed = true :=
  eval_confirmed hP h hc EnvConfirmed.nil

/-! ## Value shapes

A program is shaped when every call passes its closure or function values
of the types its parameters want, every `let` binds a value of the type its
name gets, `?` meets only a `Result`, and a `match` meets only an enum or a
`Result`, with an arm for every variant it may be. Run where every name
holds a value of its type, a shaped program never reaches a call it cannot
make, a declared type its value does not conform to, a `?` on anything but
a `Result`, or a `match` no arm of which fits: the fourth promise, for the
core. A generic function is checked once, with its type parameters
abstract, and the proof covers every profile a call gives them. -/

/-- Each name's type. -/
abbrev Shapes := List (Name × Ty)

/-- The declared enums: each one's variants, with the base type each
carries (`Void` for none). -/
abbrev Enums := Name → Option (List (Name × Name))

def PTy.subst (θ : Name → PTy) : PTy → PTy
  | .prof p => .prof p
  | .var v => θ v

/-- A type with each type parameter replaced by what `θ` says it stands
for. -/
def Ty.subst (θ : Name → PTy) : Ty → Ty
  | .fn pks => .fn pks
  | .pool p => .pool (p.subst θ)
  | .base b => .base b
  | .result t e rem => .result (t.subst θ) (e.subst θ) rem
  | .enum e rem => .enum e rem

/-- The profiles a call gives a generic function's type parameters. -/
def instFun (inst : List (Name × PTy)) : Name → PTy := fun v => (lookup inst v).getD (.var v)

/-- A value of type `t` may stand where `t'` is wanted: the same type, or
an enum or `Result` known to be one of fewer variants. -/
inductive TyFits : Ty → Ty → Prop
  | refl {t} : TyFits t t
  | enum {e r r'} : (∀ x, x ∈ r → x ∈ r') → TyFits (.enum e r) (.enum e r')
  | result {t t' e e' r r'} : TyFits t t' → TyFits e e' → (∀ x, x ∈ r → x ∈ r') →
      TyFits (.result t e r) (.result t' e' r')

/-- The type of a value a `let` binds. -/
def VExpr.type (En : Enums) : VExpr → Option Ty
  | .pool p => some (.pool (.prof p))
  | .outcome t => some (Ty.res (.base t) (.base "Str"))
  | .variant e tag b =>
    match En e with
    | some vs => if lookup vs tag = some b then some (.enum e (vs.map Prod.fst)) else none
    | none => none

/-- What an arm for `tag` binds, for a value of type `t`. -/
def armType (En : Enums) : Ty → Name → Option Ty
  | .enum e rem, tag =>
    if tag ∈ rem then
      match En e with
      | some vs => (lookup vs tag).map Ty.base
      | none => none
    else none
  | .result t e rem, tag =>
    if tag ∈ rem then (if tag = "Ok" then some t else if tag = "Err" then some e else none) else none
  | _, _ => none

/-- A value of type `t` known not to be variant `tag`. -/
def Ty.without (tag : Name) : Ty → Ty
  | .enum e rem => .enum e (rem.filter fun x => x ≠ tag)
  | .result t e rem => .result t e (rem.filter fun x => x ≠ tag)
  | t => t

/-- No variant is left for a value of type `t` to be. -/
def Ty.exhausted : Ty → Prop
  | .enum _ rem => rem = []
  | .result _ _ rem => rem = []
  | _ => False

/-- The values a call passes have the types its callee wants. -/
inductive ArgsShaped (Δ : Shapes) : List Ty → List Name → Prop
  | nil : ArgsShaped Δ [] []
  | cons {t ts a as t'} : lookup Δ a = some t' → TyFits t' t → ArgsShaped Δ ts as →
      ArgsShaped Δ (t :: ts) (a :: as)

/-- `Shaped P En Δ s`: in `s`, every call passes values of the types its
callee wants, every `let` gives its name the type of its value, and every
`?` and `match` meets a value it can take. A closure's own parameters take
closures that take nothing. -/
inductive Shaped (P : Program) (En : Enums) : Shapes → Stmt → Prop
  | op {Δ o} : Shaped P En Δ (.op o)
  | stop {Δ} : Shaped P En Δ .stop
  | seq {Δ s₁ s₂} : Shaped P En Δ s₁ → Shaped P En Δ s₂ → Shaped P En Δ (.seq s₁ s₂)
  | choice {Δ s₁ s₂} : Shaped P En Δ s₁ → Shaped P En Δ s₂ → Shaped P En Δ (.choice s₁ s₂)
  | loop {Δ s} : Shaped P En Δ s → Shaped P En Δ (.loop s)
  | letClosure {Δ x k ps body rest} : Shaped P En (paramShapes ps [] ++ Δ) body →
      Shaped P En ((x, .fn (ps.map Prod.snd)) :: Δ) rest → Shaped P En Δ (.letClosure x k ps body rest)
  | letOpaque {Δ x k ps body rest} : Shaped P En (paramShapes ps [] ++ Δ) body →
      Shaped P En ((x, .fn (ps.map Prod.snd)) :: Δ) rest → Shaped P En Δ (.letOpaque x k ps body rest)
  | letAlias {Δ x y rest t} : lookup Δ y = some t → Shaped P En ((x, t) :: Δ) rest →
      Shaped P En Δ (.letAlias x y rest)
  | callClosure {Δ x args pks} : lookup Δ x = some (.fn pks) → args.length = pks.length →
      ArgsShaped Δ (List.replicate args.length (.fn [])) args → Shaped P En Δ (.callClosure x args pks)
  | callFun {Δ f args inst d} : P f = some d →
      ArgsShaped Δ ((wantOf d.params d.ptypes).map (Ty.subst (instFun inst))) args →
      Shaped P En Δ (.callFun f args inst)
  | letVal {Δ x ex rest t} : ex.type En = some t → Shaped P En ((x, t) :: Δ) rest →
      Shaped P En Δ (.letVal x ex rest)
  | tryOk {Δ x ty y rest t e rem} : lookup Δ y = some (.result t e rem) → TyFits t ty →
      Shaped P En ((x, ty) :: Δ) rest → Shaped P En Δ (.tryOk x ty y rest)
  | matchTag {Δ y tag z s₁ s₂ t pt} : lookup Δ y = some t → armType En t tag = some pt →
      Shaped P En ((z, pt) :: Δ) s₁ → Shaped P En ((y, t.without tag) :: Δ) s₂ →
      Shaped P En Δ (.matchTag y tag z s₁ s₂)
  | noMatch {Δ y t} : lookup Δ y = some t → t.exhausted → Shaped P En Δ (.noMatch y)

/-- Every function's body is shaped, with its parameters' types. -/
def ShapeOk (P : Program) (En : Enums) : Prop :=
  ∀ f d, P f = some d → Shaped P En (paramShapes d.params d.ptypes) d.body

mutual
/-- `Has P En t v`: `v` is a value of type `t`, with no type parameter
left in `t`. A closure's body is shaped in the environment it captured,
whatever profiles that environment's type parameters stood for. -/
inductive Has (P : Program) (En : Enums) : Ty → Val → Prop
  | closure {θ Δ ps body env ks} : EnvShaped P En θ Δ env →
      Shaped P En (paramShapes ps [] ++ Δ) body →
      Has P En (.fn (ps.map Prod.snd)) (.closure ps body env ks)
  | pool {p} : Has P En (.pool (.prof p)) (.pool p)
  | base {b} : Has P En (.base b) (.base b)
  | ok {t e rem v} : "Ok" ∈ rem → Has P En t v → Has P En (.result t e rem) (.ok v)
  | err {t e rem v} : "Err" ∈ rem → Has P En e v → Has P En (.result t e rem) (.err v)
  | variant {e rem tag b vs} : En e = some vs → lookup vs tag = some b → tag ∈ rem →
      Has P En (.enum e rem) (.variant e tag b)

/-- Each name holds a value of its type, with the type parameters standing
for what `θ` says. -/
inductive EnvShaped (P : Program) (En : Enums) : (Name → PTy) → Shapes → Env → Prop
  | nil {θ} : EnvShaped P En θ [] []
  | cons {θ x t v Δ ρ} : Has P En (t.subst θ) v → EnvShaped P En θ Δ ρ →
      EnvShaped P En θ ((x, t) :: Δ) ((x, v) :: ρ)
end

/-- A run reaches something it cannot do: a call to a name with no value,
to something other than a closure, or to a closure of another type than
the call says; arguments that do not conform to their parameters' types;
an unknown function; a value that does not conform to the type declared
for it; a `?` on something other than a `Result`; a `match` on something
other than an enum or a `Result`, or one no arm of which matches. -/
inductive Stuck (P : Program) : Env → Stmt → Prop
  | callUnbound {ρ x args pks} : lookup ρ x = none → Stuck P ρ (.callClosure x args pks)
  | callNotClosure {ρ x args pks v} : lookup ρ x = some v →
      (∀ ps body ρ' ks, v ≠ .closure ps body ρ' ks) → Stuck P ρ (.callClosure x args pks)
  | callType {ρ x args pks ps body ρ' ks} : lookup ρ x = some (.closure ps body ρ' ks) →
      (ps.map Prod.snd ≠ pks ∨ args.length ≠ ps.length) → Stuck P ρ (.callClosure x args pks)
  | callArgType {ρ x args pks ps body ρ' ks} : lookup ρ x = some (.closure ps body ρ' ks) →
      ¬ ArgsConform ρ (List.replicate args.length (.fn [])) args → Stuck P ρ (.callClosure x args pks)
  | callBody {ρ x args pks ps body ρ' ks ρa} : lookup ρ x = some (.closure ps body ρ' ks) →
      ps.map Prod.snd = pks → BindArgs ρ ps args ρa → Stuck P (ρa ++ ρ') body →
      Stuck P ρ (.callClosure x args pks)
  | funUnknown {ρ f args inst} : P f = none → Stuck P ρ (.callFun f args inst)
  | funArgType {ρ f args inst d} : P f = some d → ¬ ArgsConform ρ (wantOf d.params d.ptypes) args →
      Stuck P ρ (.callFun f args inst)
  | funBody {ρ f args inst d ρf} : P f = some d → BindArgs ρ d.params args ρf → Stuck P ρf d.body →
      Stuck P ρ (.callFun f args inst)
  | aliasUnbound {ρ x y rest} : lookup ρ y = none → Stuck P ρ (.letAlias x y rest)
  | aliasRest {ρ x y rest v} : lookup ρ y = some v → Stuck P ((x, v) :: ρ) rest →
      Stuck P ρ (.letAlias x y rest)
  | closureRest {ρ x k ps body rest} : Stuck P ((x, .closure ps body ρ k.toList) :: ρ) rest →
      Stuck P ρ (.letClosure x k ps body rest)
  | opaqueRest {ρ x k ps body rest} : Stuck P ((x, .closure ps body ρ k.toList) :: ρ) rest →
      Stuck P ρ (.letOpaque x k ps body rest)
  | valRest {ρ x ex rest v} : EvalV ex v → Stuck P ((x, v) :: ρ) rest → Stuck P ρ (.letVal x ex rest)
  | tryUnbound {ρ x ty y rest} : lookup ρ y = none → Stuck P ρ (.tryOk x ty y rest)
  | tryNotResult {ρ x ty y rest v} : lookup ρ y = some v → (∀ w, v ≠ .ok w) → (∀ w, v ≠ .err w) →
      Stuck P ρ (.tryOk x ty y rest)
  | tryType {ρ x ty y rest v} : lookup ρ y = some (.ok v) → ¬ Conforms ty v →
      Stuck P ρ (.tryOk x ty y rest)
  | tryRest {ρ x ty y rest v} : lookup ρ y = some (.ok v) → Stuck P ((x, v) :: ρ) rest →
      Stuck P ρ (.tryOk x ty y rest)
  | matchUnbound {ρ y tag z s₁ s₂} : lookup ρ y = none → Stuck P ρ (.matchTag y tag z s₁ s₂)
  | matchNotTagged {ρ y tag z s₁ s₂ v} : lookup ρ y = some v → v.tagOf = none →
      Stuck P ρ (.matchTag y tag z s₁ s₂)
  | matchThen {ρ y tag z s₁ s₂ v pv} : lookup ρ y = some v → v.tagOf = some (tag, pv) →
      Stuck P ((z, pv) :: ρ) s₁ → Stuck P ρ (.matchTag y tag z s₁ s₂)
  | matchElse {ρ y tag z s₁ s₂ v tag' pv} : lookup ρ y = some v → v.tagOf = some (tag', pv) →
      tag' ≠ tag → Stuck P ((y, v) :: ρ) s₂ → Stuck P ρ (.matchTag y tag z s₁ s₂)
  | noMatch {ρ y} : Stuck P ρ (.noMatch y)
  | seqLeft {ρ s₁ s₂} : Stuck P ρ s₁ → Stuck P ρ (.seq s₁ s₂)
  | seqRight {ρ cs s₁ s₂ t} : Eval P ρ cs s₁ t false → Stuck P ρ s₂ → Stuck P ρ (.seq s₁ s₂)
  | choiceLeft {ρ s₁ s₂} : Stuck P ρ s₁ → Stuck P ρ (.choice s₁ s₂)
  | choiceRight {ρ s₁ s₂} : Stuck P ρ s₂ → Stuck P ρ (.choice s₁ s₂)
  | loopBody {ρ body} : Stuck P ρ body → Stuck P ρ (.loop body)
  | loopAgain {ρ cs body t} : Eval P ρ cs body t false → Stuck P ρ (.loop body) →
      Stuck P ρ (.loop body)

/-! ### Lemmas about types -/

theorem Ty.subst_subst (ι θ : Name → PTy) : ∀ t : Ty,
    (t.subst ι).subst θ = t.subst (fun v => (ι v).subst θ)
  | .fn _ => rfl
  | .pool (.prof _) => rfl
  | .pool (.var _) => rfl
  | .base _ => rfl
  | .result t e _ => by simp only [Ty.subst, Ty.subst_subst ι θ t, Ty.subst_subst ι θ e]
  | .enum _ _ => rfl

theorem TyFits.subst {θ : Name → PTy} : ∀ {t t' : Ty}, TyFits t t' → TyFits (t.subst θ) (t'.subst θ)
  | _, _, .refl => .refl
  | _, _, .enum h => .enum h
  | _, _, .result h₁ h₂ h => .result (TyFits.subst h₁) (TyFits.subst h₂) h

theorem TyFits.fn_inv {t pks} (h : TyFits t (.fn pks)) : t = .fn pks := by
  cases h
  rfl

theorem lookup_mem_keys {α} {x : Name} {a : α} : ∀ {l : List (Name × α)},
    lookup l x = some a → x ∈ l.map Prod.fst
  | [], h => by simp [lookup] at h
  | (y, b) :: l, h => by
    by_cases hyx : y = x
    · subst hyx
      simp
    · simp only [lookup, hyx, ite_false] at h
      exact List.mem_cons_of_mem _ (lookup_mem_keys h)

theorem Has.fits {P En} : ∀ {t t'}, TyFits t t' → ∀ {v}, Has P En t v → Has P En t' v
  | _, _, .refl, _, h => h
  | _, _, .enum hr, _, h => by
    cases h with
    | variant hvs hl hin => exact Has.variant hvs hl (hr _ hin)
  | _, _, .result h₁ h₂ hr, _, h => by
    cases h with
    | ok hin hv => exact Has.ok (hr _ hin) (Has.fits h₁ hv)
    | err hin hv => exact Has.err (hr _ hin) (Has.fits h₂ hv)

/-- What a run checks follows from the type: with the type parameters
erased, a value of a type conforms to it. -/
theorem Has.conforms {P En} {θ : Name → PTy} : ∀ {t : Ty} {v : Val}, Has P En (t.subst θ) v → Conforms t v
  | .fn pks, v, h => by
    simp only [Ty.subst] at h
    cases h
    simp [Conforms]
  | .pool (.prof p), v, h => by
    simp only [Ty.subst, PTy.subst] at h
    cases h
    simp [Conforms]
  | .pool (.var w), v, h => by
    simp only [Ty.subst, PTy.subst] at h
    generalize θ w = q at h
    cases h
    simp [Conforms]
  | .base b, v, h => by
    simp only [Ty.subst] at h
    cases h
    simp [Conforms]
  | .result t e rem, v, h => by
    simp only [Ty.subst] at h
    cases h with
    | ok hin hv => exact ⟨hin, Has.conforms hv⟩
    | err hin hv => exact ⟨hin, Has.conforms hv⟩
  | .enum e rem, v, h => by
    simp only [Ty.subst] at h
    cases h with
    | variant _ _ hin => exact ⟨rfl, hin⟩

theorem Has.underCeiling {P En t v} (h : Has P En t v) (k : Option Ceiling) :
    Has P En t (v.underCeiling k) := by
  cases k with
  | none => exact h
  | some c =>
    cases h with
    | closure he hb => exact Has.closure he hb
    | pool => exact Has.pool
    | base => exact Has.base
    | ok hin hv => exact Has.ok hin hv
    | err hin hv => exact Has.err hin hv
    | variant hvs hl hin => exact Has.variant hvs hl hin

theorem Has.closure_inv {P En pks v} (h : Has P En (.fn pks) v) :
    ∃ ps body env ks, v = .closure ps body env ks ∧ ps.map Prod.snd = pks ∧
      ∃ θ Δ, EnvShaped P En θ Δ env ∧ Shaped P En (paramShapes ps [] ++ Δ) body := by
  cases h with
  | closure he hb => exact ⟨_, _, _, _, rfl, rfl, _, _, he, hb⟩

/-- What a matching arm binds has the type the arm gives it. -/
theorem Has.payload {P En} {θ : Name → PTy} {t pt : Ty} {v pv : Val} {tag : Name} (h : Has P En (t.subst θ) v)
    (ht : v.tagOf = some (tag, pv)) (ha : armType En t tag = some pt) : Has P En (pt.subst θ) pv := by
  cases t with
  | enum e rem =>
    simp only [Ty.subst] at h
    cases h with
    | @variant _ _ tag' b vs hvs hl hin =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      simp only [armType, hin, ite_true, hvs, hl, Option.map_some, Option.some.injEq] at ha
      subst ha
      exact Has.base
  | result t e rem =>
    simp only [Ty.subst] at h
    cases h with
    | ok hin hv =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      simp only [armType, hin, ite_true, Option.some.injEq] at ha
      subst ha
      exact hv
    | err hin hv =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      have harm : armType En (.result t e rem) "Err" = some e := by
        unfold armType
        simp only [hin, ↓reduceIte]
        rfl
      rw [harm] at ha
      cases ha
      exact hv
  | fn _ => simp [armType] at ha
  | pool _ => simp [armType] at ha
  | base _ => simp [armType] at ha

/-- A value that did not match an arm is one of the variants left. -/
theorem Has.without {P En} {θ : Name → PTy} {t : Ty} {v pv : Val} {tag tag' : Name} (h : Has P En (t.subst θ) v)
    (ht : v.tagOf = some (tag', pv)) (hne : tag' ≠ tag) : Has P En ((t.without tag).subst θ) v := by
  cases t with
  | enum e rem =>
    simp only [Ty.subst] at h
    simp only [Ty.without, Ty.subst]
    cases h with
    | variant hvs hl hin =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      exact Has.variant hvs hl (List.mem_filter.mpr ⟨hin, by simp [hne]⟩)
  | result t e rem =>
    simp only [Ty.subst] at h
    simp only [Ty.without, Ty.subst]
    cases h with
    | ok hin hv =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      exact Has.ok (List.mem_filter.mpr ⟨hin, by simp [hne]⟩) hv
    | err hin hv =>
      simp only [Val.tagOf, Option.some.injEq, Prod.mk.injEq] at ht
      obtain ⟨rfl, rfl⟩ := ht
      exact Has.err (List.mem_filter.mpr ⟨hin, by simp [hne]⟩) hv
  | fn _ => exact h
  | pool _ => exact h
  | base _ => exact h

theorem Has.not_exhausted {P En} {θ : Name → PTy} {t : Ty} {v : Val} (h : Has P En (t.subst θ) v) (hx : t.exhausted) : False := by
  cases t with
  | enum e rem =>
    simp only [Ty.exhausted] at hx
    subst hx
    simp only [Ty.subst] at h
    cases h with
    | variant _ _ hin => cases hin
  | result t e rem =>
    simp only [Ty.exhausted] at hx
    subst hx
    simp only [Ty.subst] at h
    cases h with
    | ok hin _ => cases hin
    | err hin _ => cases hin
  | fn _ => exact hx
  | pool _ => exact hx
  | base _ => exact hx

/-- A value of a type a `match` arm can take is a variant. -/
theorem Has.tagged {P En} {θ : Name → PTy} {t pt : Ty} {v : Val} {tag : Name} (h : Has P En (t.subst θ) v) (ha : armType En t tag = some pt) :
    v.tagOf ≠ none := by
  cases t with
  | enum e rem =>
    simp only [Ty.subst] at h
    cases h
    simp [Val.tagOf]
  | result t e rem =>
    simp only [Ty.subst] at h
    cases h <;> simp [Val.tagOf]
  | fn _ => simp [armType] at ha
  | pool _ => simp [armType] at ha
  | base _ => simp [armType] at ha

theorem Has.result_inv {P En t e rem v} (h : Has P En (.result t e rem) v) :
    (∃ w, v = .ok w ∧ Has P En t w) ∨ (∃ w, v = .err w) := by
  cases h with
  | ok _ hv => exact Or.inl ⟨_, rfl, hv⟩
  | err _ _ => exact Or.inr ⟨_, rfl⟩

theorem evalV_has {P En} {θ : Name → PTy} {ex : VExpr} {v : Val} {t : Ty} (h : EvalV ex v) (ht : ex.type En = some t) : Has P En (t.subst θ) v := by
  cases h with
  | pool =>
    simp only [VExpr.type, Option.some.injEq] at ht
    subst ht
    exact Has.pool
  | outcomeOk =>
    simp only [VExpr.type, Option.some.injEq] at ht
    subst ht
    exact Has.ok (by decide) Has.base
  | outcomeErr =>
    simp only [VExpr.type, Option.some.injEq] at ht
    subst ht
    exact Has.err (by decide) Has.base
  | @variant e tag b =>
    simp only [VExpr.type] at ht
    split at ht
    · rename_i vs hvs
      split at ht
      · rename_i hl
        simp only [Option.some.injEq] at ht
        subst ht
        exact Has.variant hvs hl (lookup_mem_keys hl)
      · cases ht
    · cases ht

/-! ### Lemmas about environments -/

theorem EnvShaped.find {P En θ x} : ∀ {Δ ρ}, EnvShaped P En θ Δ ρ → ∀ {t}, lookup Δ x = some t →
    ∃ v, lookup ρ x = some v ∧ Has P En (t.subst θ) v := by
  intro Δ ρ h
  induction ρ generalizing Δ with
  | nil =>
    cases h
    intro t ht
    simp [lookup] at ht
  | cons yv ρ ih =>
    obtain ⟨y, v⟩ := yv
    cases h with
    | @cons _ _ s _ Δ' _ hv hr =>
      intro t ht
      by_cases hyx : y = x
      · subst hyx
        simp [lookup] at ht
        subst ht
        exact ⟨v, by simp [lookup], hv⟩
      · simp [lookup, hyx] at ht
        obtain ⟨w, hw, hws⟩ := ih hr ht
        exact ⟨w, by simp [lookup, hyx, hw], hws⟩

theorem EnvShaped.append {P En θ Δ₂ ρ₂} (h₂ : EnvShaped P En θ Δ₂ ρ₂) :
    ∀ {ρ₁ Δ₁}, EnvShaped P En θ Δ₁ ρ₁ → EnvShaped P En θ (Δ₁ ++ Δ₂) (ρ₁ ++ ρ₂) := by
  intro ρ₁
  induction ρ₁ with
  | nil =>
    intro Δ₁ h₁
    cases h₁
    exact h₂
  | cons xv ρ ih =>
    intro Δ₁ h₁
    cases h₁ with
    | cons hv hr => exact EnvShaped.cons hv (ih hr)

/-- A run checks what a shaped call passes and finds it conforms. -/
theorem argsConform_of_shaped {P En θ Δ ρ} (h : EnvShaped P En θ Δ ρ) (ι : Name → PTy) :
    ∀ {ts as}, ArgsShaped Δ (ts.map (Ty.subst ι)) as → ArgsConform ρ ts as
  | [], [], _ => ArgsConform.nil
  | t :: ts, a :: as, ha => by
    simp only [List.map_cons] at ha
    cases ha with
    | cons hl hf hr =>
      obtain ⟨v, hv, hvs⟩ := h.find hl
      have hvt : Has P En ((t.subst ι).subst θ) v := Has.fits (TyFits.subst hf) hvs
      rw [Ty.subst_subst] at hvt
      exact ArgsConform.cons hv (Has.conforms hvt) (argsConform_of_shaped h ι hr)
  | [], _ :: _, ha => by cases ha
  | _ :: _, [], ha => by cases ha

theorem replicate_fn_subst (ι : Name → PTy) (n : Nat) :
    (List.replicate n (Ty.fn [])).map (Ty.subst ι) = List.replicate n (Ty.fn []) := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [List.replicate_succ, List.map_cons, ih, Ty.subst]

/-- A function's parameters hold values of their types, with the type
parameters standing for what the call gave them. -/
theorem bindArgs_shaped {P En θ Δ ρ} (h : EnvShaped P En θ Δ ρ) (ι : Name → PTy) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → ∀ {ts}, ArgsShaped Δ ((wantOf ps ts).map (Ty.subst ι)) as →
    EnvShaped P En (fun v => (ι v).subst θ) (paramShapes ps ts) ρf := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _; exact EnvShaped.nil
  | @cons p k a ps as v ρf hl _ ih =>
    intro ts ha
    have step : ∀ t : Ty, ArgsShaped Δ (t.subst ι :: (wantOf ps ts.tail).map (Ty.subst ι)) (a :: as) →
        Has P En (t.subst (fun v => (ι v).subst θ)) (v.underCeiling k) ∧
          ArgsShaped Δ ((wantOf ps ts.tail).map (Ty.subst ι)) as := by
      intro t hs
      cases hs with
      | cons hla hf hr =>
        obtain ⟨w, hw, hws⟩ := h.find hla
        rw [hl] at hw
        cases hw
        have hvt := Has.fits (TyFits.subst (θ := θ) hf) hws
        rw [Ty.subst_subst] at hvt
        exact ⟨hvt.underCeiling k, hr⟩
    cases ts with
    | nil =>
      simp only [wantOf, paramShapes, List.map_cons] at ha
      obtain ⟨hv, hr⟩ := step (.fn []) ha
      exact EnvShaped.cons hv (ih (ts := []) hr)
    | cons t ts =>
      simp only [wantOf, paramShapes, List.map_cons] at ha
      obtain ⟨hv, hr⟩ := step t ha
      exact EnvShaped.cons hv (ih (ts := ts) hr)

/-- A closure's own parameters hold closures that take nothing, under
whatever profiles the closure's environment gives type parameters. -/
theorem bindArgs_closure {P En θ Δ ρ} (h : EnvShaped P En θ Δ ρ) (θ' : Name → PTy) :
    ∀ {ps as ρa}, BindArgs ρ ps as ρa → ArgsShaped Δ (List.replicate as.length (.fn [])) as →
    EnvShaped P En θ' (paramShapes ps []) ρa := by
  intro ps as ρa hbind
  induction hbind with
  | nil => intro _; exact EnvShaped.nil
  | @cons p k a ps as v ρf hl _ ih =>
    intro ha
    simp only [List.length_cons, List.replicate_succ] at ha
    cases ha with
    | cons hla hf hr =>
      obtain ⟨w, hw, hws⟩ := h.find hla
      rw [hl] at hw
      cases hw
      rw [TyFits.fn_inv hf] at hws
      exact EnvShaped.cons (hws.underCeiling k) (ih hr)

/-! ### The theorem -/

/-- A shaped statement, run where every name holds a value of its type,
reaches nothing it cannot do. -/
theorem shaped_not_stuck {P En} (hP : ShapeOk P En) :
    ∀ {ρ s}, Stuck P ρ s → ∀ {Δ θ}, Shaped P En Δ s → EnvShaped P En θ Δ ρ → False := by
  intro ρ s h
  induction h with
  | callUnbound hl =>
    intro Δ θ hs he
    cases hs with
    | callClosure hx _ _ =>
      obtain ⟨v, hv, _⟩ := he.find hx
      rw [hl] at hv
      cases hv
  | callNotClosure hl hnc =>
    intro Δ θ hs he
    cases hs with
    | callClosure hx _ _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hx
      rw [hl] at hv
      cases hv
      obtain ⟨ps, body, env, ks, rfl, _⟩ := Has.closure_inv hvs
      exact hnc ps body env ks rfl
  | callType hl hne =>
    intro Δ θ hs he
    cases hs with
    | callClosure hx hlen _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hx
      rw [hl] at hv
      cases hv
      obtain ⟨_, _, _, _, heq, ht, _⟩ := Has.closure_inv hvs
      cases heq
      rcases hne with h₁ | h₂
      · exact h₁ ht
      · apply h₂
        rw [hlen, ← ht, List.length_map]
  | callArgType _ hnc =>
    intro Δ θ hs he
    cases hs with
    | callClosure _ _ hargs =>
      apply hnc
      refine argsConform_of_shaped he (fun v => .var v) ?_
      rw [replicate_fn_subst]
      exact hargs
  | @callBody ρ x args pks ps body ρ' ks ρa hl hpks hbind _ ih =>
    intro Δ θ hs he
    cases hs with
    | callClosure hx hlen hargs =>
      obtain ⟨v, hv, hvs⟩ := he.find hx
      rw [hl] at hv
      cases hv
      obtain ⟨_, _, _, _, heq, _, θ', Δ', hec, hbody⟩ := Has.closure_inv hvs
      cases heq
      exact ih hbody (EnvShaped.append hec (bindArgs_closure he θ' hbind hargs))
  | funUnknown hf =>
    intro Δ θ hs _
    cases hs with
    | callFun hf' _ =>
      rw [hf] at hf'
      cases hf'
  | funArgType hf hnc =>
    intro Δ θ hs he
    cases hs with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      exact hnc (argsConform_of_shaped he _ hargs)
  | @funBody ρ f args inst d ρf hf hbind _ ih =>
    intro Δ θ hs he
    cases hs with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      exact ih (hP f d hf) (bindArgs_shaped he _ hbind hargs)
  | aliasUnbound hl =>
    intro Δ θ hs he
    cases hs with
    | letAlias hy _ =>
      obtain ⟨v, hv, _⟩ := he.find hy
      rw [hl] at hv
      cases hv
  | aliasRest hl _ ih =>
    intro Δ θ hs he
    cases hs with
    | letAlias hy hr =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      exact ih hr (EnvShaped.cons hvs he)
  | closureRest _ ih =>
    intro Δ θ hs he
    cases hs with
    | letClosure hb hr => exact ih hr (EnvShaped.cons (Has.closure he hb) he)
  | opaqueRest _ ih =>
    intro Δ θ hs he
    cases hs with
    | letOpaque hb hr => exact ih hr (EnvShaped.cons (Has.closure he hb) he)
  | valRest hv _ ih =>
    intro Δ θ hs he
    cases hs with
    | letVal ht hr => exact ih hr (EnvShaped.cons (evalV_has hv ht) he)
  | tryUnbound hl =>
    intro Δ θ hs he
    cases hs with
    | tryOk hy _ _ =>
      obtain ⟨v, hv, _⟩ := he.find hy
      rw [hl] at hv
      cases hv
  | tryNotResult hl hok herr =>
    intro Δ θ hs he
    cases hs with
    | tryOk hy _ _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      simp only [Ty.subst] at hvs
      rcases Has.result_inv hvs with ⟨w, rfl, _⟩ | ⟨w, rfl⟩
      · exact hok w rfl
      · exact herr w rfl
  | tryType hl hnc =>
    intro Δ θ hs he
    cases hs with
    | tryOk hy hf _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      simp only [Ty.subst] at hvs
      rcases Has.result_inv hvs with ⟨w, hw, hws⟩ | ⟨w, hw⟩
      · cases hw
        exact hnc (Has.conforms (Has.fits (TyFits.subst hf) hws))
      · cases hw
  | tryRest hl _ ih =>
    intro Δ θ hs he
    cases hs with
    | tryOk hy hf hr =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      simp only [Ty.subst] at hvs
      rcases Has.result_inv hvs with ⟨w, hw, hws⟩ | ⟨w, hw⟩
      · cases hw
        exact ih hr (EnvShaped.cons (Has.fits (TyFits.subst hf) hws) he)
      · cases hw
  | matchUnbound hl =>
    intro Δ θ hs he
    cases hs with
    | matchTag hy _ _ _ =>
      obtain ⟨v, hv, _⟩ := he.find hy
      rw [hl] at hv
      cases hv
  | matchNotTagged hl hnone =>
    intro Δ θ hs he
    cases hs with
    | matchTag hy ha _ _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      exact Has.tagged hvs ha hnone
  | matchThen hl htag _ ih =>
    intro Δ θ hs he
    cases hs with
    | matchTag hy ha h₁ _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      exact ih h₁ (EnvShaped.cons (Has.payload hvs htag ha) he)
  | matchElse hl htag hne _ ih =>
    intro Δ θ hs he
    cases hs with
    | matchTag hy _ _ h₂ =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      exact ih h₂ (EnvShaped.cons (Has.without hvs htag hne) he)
  | noMatch =>
    intro Δ θ hs he
    cases hs with
    | noMatch hy hx =>
      obtain ⟨v, _, hvs⟩ := he.find hy
      exact Has.not_exhausted hvs hx
  | seqLeft _ ih =>
    intro Δ θ hs he
    cases hs with
    | seq h₁ _ => exact ih h₁ he
  | seqRight _ _ ih =>
    intro Δ θ hs he
    cases hs with
    | seq _ h₂ => exact ih h₂ he
  | choiceLeft _ ih =>
    intro Δ θ hs he
    cases hs with
    | choice h₁ _ => exact ih h₁ he
  | choiceRight _ ih =>
    intro Δ θ hs he
    cases hs with
    | choice _ h₂ => exact ih h₂ he
  | loopBody _ ih =>
    intro Δ θ hs he
    cases hs with
    | loop hb => exact ih hb he
  | loopAgain _ _ ih =>
    intro Δ θ hs he
    exact ih hs he

/-- A whole shaped program, run from nothing, reaches nothing it cannot
do. -/
theorem program_shapes_sound {P En s} (hP : ShapeOk P En) (hs : Shaped P En [] s) : ¬ Stuck P [] s :=
  fun h => shaped_not_stuck hP h hs (EnvShaped.nil (θ := fun v => .var v))

end NucleScript
