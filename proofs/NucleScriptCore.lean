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
closures as arguments of their own, without ceilings, and a call passes
them. Pools, files, `Result` and the rest add no effects of their own, so
they are left out.

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

inductive Stmt where
  | op (o : Op)
  | seq (s₁ s₂ : Stmt)
  /-- An `if` or `match`: either side may run. -/
  | choice (s₁ s₂ : Stmt)
  /-- `let x: Fn(...) -> Void [confirm k] = fn(p₁: Fn() -> Void, ...) -> Void
  { body }`, then `rest`: a closure whose own parameters `ps` take
  closures and have no ceiling. -/
  | letClosure (x : Name) (k : Option Ceiling) (ps : List Name) (body rest : Stmt)
  /-- `let x: Fn(...) -> Void = y`, then `rest`. -/
  | letAlias (x y : Name) (rest : Stmt)
  /-- A closure the checker cannot see into, such as one returned by a
  function: `body` is what it does, which the rules do not look at. -/
  | letOpaque (x : Name) (k : Option Ceiling) (ps : List Name) (body rest : Stmt)
  /-- `x(a₁, ...)`: a call to a closure, passing it closures. -/
  | callClosure (x : Name) (args : List Name)
  | callFun (f : Name) (args : List Name)
  /-- `for f in files of pool { body }`: the body runs zero or more times. -/
  | loop (body : Stmt)
  /-- A `?` meeting an `Err`: the enclosing function or closure returns
  here, and nothing after it in that function or closure runs. -/
  | stop

/-- A named function: its closure parameters, each with or without a
ceiling, and its body. Named functions capture nothing. -/
structure FunDecl where
  params : List (Name × Option Ceiling)
  body : Stmt
  /-- How many closures the closure each parameter holds takes, in order;
  zero where the list stops. Only the value-shape theorem reads it. -/
  arities : List Nat := []

abbrev Program := Name → Option FunDecl

/-- A closure's own parameters, as parameters without ceilings. -/
def unann (ps : List Name) : List (Name × Option Ceiling) := ps.map fun p => (p, none)

/-! ## Running a statement -/

/-- A closure value: its own parameters, its body, the environment it
captured, and the ceilings of the slots it was bound into. -/
inductive Val where
  | closure (ps : List Name) (body : Stmt) (env : List (Name × Val)) (ceilings : List Ceiling)

abbrev Env := List (Name × Val)

def lookup {α : Type} : List (Name × α) → Name → Option α
  | [], _ => none
  | (y, a) :: rest, x => if y = x then some a else lookup rest x

/-- A value as bound into a slot: into one with a ceiling, it takes it on. -/
def Val.underCeiling (v : Val) : Option Ceiling → Val
  | none => v
  | some c => match v with
    | .closure ps body env ks => .closure ps body env (c :: ks)

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
  parameters bound to the arguments. -/
  | callClosure {ρ cs x args ps body ρ' ks ρa t b} :
      lookup ρ x = some (.closure ps body ρ' ks) → BindArgs ρ (unann ps) args ρa →
      Eval P (ρa ++ ρ') (ks ++ cs) body t b →
      Eval P ρ cs (.callClosure x args) t false
  | callFun {ρ cs f args d ρf t b} : P f = some d → BindArgs ρ d.params args ρf →
      Eval P ρf cs d.body t b → Eval P ρ cs (.callFun f args) t false
  | loopDone {ρ cs body} : Eval P ρ cs (.loop body) [] false
  | loopStep {ρ cs body t₁ t₂ b} : Eval P ρ cs body t₁ false → Eval P ρ cs (.loop body) t₂ b →
      Eval P ρ cs (.loop body) (t₁ ++ t₂) b
  | loopStop {ρ cs body t} : Eval P ρ cs body t true → Eval P ρ cs (.loop body) t true

/-! ## The rules -/

/-- What a name in scope stands for: a closure whose effects `R`, used
parameters `U` and own parameters `ps` are known (a closure literal, a slot
with a ceiling, an untraceable closure), or a parameter without a ceiling.
`U` may name the closure's own parameters, which each call fills. -/
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

/-- A closure with effects `R` using parameters `U` fits the ceiling `k`:
it uses no parameter without a ceiling, and every effect is allowed. -/
def Fits (k : Option Ceiling) (R : Effs) (U : List Name) : Prop :=
  ∀ c, k = some c → U = [] ∧ Effs.Sub R c.allows

/-- The arguments of a call, checked against the parameters: an argument to
a parameter with a ceiling must fit it; the others add their effects. -/
inductive ArgsOk (Γ : Ctx) : List (Name × Option Ceiling) → List Name → Effs → List Name → Prop
  | nil : ArgsOk Γ [] [] Effs.none []
  | unannotated {p a ps as b R U} : lookup Γ a = some b → ArgsOk Γ ps as R U →
      ArgsOk Γ ((p, none) :: ps) (a :: as) (Effs.union b.R R) (b.outer ++ U)
  | annotated {p c a ps as b R U} : lookup Γ a = some b → b.U = [] →
      Effs.Sub b.R c.allows → ArgsOk Γ ps as R U →
      ArgsOk Γ ((p, some c) :: ps) (a :: as) R U

/-- The closures passed to a closure call: each may be run, so each adds its
effects. -/
inductive ArgsAll (Γ : Ctx) : List Name → Effs → List Name → Prop
  | nil : ArgsAll Γ [] Effs.none []
  | cons {a as b R U} : lookup Γ a = some b → ArgsAll Γ as R U →
      ArgsAll Γ (a :: as) (Effs.union b.R R) (b.outer ++ U)

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
      Typed P Φ (paramCtx (unann ps) ++ weaken ps Γ) body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known Rb Ub ps) :: Γ) rest R U →
      Typed P Φ Γ (.letClosure x k ps body rest) R U
  | letAlias {Γ x y rest b R U} : lookup Γ y = some b →
      Typed P Φ ((x, b) :: Γ) rest R U → Typed P Φ Γ (.letAlias x y rest) R U
  | letOpaque {Γ x k ps body rest Rb Ub R U} :
      Typed P Φ (paramCtx (unann ps) ++ weaken ps Γ) body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known (opaqueEffs k) (opaqueU k ps) (opaquePs k ps)) :: Γ) rest R U →
      Typed P Φ Γ (.letOpaque x k ps body rest) R U
  /-- A call may do what the closure does, and what the closures passed to
  it do if it may call them. -/
  | callClosure {Γ x args b R U} : lookup Γ x = some b → ArgsAll Γ args R U →
      Typed P Φ Γ (.callClosure x args) (Effs.union b.R (argEffs b R)) (b.outer ++ argUses b U)
  | callFun {Γ f args d R U} : P f = some d → ArgsOk Γ d.params args R U →
      Typed P Φ Γ (.callFun f args) (Effs.union (Φ f) R) U
  /-- A loop may do whatever its body does, any number of times. -/
  | loop {Γ body R U} : Typed P Φ Γ body R U → Typed P Φ Γ (.loop body) R U
  | stop {Γ} : Typed P Φ Γ .stop Effs.none []

/-- Every function's body is accepted, with its effect within its
signature. -/
def ProgramOk (P : Program) (Φ : Name → Effs) : Prop :=
  ∀ f d, P f = some d → ∃ R U, Typed P Φ (paramCtx d.params) d.body R U ∧ Effs.Sub R (Φ f)

/-! ## What a value may do -/

mutual
/-- `ValOk P Φ B o v`: running `v` performs only effects in `B`, besides
those of the closures passed to it; it calls none of those unless `o`; and
under each of its ceilings it calls none of them and does only what the
ceiling allows. -/
inductive ValOk (P : Program) (Φ : Name → Effs) : Effs → Bool → Val → Prop
  | closure {B o Γ σ ps body env ks R U} :
      Fresh ps Γ → Typed P Φ (paramCtx (unann ps) ++ Γ) body R U → EnvOk P Φ [] Γ env σ →
      Effs.Sub (Effs.union R (bigU σ (outerOf U ps))) B →
      (o = false → ∀ q, q ∈ U → q ∉ ps) →
      (∀ c, c ∈ ks → (∀ q, q ∈ U → q ∉ ps) ∧ Effs.Sub (Effs.union R (bigU σ U)) c.allows) →
      ValOk P Φ B o (.closure ps body env ks)

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
    · rw [if_pos hc] at hq
      cases b₀ with
      | known R U ps' =>
        simp only [weakB, Binding.outer, Binding.U, Binding.ps] at hq
        exact (mem_outerOf.mp hq).2 (mem_outerOf.mp hq).1
      | param p =>
        simp only [weakB, Binding.outer, Binding.U, Binding.ps] at hq
        exact (mem_outerOf.mp hq).2 (mem_outerOf.mp hq).1
    · rw [if_neg hc] at hq
      apply hc
      simp only [Bool.and_eq_true, decide_eq_true_eq]
      exact ⟨hy, List.any_eq_true.mpr ⟨q, hq, decide_eq_true hqp⟩⟩

theorem weakenB_ok {P Φ σ ps y b v} (h : ValOk P Φ (b.sem σ) b.callsOwn v) :
    ValOk P Φ ((weakenB ps y b).sem σ) (weakenB ps y b).callsOwn v := by
  unfold weakenB
  by_cases hc : (decide (y ∉ ps) && b.outer.any fun q => decide (q ∈ ps)) = true
  · rw [if_pos hc]
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
  · rw [if_neg hc]
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

theorem paramCtx_unann_names : ∀ ps : List Name, (paramCtx (unann ps)).map Prod.fst = ps
  | [] => rfl
  | p :: ps => by
    show (paramCtx ((p, none) :: unann ps)).map Prod.fst = p :: ps
    simp only [paramCtx, List.map_cons, paramCtx_unann_names ps]

/-- The callee's environment does what the arguments' bindings say, with
every parameter without a ceiling bounded by all the closures they pass. -/
theorem bindArgs_ok {P Φ Γ ρ σ} (henv : EnvOk P Φ [] Γ ρ σ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → ∀ {R U B seen}, ArgsOk Γ ps as R U →
    Effs.Sub (Effs.union R (bigU σ U)) B →
    EnvOk P Φ seen (paramCtx ps) ρf (fun _ => B) := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _ _ _ _ _; exact EnvOk.nil
  | @cons p k a ps as v ρf hl hrest ih =>
    intro R U B seen ha hB
    obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
    cases ha with
    | unannotated hlb ht =>
      rw [hlb] at hb'
      cases hb'
      refine EnvOk.cons ?_ (ih ht ?_)
      · refine hv.mono ?_ (fun h => by simp [paramBinding, Binding.callsOwn] at h)
        intro e he
        refine Or.inr ⟨p, param_mem_outer p, hB e ?_⟩
        rcases he with hR | hU
        · exact Or.inl (Or.inl hR)
        · exact Or.inr (bigU_append_left σ _ _ e hU)
      · intro e he
        apply hB e
        rcases he with hR | hU
        · exact Or.inl (Or.inr hR)
        · exact Or.inr (bigU_append_right σ _ _ e hU)
    | annotated hlb hU hR ht =>
      rw [hlb] at hb'
      cases hb'
      refine EnvOk.cons ?_ (ih ht hB)
      -- The value takes on the ceiling, which its effects fit, and it calls
      -- none of its own arguments, since its binding uses no parameter.
      have hbR := sem_of_outer_nil (σ := σ) (outer_of_U_nil hU)
      cases hv with
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

/-- A closure's own parameters, bound to the closures a call passes, each
do at most what all of those closures do. -/
theorem argsAll_bind {P Φ Γ ρ σ σ'' A} (henv : EnvOk P Φ [] Γ ρ σ) :
    ∀ {qs as ρa}, BindArgs ρ qs as ρa →
    (∀ p k, (p, k) ∈ qs → k = none ∧ Effs.Sub A (σ'' p)) →
    ∀ {R U seen}, ArgsAll Γ as R U → Effs.Sub (Effs.union R (bigU σ U)) A →
    EnvOk P Φ seen (paramCtx qs) ρa σ'' := by
  intro qs as ρa hbind
  induction hbind with
  | nil => intro _ _ _ _ _ _; exact EnvOk.nil
  | @cons p k a ps as v ρf hl hrest ih =>
    intro hqs R U seen ha hA
    obtain ⟨hk, hpA⟩ := hqs p k (by simp)
    subst hk
    cases ha with
    | cons hlb ht =>
      obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
      rw [hlb] at hb'
      cases hb'
      refine EnvOk.cons ?_ (ih (fun p' k' h => hqs p' k' (List.mem_cons_of_mem _ h)) ht ?_)
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
      refine ValOk.closure (fresh_weaken ps Γ) hb henv.weaken (Effs.Sub.refl _) (fun h => callsOwn_false h) ?_
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
      refine ValOk.closure (fresh_weaken ps Γ) hb henv.weaken ?_ ?_ ?_
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
  | @callClosure ρ cs x args ps body ρ' ks ρa t b hl hbind _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | @callClosure _ _ _ bx Ra Ua hlb hargs =>
      obtain ⟨b', hb', hv⟩ := henv.find (by simp) hl
      rw [hlb] at hb'
      cases hb'
      cases hv with
      | @closure _ _ Γc σc _ _ _ _ Rb Ub hfresh htb henvc hbound hown hceil =>
        -- The closure's own parameters hold the arguments; everything it
        -- captured keeps its meaning, since its parameters are named apart.
        obtain ⟨σ'', hin, hout⟩ : ∃ σ'' : Name → Effs,
            (∀ q, q ∈ ps → σ'' q = Effs.union Ra (bigU σ Ua)) ∧ (∀ q, q ∉ ps → σ'' q = σc q) :=
          ⟨fun q => if q ∈ ps then Effs.union Ra (bigU σ Ua) else σc q,
            fun q h => by simp [h], fun q h => by simp [h]⟩
        have henva : EnvOk P Φ [] (paramCtx (unann ps)) ρa σ'' := by
          refine argsAll_bind henv hbind ?_ hargs (Effs.Sub.refl _)
          intro p k hpk
          simp only [unann, List.mem_map] at hpk
          obtain ⟨q, hq, hqe⟩ := hpk
          cases hqe
          refine ⟨rfl, ?_⟩
          rw [hin _ hq]
          exact Effs.Sub.refl _
        have henvc' : EnvOk P Φ ps.reverse Γc ρ' σ'' :=
          (henvc.mono_seen (by simp)).congr
            (fun y b' hyb hy q hq => hout q (hfresh y b' hyb (by simpa using hy) q hq))
        have henvc'' : EnvOk P Φ (((paramCtx (unann ps)).map Prod.fst).reverse ++ []) Γc ρ' σ'' := by
          rw [paramCtx_unann_names, List.append_nil]
          exact henvc'
        obtain ⟨he, hc⟩ := ih htb (EnvOk.append henva henvc'') ev hev
        have lift : ∀ e, Effs.union Rb (bigU σc (outerOf Ub ps)) e →
            Effs.union (Effs.union bx.R (argEffs bx Ra)) (bigU σ (bx.outer ++ argUses bx Ua)) e := by
          intro e h'
          rcases hbound e h' with hR | hU
          · exact Or.inl (Or.inl hR)
          · exact Or.inr (bigU_append_left σ _ _ e hU)
        refine ⟨?_, ?_⟩
        · rcases he with hR | ⟨q, hq, hqe⟩
          · exact lift _ (Or.inl hR)
          · by_cases hqp : q ∈ ps
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
  | @callFun ρ cs f args d ρf t b hf hbind _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      obtain ⟨Rb, Ub, htb, hsig⟩ := hP f d hf
      have henvf := bindArgs_ok (seen := []) henv hbind hargs (Effs.Sub.refl _)
      obtain ⟨he, hc⟩ := ih htb henvf ev hev
      refine ⟨?_, hc⟩
      rcases he with hR | hU
      · exact Or.inl (Or.inl (hsig _ hR))
      · rcases bigU_const _ Ub _ hU with hR' | hU'
        · exact Or.inl (Or.inr hR')
        · exact Or.inr hU'

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
  | callClosure {x args} : Confirmed (.callClosure x args)
  | callFun {f args} : Confirmed (.callFun f args)
  | loop {body} : Confirmed body → Confirmed (.loop body)
  | stop : Confirmed .stop

mutual
inductive ValConfirmed : Val → Prop
  | closure {ps body env ks} : Confirmed body → EnvConfirmed env →
      ValConfirmed (.closure ps body env ks)

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

theorem bindArgs_confirmed {ρ} (h : EnvConfirmed ρ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → EnvConfirmed ρf := by
  intro ps as ρf hb
  induction hb with
  | nil => exact EnvConfirmed.nil
  | @cons p k a ps as v ρf hl _ ih =>
    refine EnvConfirmed.cons ?_ ih
    have hv := h.find hl
    cases hv with
    | closure hb he =>
      cases k with
      | none => exact ValConfirmed.closure hb he
      | some c => exact ValConfirmed.closure hb he

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
  | callClosure hl hbind _ ih =>
    intro _ henv ev hev
    have hv := henv.find hl
    cases hv with
    | closure hb he => exact ih hb (EnvConfirmed.append he (bindArgs_confirmed henv hbind)) ev hev
  | @callFun ρ cs f args d ρf t b hf hbind _ ih =>
    intro _ henv ev hev
    exact ih (hP f d hf) (bindArgs_confirmed henv hbind) ev hev

/-- A whole program whose operations and function bodies all carry their
confirmations performs only confirmed operations. -/
theorem program_confirmed {P s t b} (hP : ProgramConfirmed P) (h : Eval P [] [] s t b)
    (hc : Confirmed s) : ∀ ev, ev ∈ t → ev.confirmed = true :=
  eval_confirmed hP h hc EnvConfirmed.nil

/-! ## Value shapes

In the core every value is a closure, and its shape is how many closures it
takes. A program whose calls pass every closure the number of arguments it
takes, built only from names in scope, never reaches a call it cannot make:
the fourth promise, for the core. -/

/-- How many closures each callable name takes. -/
abbrev Shapes := List (Name × Nat)

/-- A function's parameters with the number of closures each one's closure
takes, zero where the declaration does not say. -/
def paramShapes : List (Name × Option Ceiling) → List Nat → Shapes
  | [], _ => []
  | (p, _) :: ps, [] => (p, 0) :: paramShapes ps []
  | (p, _) :: ps, n :: ns => (p, n) :: paramShapes ps ns

/-- What a call must pass each parameter. -/
def wantOf (ps : List (Name × Option Ceiling)) (ns : List Nat) : List Nat :=
  (paramShapes ps ns).map Prod.snd

/-- The closures a call passes have the shapes its callee wants. -/
inductive ArgsShaped (Δ : Shapes) : List Nat → List Name → Prop
  | nil : ArgsShaped Δ [] []
  | cons {n ns a as} : lookup Δ a = some n → ArgsShaped Δ ns as → ArgsShaped Δ (n :: ns) (a :: as)

/-- `Shaped P Δ s`: every call in `s` passes its callee the closures it
takes. A closure's own parameters take none. -/
inductive Shaped (P : Program) : Shapes → Stmt → Prop
  | op {Δ o} : Shaped P Δ (.op o)
  | stop {Δ} : Shaped P Δ .stop
  | seq {Δ s₁ s₂} : Shaped P Δ s₁ → Shaped P Δ s₂ → Shaped P Δ (.seq s₁ s₂)
  | choice {Δ s₁ s₂} : Shaped P Δ s₁ → Shaped P Δ s₂ → Shaped P Δ (.choice s₁ s₂)
  | loop {Δ s} : Shaped P Δ s → Shaped P Δ (.loop s)
  | letClosure {Δ x k ps body rest} : Shaped P (paramShapes (unann ps) [] ++ Δ) body →
      Shaped P ((x, ps.length) :: Δ) rest → Shaped P Δ (.letClosure x k ps body rest)
  | letOpaque {Δ x k ps body rest} : Shaped P (paramShapes (unann ps) [] ++ Δ) body →
      Shaped P ((x, ps.length) :: Δ) rest → Shaped P Δ (.letOpaque x k ps body rest)
  | letAlias {Δ x y rest n} : lookup Δ y = some n → Shaped P ((x, n) :: Δ) rest →
      Shaped P Δ (.letAlias x y rest)
  | callClosure {Δ x args} : lookup Δ x = some args.length →
      ArgsShaped Δ (List.replicate args.length 0) args → Shaped P Δ (.callClosure x args)
  | callFun {Δ f args d} : P f = some d → ArgsShaped Δ (wantOf d.params d.arities) args →
      Shaped P Δ (.callFun f args)

/-- Every function's body is shaped, with its parameters' shapes. -/
def ShapeOk (P : Program) : Prop :=
  ∀ f d, P f = some d → Shaped P (paramShapes d.params d.arities) d.body

mutual
/-- A closure value takes `n` closures, and its body is shaped in the
environment it captured. -/
inductive ValShaped (P : Program) : Nat → Val → Prop
  | closure {Δ ps body env ks} : EnvShaped P Δ env →
      Shaped P (paramShapes (unann ps) [] ++ Δ) body → ValShaped P ps.length (.closure ps body env ks)

/-- Each name's value has the shape the context says. -/
inductive EnvShaped (P : Program) : Shapes → Env → Prop
  | nil : EnvShaped P [] []
  | cons {x n v Δ ρ} : ValShaped P n v → EnvShaped P Δ ρ → EnvShaped P ((x, n) :: Δ) ((x, v) :: ρ)
end

/-- A run reaches a call it cannot make: a name with no value, a closure
or function given the wrong number of arguments, or an unknown function. -/
inductive Stuck (P : Program) : Env → Stmt → Prop
  | callUnbound {ρ x args} : lookup ρ x = none → Stuck P ρ (.callClosure x args)
  | callArity {ρ x args ps body ρ' ks} : lookup ρ x = some (.closure ps body ρ' ks) →
      args.length ≠ ps.length → Stuck P ρ (.callClosure x args)
  | callArgUnbound {ρ x args a} : a ∈ args → lookup ρ a = none → Stuck P ρ (.callClosure x args)
  | callBody {ρ x args ps body ρ' ks ρa} : lookup ρ x = some (.closure ps body ρ' ks) →
      BindArgs ρ (unann ps) args ρa → Stuck P (ρa ++ ρ') body → Stuck P ρ (.callClosure x args)
  | funUnknown {ρ f args} : P f = none → Stuck P ρ (.callFun f args)
  | funArity {ρ f args d} : P f = some d → args.length ≠ d.params.length → Stuck P ρ (.callFun f args)
  | funArgUnbound {ρ f args a} : a ∈ args → lookup ρ a = none → Stuck P ρ (.callFun f args)
  | funBody {ρ f args d ρf} : P f = some d → BindArgs ρ d.params args ρf → Stuck P ρf d.body →
      Stuck P ρ (.callFun f args)
  | aliasUnbound {ρ x y rest} : lookup ρ y = none → Stuck P ρ (.letAlias x y rest)
  | aliasRest {ρ x y rest v} : lookup ρ y = some v → Stuck P ((x, v) :: ρ) rest →
      Stuck P ρ (.letAlias x y rest)
  | closureRest {ρ x k ps body rest} : Stuck P ((x, .closure ps body ρ k.toList) :: ρ) rest →
      Stuck P ρ (.letClosure x k ps body rest)
  | opaqueRest {ρ x k ps body rest} : Stuck P ((x, .closure ps body ρ k.toList) :: ρ) rest →
      Stuck P ρ (.letOpaque x k ps body rest)
  | seqLeft {ρ s₁ s₂} : Stuck P ρ s₁ → Stuck P ρ (.seq s₁ s₂)
  | seqRight {ρ cs s₁ s₂ t} : Eval P ρ cs s₁ t false → Stuck P ρ s₂ → Stuck P ρ (.seq s₁ s₂)
  | choiceLeft {ρ s₁ s₂} : Stuck P ρ s₁ → Stuck P ρ (.choice s₁ s₂)
  | choiceRight {ρ s₁ s₂} : Stuck P ρ s₂ → Stuck P ρ (.choice s₁ s₂)
  | loopBody {ρ body} : Stuck P ρ body → Stuck P ρ (.loop body)
  | loopAgain {ρ cs body t} : Eval P ρ cs body t false → Stuck P ρ (.loop body) →
      Stuck P ρ (.loop body)

theorem EnvShaped.find {P x} : ∀ {Δ ρ}, EnvShaped P Δ ρ → ∀ {n}, lookup Δ x = some n →
    ∃ v, lookup ρ x = some v ∧ ValShaped P n v := by
  intro Δ ρ h
  induction ρ generalizing Δ with
  | nil =>
    cases h
    intro n hn
    simp [lookup] at hn
  | cons yv ρ ih =>
    obtain ⟨y, v⟩ := yv
    cases h with
    | @cons _ m _ Δ' _ hv hr =>
      intro n hn
      by_cases hyx : y = x
      · subst hyx
        simp [lookup] at hn
        subst hn
        exact ⟨v, by simp [lookup], hv⟩
      · simp [lookup, hyx] at hn
        obtain ⟨w, hw, hws⟩ := ih hr hn
        exact ⟨w, by simp [lookup, hyx, hw], hws⟩

theorem EnvShaped.append {P Δ₂ ρ₂} (h₂ : EnvShaped P Δ₂ ρ₂) :
    ∀ {ρ₁ Δ₁}, EnvShaped P Δ₁ ρ₁ → EnvShaped P (Δ₁ ++ Δ₂) (ρ₁ ++ ρ₂) := by
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

theorem ArgsShaped.length {Δ} : ∀ {ns as}, ArgsShaped Δ ns as → as.length = ns.length
  | _, _, .nil => rfl
  | _, _, .cons _ h => by simp [ArgsShaped.length h]

theorem ArgsShaped.mem {Δ a} : ∀ {ns as}, ArgsShaped Δ ns as → a ∈ as → ∃ n, lookup Δ a = some n
  | _, _, .nil, h => by cases h
  | _, _, .cons hl hr, h => by
    rcases List.mem_cons.mp h with rfl | h
    · exact ⟨_, hl⟩
    · exact ArgsShaped.mem hr h

theorem paramShapes_length : ∀ (ps : List (Name × Option Ceiling)) (ns : List Nat),
    (paramShapes ps ns).length = ps.length
  | [], _ => rfl
  | (_, _) :: ps, [] => by simp [paramShapes, paramShapes_length ps []]
  | (_, _) :: ps, _ :: ns => by simp [paramShapes, paramShapes_length ps ns]

theorem wantOf_length (ps : List (Name × Option Ceiling)) (ns : List Nat) :
    (wantOf ps ns).length = ps.length := by
  simp [wantOf, paramShapes_length]

theorem wantOf_unann : ∀ (ps : List Name), wantOf (unann ps) [] = List.replicate ps.length 0
  | [] => rfl
  | p :: ps => by
    have h := wantOf_unann ps
    unfold wantOf unann at h ⊢
    simp only [List.map_cons, paramShapes, List.length_cons, List.replicate_succ]
    rw [h]

theorem ValShaped.arity {P n ps body env ks} (h : ValShaped P n (.closure ps body env ks)) :
    ps.length = n := by
  cases h
  rfl

theorem ValShaped.body {P n ps body env ks} (h : ValShaped P n (.closure ps body env ks)) :
    ∃ Δ', EnvShaped P Δ' env ∧ Shaped P (paramShapes (unann ps) [] ++ Δ') body := by
  cases h with
  | closure he hb => exact ⟨_, he, hb⟩

theorem ValShaped.underCeiling {P n v} (h : ValShaped P n v) (k : Option Ceiling) :
    ValShaped P n (v.underCeiling k) := by
  cases h with
  | closure he hb =>
    cases k with
    | none => exact ValShaped.closure he hb
    | some c => exact ValShaped.closure he hb

/-- The callee's parameters hold closures of the shapes they want. -/
theorem bindArgs_shaped {P Δ ρ} (h : EnvShaped P Δ ρ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → ∀ {ns}, ArgsShaped Δ (wantOf ps ns) as →
    EnvShaped P (paramShapes ps ns) ρf := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _; exact EnvShaped.nil
  | @cons p k a ps as v ρf hl _ ih =>
    intro ns ha
    cases ns with
    | nil =>
      simp only [wantOf, paramShapes, List.map_cons] at ha
      cases ha with
      | cons hla hr =>
        obtain ⟨w, hw, hws⟩ := h.find hla
        rw [hl] at hw
        cases hw
        exact EnvShaped.cons (hws.underCeiling k) (ih (ns := []) hr)
    | cons n ns =>
      simp only [wantOf, paramShapes, List.map_cons] at ha
      cases ha with
      | cons hla hr =>
        obtain ⟨w, hw, hws⟩ := h.find hla
        rw [hl] at hw
        cases hw
        exact EnvShaped.cons (hws.underCeiling k) (ih (ns := ns) hr)

/-- A shaped statement, run where every name has the shape the context
says, never reaches a call it cannot make. -/
theorem shaped_not_stuck {P} (hP : ShapeOk P) :
    ∀ {ρ s}, Stuck P ρ s → ∀ {Δ}, Shaped P Δ s → EnvShaped P Δ ρ → False := by
  intro ρ s h
  induction h with
  | callUnbound hl =>
    intro Δ hs he
    cases hs with
    | callClosure hx _ =>
      obtain ⟨v, hv, _⟩ := he.find hx
      rw [hl] at hv
      cases hv
  | callArity hl hne =>
    intro Δ hs he
    cases hs with
    | callClosure hx _ =>
      obtain ⟨v, hv, hvs⟩ := he.find hx
      rw [hl] at hv
      cases hv
      exact hne hvs.arity.symm
  | callArgUnbound ha hl =>
    intro Δ hs he
    cases hs with
    | callClosure _ hargs =>
      obtain ⟨n, hn⟩ := hargs.mem ha
      obtain ⟨v, hv, _⟩ := he.find hn
      rw [hl] at hv
      cases hv
  | @callBody ρ x args ps body ρ' ks ρa hl hbind _ ih =>
    intro Δ hs he
    cases hs with
    | callClosure hx hargs =>
      obtain ⟨v, hv, hvs⟩ := he.find hx
      rw [hl] at hv
      cases hv
      obtain ⟨Δ', hec, hbody⟩ := hvs.body
      have hlen : args.length = ps.length := hvs.arity.symm
      have hwant : ArgsShaped Δ (wantOf (unann ps) []) args := by
        rw [wantOf_unann, ← hlen]
        exact hargs
      exact ih hbody (EnvShaped.append hec (bindArgs_shaped he hbind hwant))
  | funUnknown hf =>
    intro Δ hs _
    cases hs with
    | callFun hf' _ =>
      rw [hf] at hf'
      cases hf'
  | funArity hf hne =>
    intro Δ hs _
    cases hs with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      exact hne (by rw [hargs.length, wantOf_length])
  | funArgUnbound ha hl =>
    intro Δ hs he
    cases hs with
    | callFun _ hargs =>
      obtain ⟨n, hn⟩ := hargs.mem ha
      obtain ⟨v, hv, _⟩ := he.find hn
      rw [hl] at hv
      cases hv
  | @funBody ρ f args d ρf hf hbind _ ih =>
    intro Δ hs he
    cases hs with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      exact ih (hP f d hf) (bindArgs_shaped he hbind hargs)
  | aliasUnbound hl =>
    intro Δ hs he
    cases hs with
    | letAlias hy _ =>
      obtain ⟨v, hv, _⟩ := he.find hy
      rw [hl] at hv
      cases hv
  | aliasRest hl _ ih =>
    intro Δ hs he
    cases hs with
    | letAlias hy hr =>
      obtain ⟨v, hv, hvs⟩ := he.find hy
      rw [hl] at hv
      cases hv
      exact ih hr (EnvShaped.cons hvs he)
  | closureRest _ ih =>
    intro Δ hs he
    cases hs with
    | letClosure hb hr => exact ih hr (EnvShaped.cons (ValShaped.closure he hb) he)
  | opaqueRest _ ih =>
    intro Δ hs he
    cases hs with
    | letOpaque hb hr => exact ih hr (EnvShaped.cons (ValShaped.closure he hb) he)
  | seqLeft _ ih =>
    intro Δ hs he
    cases hs with
    | seq h₁ _ => exact ih h₁ he
  | seqRight _ _ ih =>
    intro Δ hs he
    cases hs with
    | seq _ h₂ => exact ih h₂ he
  | choiceLeft _ ih =>
    intro Δ hs he
    cases hs with
    | choice h₁ _ => exact ih h₁ he
  | choiceRight _ ih =>
    intro Δ hs he
    cases hs with
    | choice _ h₂ => exact ih h₂ he
  | loopBody _ ih =>
    intro Δ hs he
    cases hs with
    | loop hb => exact ih hb he
  | loopAgain _ _ ih =>
    intro Δ hs he
    exact ih hs he

/-- A whole shaped program, run from nothing, never reaches a call it
cannot make. -/
theorem program_shapes_sound {P s} (hP : ShapeOk P) (hs : Shaped P [] s) : ¬ Stuck P [] s :=
  fun h => shaped_not_stuck hP h hs EnvShaped.nil


end NucleScript
