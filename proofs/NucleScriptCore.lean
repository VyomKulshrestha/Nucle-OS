/-!
# NucleScript's effect rules, proved for a core of the language

A core of NucleScript with the parts its effect rules are about:
operations with effects and confirmations, sequencing, branches (an `if` or
a `match`: either side may run), closures bound by `let` with or without an
effect ceiling, other names for closures, closures the checker cannot see
into (returned by a function, taken out of an enum), calls to closures, and
named functions taking closure arguments, with or without ceilings. Closures
take no arguments of their own. Pools, files, `Result` and the rest add no
effects of their own (an early return through `?` only skips operations),
so they are left out.

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

/-- `confirm hardware` covers synthesis and sequencing; `confirm
physical_key` covers deletion. -/
inductive Ceiling where
  | hardware
  | physicalKey
  deriving DecidableEq, Repr

def Ceiling.allows : Ceiling → Effs
  | .hardware => fun e => e = .synthesis ∨ e = .sequencing
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
  /-- `let x: Fn() -> Void [confirm k] = fn() -> Void { body }`, then `rest`. -/
  | letClosure (x : Name) (k : Option Ceiling) (body rest : Stmt)
  /-- `let x: Fn() -> Void = y`, then `rest`. -/
  | letAlias (x y : Name) (rest : Stmt)
  /-- A closure the checker cannot see into, such as one returned by a
  function: `body` is what it does, which the rules do not look at. -/
  | letOpaque (x : Name) (k : Option Ceiling) (body rest : Stmt)
  | callClosure (x : Name)
  | callFun (f : Name) (args : List Name)

/-- A named function: its closure parameters, each with or without a
ceiling, and its body. Named functions capture nothing. -/
structure FunDecl where
  params : List (Name × Option Ceiling)
  body : Stmt

abbrev Program := Name → Option FunDecl

/-! ## Running a statement -/

/-- A closure value: its body, the environment it captured, and the ceilings
of the slots it was bound into. -/
inductive Val where
  | closure (body : Stmt) (env : List (Name × Val)) (ceilings : List Ceiling)

abbrev Env := List (Name × Val)

def lookup {α : Type} : List (Name × α) → Name → Option α
  | [], _ => none
  | (y, a) :: rest, x => if y = x then some a else lookup rest x

/-- A value as bound into a slot: into one with a ceiling, it takes it on. -/
def Val.underCeiling (v : Val) : Option Ceiling → Val
  | none => v
  | some c => match v with
    | .closure body env ks => .closure body env (c :: ks)

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

/-- `Eval P ρ cs s t`: in environment `ρ`, under the ceilings `cs` of the
closure calls in progress, `s` can run and perform the events `t`. -/
inductive Eval (P : Program) : Env → List Ceiling → Stmt → List Event → Prop
  | opEffect {ρ cs o e} : o.effect = some e →
      Eval P ρ cs (.op o) [⟨e, o.confirmed, cs⟩]
  | opPure {ρ cs o} : o.effect = none → Eval P ρ cs (.op o) []
  | seq {ρ cs s₁ s₂ t₁ t₂} : Eval P ρ cs s₁ t₁ → Eval P ρ cs s₂ t₂ →
      Eval P ρ cs (.seq s₁ s₂) (t₁ ++ t₂)
  | choiceLeft {ρ cs s₁ s₂ t} : Eval P ρ cs s₁ t → Eval P ρ cs (.choice s₁ s₂) t
  | choiceRight {ρ cs s₁ s₂ t} : Eval P ρ cs s₂ t → Eval P ρ cs (.choice s₁ s₂) t
  | letClosure {ρ cs x k body rest t} :
      Eval P ((x, .closure body ρ k.toList) :: ρ) cs rest t →
      Eval P ρ cs (.letClosure x k body rest) t
  | letAlias {ρ cs x y rest v t} : lookup ρ y = some v →
      Eval P ((x, v) :: ρ) cs rest t → Eval P ρ cs (.letAlias x y rest) t
  | letOpaque {ρ cs x k body rest t} :
      Eval P ((x, .closure body ρ k.toList) :: ρ) cs rest t →
      Eval P ρ cs (.letOpaque x k body rest) t
  | callClosure {ρ cs x body ρ' ks t} :
      lookup ρ x = some (.closure body ρ' ks) → Eval P ρ' (ks ++ cs) body t →
      Eval P ρ cs (.callClosure x) t
  | callFun {ρ cs f args d ρf t} : P f = some d → BindArgs ρ d.params args ρf →
      Eval P ρf cs d.body t → Eval P ρ cs (.callFun f args) t

/-! ## The rules -/

/-- What a name in scope stands for: a closure whose effects `R` and used
parameters `U` are known (a closure literal, a slot with a ceiling, an
untraceable closure), or a parameter without a ceiling. -/
inductive Binding where
  | known (R : Effs) (U : List Name)
  | param (p : Name)

def Binding.R : Binding → Effs
  | .known R _ => R
  | .param _ => Effs.none

def Binding.U : Binding → List Name
  | .known _ U => U
  | .param p => [p]

abbrev Ctx := List (Name × Binding)

/-- What the closures held by the parameters in `U` may do, if parameter `p`
holds closures doing at most `σ p`. -/
def bigU (σ : Name → Effs) (U : List Name) : Effs := fun e => ∃ p, p ∈ U ∧ σ p e

def Binding.sem (b : Binding) (σ : Name → Effs) : Effs := Effs.union b.R (bigU σ b.U)

/-- What a closure the rules cannot see into may do: anything, unless the
type it comes through names a ceiling. -/
def opaqueEffs : Option Ceiling → Effs
  | none => Effs.all
  | some c => c.allows

def paramBinding (p : Name) : Option Ceiling → Binding
  | none => .param p
  | some c => .known c.allows []

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
      ArgsOk Γ ((p, none) :: ps) (a :: as) (Effs.union b.R R) (b.U ++ U)
  | annotated {p c a ps as b R U} : lookup Γ a = some b → b.U = [] →
      Effs.Sub b.R c.allows → ArgsOk Γ ps as R U →
      ArgsOk Γ ((p, some c) :: ps) (a :: as) R U

/-- `Typed P Φ Γ s R U`: `s` may have the effects `R` and run the closures
of the parameters `U`. `Φ f` is the effect of calling `f`, its own body's
`R`. -/
inductive Typed (P : Program) (Φ : Name → Effs) : Ctx → Stmt → Effs → List Name → Prop
  | op {Γ o} : Typed P Φ Γ (.op o) o.effs []
  | seq {Γ s₁ s₂ R₁ R₂ U₁ U₂} : Typed P Φ Γ s₁ R₁ U₁ → Typed P Φ Γ s₂ R₂ U₂ →
      Typed P Φ Γ (.seq s₁ s₂) (Effs.union R₁ R₂) (U₁ ++ U₂)
  | choice {Γ s₁ s₂ R₁ R₂ U₁ U₂} : Typed P Φ Γ s₁ R₁ U₁ → Typed P Φ Γ s₂ R₂ U₂ →
      Typed P Φ Γ (.choice s₁ s₂) (Effs.union R₁ R₂) (U₁ ++ U₂)
  | letClosure {Γ x k body rest Rb Ub R U} :
      Typed P Φ Γ body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known Rb Ub) :: Γ) rest R U →
      Typed P Φ Γ (.letClosure x k body rest) R U
  | letAlias {Γ x y rest b R U} : lookup Γ y = some b →
      Typed P Φ ((x, b) :: Γ) rest R U → Typed P Φ Γ (.letAlias x y rest) R U
  | letOpaque {Γ x k body rest Rb Ub R U} :
      Typed P Φ Γ body Rb Ub → Fits k Rb Ub →
      Typed P Φ ((x, .known (opaqueEffs k) []) :: Γ) rest R U →
      Typed P Φ Γ (.letOpaque x k body rest) R U
  | callClosure {Γ x b} : lookup Γ x = some b → Typed P Φ Γ (.callClosure x) b.R b.U
  | callFun {Γ f args d R U} : P f = some d → ArgsOk Γ d.params args R U →
      Typed P Φ Γ (.callFun f args) (Effs.union (Φ f) R) U

/-- Every function's body is accepted, with its effect within its
signature. -/
def ProgramOk (P : Program) (Φ : Name → Effs) : Prop :=
  ∀ f d, P f = some d → ∃ R U, Typed P Φ (paramCtx d.params) d.body R U ∧ Effs.Sub R (Φ f)

/-! ## What a value may do -/

mutual
/-- `ValOk P Φ B v`: running `v` performs only effects in `B`, and only
effects each of its ceilings allows. -/
inductive ValOk (P : Program) (Φ : Name → Effs) : Effs → Val → Prop
  | closure {B Γ σ body env ks R U} :
      Typed P Φ Γ body R U → EnvOk P Φ Γ env σ →
      Effs.Sub (Effs.union R (bigU σ U)) B →
      (∀ c, c ∈ ks → Effs.Sub (Effs.union R (bigU σ U)) c.allows) →
      ValOk P Φ B (.closure body env ks)

/-- Each name's value does at most what its binding says. -/
inductive EnvOk (P : Program) (Φ : Name → Effs) : Ctx → Env → (Name → Effs) → Prop
  | nil {σ} : EnvOk P Φ [] [] σ
  | cons {x b v Γ ρ σ} : ValOk P Φ (b.sem σ) v → EnvOk P Φ Γ ρ σ →
      EnvOk P Φ ((x, b) :: Γ) ((x, v) :: ρ) σ
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

theorem ValOk.mono {P Φ B B' v} (h : ValOk P Φ B v) (hs : Effs.Sub B B') : ValOk P Φ B' v := by
  cases h with
  | closure ht henv hb hc => exact ValOk.closure ht henv (Effs.Sub.trans hb hs) hc

theorem EnvOk.find {P Φ σ x v} : ∀ {Γ ρ}, EnvOk P Φ Γ ρ σ → NucleScript.lookup ρ x = some v →
    ∃ b, NucleScript.lookup Γ x = some b ∧ ValOk P Φ (b.sem σ) v := by
  intro Γ ρ h hl
  induction ρ generalizing Γ with
  | nil => simp [NucleScript.lookup] at hl
  | cons yw ρ ih =>
    obtain ⟨y, w⟩ := yw
    cases h with
    | cons hw hrest =>
      by_cases hyx : y = x
      · subst hyx
        simp [NucleScript.lookup] at hl
        subst hl
        exact ⟨_, by simp [NucleScript.lookup], hw⟩
      · simp [NucleScript.lookup, hyx] at hl
        obtain ⟨b', hb', hv'⟩ := ih hrest hl
        exact ⟨b', by simp [NucleScript.lookup, hyx, hb'], hv'⟩

/-- The callee's environment does what the arguments' bindings say, with
every parameter without a ceiling bounded by all the closures they pass. -/
theorem bindArgs_ok {P Φ Γ ρ σ} (henv : EnvOk P Φ Γ ρ σ) :
    ∀ {ps as ρf}, BindArgs ρ ps as ρf → ∀ {R U B}, ArgsOk Γ ps as R U →
    Effs.Sub (Effs.union R (bigU σ U)) B →
    EnvOk P Φ (paramCtx ps) ρf (fun _ => B) := by
  intro ps as ρf hbind
  induction hbind with
  | nil => intro _ _ _ _ _; exact EnvOk.nil
  | @cons p k a ps as v ρf hl hrest ih =>
    intro R U B ha hB
    obtain ⟨b', hb', hv⟩ := henv.find hl
    cases ha with
    | unannotated hlb ht =>
      rw [hlb] at hb'
      cases hb'
      refine EnvOk.cons ?_ (ih ht ?_)
      · refine hv.mono ?_
        intro e he
        refine Or.inr ⟨p, List.mem_singleton.mpr rfl, hB e ?_⟩
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
      -- The value takes on the ceiling, which its effects fit.
      cases hv with
      | closure ht' henv' hb hc =>
        refine ValOk.closure ht' henv' ?_ ?_
        · intro e he
          exact Or.inl (fits_sem hU hR e (hb e he))
        · intro c' hc'
          rcases List.mem_cons.mp hc' with heq | hin
          · subst heq
            exact Effs.Sub.trans hb (fits_sem hU hR)
          · exact hc c' hin

/-! ## Soundness -/

/-- Every event a statement performs is within its rules' effects (with the
parameters' closures bounded by `σ`) and allowed by every ceiling in force
that the statement itself did not inherit. -/
theorem eval_sound {P Φ} (hP : ProgramOk P Φ) :
    ∀ {ρ cs s t}, Eval P ρ cs s t →
    ∀ {Γ R U σ}, Typed P Φ Γ s R U → EnvOk P Φ Γ ρ σ →
    ∀ ev, ev ∈ t →
      (Effs.union R (bigU σ U)) ev.effect ∧ (∀ c, c ∈ ev.ceilings → c ∈ cs ∨ c.allows ev.effect) := by
  intro ρ cs s t h
  induction h with
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
  | @letClosure ρ cs x k body rest t _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letClosure hb hfit hr =>
      refine ih hr (EnvOk.cons ?_ henv) ev hev
      refine ValOk.closure hb henv (Effs.Sub.refl _) ?_
      intro c hc
      obtain ⟨hU, hR⟩ := hfit c (mem_toList hc)
      exact fits_sem hU hR
  | letAlias hl _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letAlias hlb hr =>
      obtain ⟨b', hb', hv⟩ := henv.find hl
      rw [hlb] at hb'
      cases hb'
      exact ih hr (EnvOk.cons hv henv) ev hev
  | @letOpaque ρ cs x k body rest t _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | letOpaque hb hfit hr =>
      refine ih hr (EnvOk.cons ?_ henv) ev hev
      refine ValOk.closure hb henv ?_ ?_
      · intro e he
        cases k with
        | none => exact Or.inl trivial
        | some c =>
          obtain ⟨hU, hR⟩ := hfit c rfl
          exact Or.inl (fits_sem hU hR e he)
      · intro c hc
        obtain ⟨hU, hR⟩ := hfit c (mem_toList hc)
        exact fits_sem hU hR
  | callClosure hl _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | callClosure hlb =>
      obtain ⟨b', hb', hv⟩ := henv.find hl
      rw [hlb] at hb'
      cases hb'
      cases hv with
      | closure htb henvb hbound hceil =>
        obtain ⟨he, hc⟩ := ih htb henvb ev hev
        refine ⟨hbound _ he, ?_⟩
        intro c hcin
        rcases hc c hcin with hks | hall
        · rcases List.mem_append.mp hks with hk | hcs
          · exact Or.inr (hceil c hk _ he)
          · exact Or.inl hcs
        · exact Or.inr hall
  | @callFun ρ cs f args d ρf t hf hbind _ ih =>
    intro Γ R U σ ht henv ev hev
    cases ht with
    | callFun hf' hargs =>
      rw [hf] at hf'
      cases hf'
      obtain ⟨Rb, Ub, htb, hsig⟩ := hP f d hf
      have henvf := bindArgs_ok henv hbind hargs (Effs.Sub.refl _)
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
theorem program_effects_sound {P Φ s t R U} (hP : ProgramOk P Φ)
    (h : Eval P [] [] s t) (ht : Typed P Φ [] s R U) :
    ∀ ev, ev ∈ t → R ev.effect := by
  intro ev hev
  rcases (eval_sound hP h ht (EnvOk.nil (σ := fun _ => Effs.none)) ev hev).1 with hR | hU
  · exact hR
  · obtain ⟨_, _, hn⟩ := hU
    exact False.elim hn

/-- Every operation a whole program performs is allowed by every ceiling of
the closure calls it runs inside. -/
theorem program_ceilings_sound {P Φ s t R U} (hP : ProgramOk P Φ)
    (h : Eval P [] [] s t) (ht : Typed P Φ [] s R U) :
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
  | letClosure {x k body rest} : Confirmed body → Confirmed rest →
      Confirmed (.letClosure x k body rest)
  | letAlias {x y rest} : Confirmed rest → Confirmed (.letAlias x y rest)
  | letOpaque {x k body rest} : Confirmed body → Confirmed rest →
      Confirmed (.letOpaque x k body rest)
  | callClosure {x} : Confirmed (.callClosure x)
  | callFun {f args} : Confirmed (.callFun f args)

mutual
inductive ValConfirmed : Val → Prop
  | closure {body env ks} : Confirmed body → EnvConfirmed env →
      ValConfirmed (.closure body env ks)

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
    ∀ {ρ cs s t}, Eval P ρ cs s t → Confirmed s → EnvConfirmed ρ →
    ∀ ev, ev ∈ t → ev.confirmed = true := by
  intro ρ cs s t h
  induction h with
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
  | callClosure hl _ ih =>
    intro _ henv ev hev
    have hv := henv.find hl
    cases hv with
    | closure hb he => exact ih hb he ev hev
  | @callFun ρ cs f args d ρf t hf hbind _ ih =>
    intro _ henv ev hev
    exact ih (hP f d hf) (bindArgs_confirmed henv hbind) ev hev

/-- A whole program whose operations and function bodies all carry their
confirmations performs only confirmed operations. -/
theorem program_confirmed {P s t} (hP : ProgramConfirmed P) (h : Eval P [] [] s t)
    (hc : Confirmed s) : ∀ ev, ev ∈ t → ev.confirmed = true :=
  eval_confirmed hP h hc EnvConfirmed.nil

end NucleScript
