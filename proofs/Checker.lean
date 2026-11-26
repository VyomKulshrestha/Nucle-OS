import NucleScriptCore

/-!
# A checker Lean can run

`check` computes, for a statement in a context, the effects `R` and the
parameters without a ceiling `U` that the rules in `NucleScriptCore` give
it, or `none` when the rules reject it. `check_sound` proves that whatever
it returns is a derivation of `Typed`, so its verdicts carry the theorems
proved there: `accept_effects_sound`, `accept_ceilings_sound` and
`accept_confirmed` restate them for a program `check`, `programOkB` and
`confirmedB` accept.

The differential test in `nucle_lang/tests/lean_differential.rs` runs this
checker (through `Main.lean`) on generated programs and compares its
verdicts with the Rust checker's.
-/

namespace NucleScript

/-! ## Effect sets that can be computed -/

/-- An effect set as three flags. -/
structure BEffs where
  syn : Bool
  seq : Bool
  des : Bool
  deriving DecidableEq, Repr, Inhabited

namespace BEffs

def none : BEffs := ⟨false, false, false⟩

def all : BEffs := ⟨true, true, true⟩

def union (a b : BEffs) : BEffs := ⟨a.syn || b.syn, a.seq || b.seq, a.des || b.des⟩

/-- Every effect in `a` is in `b`. -/
def sub (a b : BEffs) : Bool := (!a.syn || b.syn) && (!a.seq || b.seq) && (!a.des || b.des)

def toEffs (b : BEffs) : Effs
  | .synthesis => b.syn = true
  | .sequencing => b.seq = true
  | .destructive => b.des = true

def ofEff : Option Eff → BEffs
  | Option.none => BEffs.none
  | some .synthesis => ⟨true, false, false⟩
  | some .sequencing => ⟨false, true, false⟩
  | some .destructive => ⟨false, false, true⟩

theorem toEffs_none : BEffs.none.toEffs = Effs.none := by
  funext e
  cases e <;> simp [toEffs, BEffs.none, Effs.none]

theorem toEffs_all : BEffs.all.toEffs = Effs.all := by
  funext e
  cases e <;> simp [toEffs, BEffs.all, Effs.all]

theorem toEffs_union (a b : BEffs) : (a.union b).toEffs = Effs.union a.toEffs b.toEffs := by
  funext e
  cases e <;> simp [toEffs, union, Effs.union]

theorem toEffs_ofEff (o : Option Eff) : (ofEff o).toEffs = fun e => o = some e := by
  funext e
  cases o with
  | none => cases e <;> simp [toEffs, ofEff, BEffs.none]
  | some x => cases x <;> cases e <;> simp [toEffs, ofEff]

theorem sub_sound {a b : BEffs} (h : a.sub b = true) : Effs.Sub a.toEffs b.toEffs := by
  intro e he
  simp only [sub, Bool.and_eq_true, Bool.or_eq_true, Bool.not_eq_true'] at h
  obtain ⟨⟨h₁, h₂⟩, h₃⟩ := h
  cases e <;> simp only [toEffs] at he ⊢
  · rcases h₁ with h₁ | h₁
    · rw [h₁] at he; cases he
    · exact h₁
  · rcases h₂ with h₂ | h₂
    · rw [h₂] at he; cases he
    · exact h₂
  · rcases h₃ with h₃ | h₃
    · rw [h₃] at he; cases he
    · exact h₃

end BEffs

def Ceiling.ballows : Ceiling → BEffs
  | .hardware => ⟨true, true, false⟩
  | .synthesis => ⟨true, false, false⟩
  | .sequencing => ⟨false, true, false⟩
  | .physicalKey => ⟨false, false, true⟩

theorem Ceiling.ballows_toEffs (c : Ceiling) : c.ballows.toEffs = c.allows := by
  funext e
  cases c <;> cases e <;> simp [Ceiling.ballows, BEffs.toEffs, Ceiling.allows]

def opaqueB : Option Ceiling → BEffs
  | none => BEffs.all
  | some c => c.ballows

theorem opaqueB_toEffs (k : Option Ceiling) : (opaqueB k).toEffs = opaqueEffs k := by
  cases k with
  | none => exact BEffs.toEffs_all
  | some c => exact Ceiling.ballows_toEffs c

/-! ## Contexts that can be computed -/

inductive CBinding where
  | known (R : BEffs) (U : List Name)
  | param (p : Name)

def CBinding.R : CBinding → BEffs
  | .known R _ => R
  | .param _ => BEffs.none

def CBinding.U : CBinding → List Name
  | .known _ U => U
  | .param p => [p]

def CBinding.toBinding : CBinding → Binding
  | .known R U => .known R.toEffs U
  | .param p => .param p

theorem CBinding.toBinding_R (b : CBinding) : b.toBinding.R = b.R.toEffs := by
  cases b with
  | known R U => rfl
  | param p => exact BEffs.toEffs_none.symm

theorem CBinding.toBinding_U (b : CBinding) : b.toBinding.U = b.U := by
  cases b <;> rfl

abbrev CCtx := List (Name × CBinding)

def CCtx.toCtx : CCtx → Ctx
  | [] => []
  | (x, b) :: Γ => (x, b.toBinding) :: CCtx.toCtx Γ

theorem CCtx.lookup_toCtx {x : Name} {b : CBinding} :
    ∀ {Γ : CCtx}, lookup Γ x = some b → lookup (CCtx.toCtx Γ) x = some b.toBinding := by
  intro Γ h
  induction Γ with
  | nil => simp [lookup] at h
  | cons yb Γ ih =>
    obtain ⟨y, c⟩ := yb
    by_cases hyx : y = x
    · simp [lookup, hyx] at h
      subst h
      simp [CCtx.toCtx, lookup, hyx]
    · simp [lookup, hyx] at h
      simp [CCtx.toCtx, lookup, hyx, ih h]

def cparamBinding (p : Name) : Option Ceiling → CBinding
  | none => .param p
  | some c => .known c.ballows []

def cparamCtx : List (Name × Option Ceiling) → CCtx
  | [] => []
  | (p, k) :: ps => (p, cparamBinding p k) :: cparamCtx ps

theorem cparamCtx_toCtx : ∀ ps, CCtx.toCtx (cparamCtx ps) = paramCtx ps
  | [] => rfl
  | (p, none) :: ps => by
    simp [cparamCtx, CCtx.toCtx, paramCtx, cparamBinding, paramBinding, CBinding.toBinding, cparamCtx_toCtx ps]
  | (p, some c) :: ps => by
    simp [cparamCtx, CCtx.toCtx, paramCtx, cparamBinding, paramBinding, CBinding.toBinding,
      Ceiling.ballows_toEffs, cparamCtx_toCtx ps]

/-! ## The checker -/

def fitsB (k : Option Ceiling) (R : BEffs) (U : List Name) : Bool :=
  match k with
  | none => true
  | some c => U.isEmpty && R.sub c.ballows

theorem fitsB_sound {k : Option Ceiling} {R : BEffs} {U : List Name} (h : fitsB k R U = true) :
    Fits k R.toEffs U := by
  intro c hk
  subst hk
  simp only [fitsB, Bool.and_eq_true, List.isEmpty_iff] at h
  obtain ⟨hU, hR⟩ := h
  refine ⟨hU, ?_⟩
  have := BEffs.sub_sound hR
  rw [Ceiling.ballows_toEffs] at this
  exact this

def argsCheck (Γ : CCtx) : List (Name × Option Ceiling) → List Name → Option (BEffs × List Name)
  | [], [] => some (BEffs.none, [])
  | (_, none) :: ps, a :: as =>
    match lookup Γ a, argsCheck Γ ps as with
    | some b, some (R, U) => some (b.R.union R, b.U ++ U)
    | _, _ => none
  | (_, some c) :: ps, a :: as =>
    match lookup Γ a with
    | some b => if b.U.isEmpty && b.R.sub c.ballows then argsCheck Γ ps as else none
    | none => none
  | _, _ => none

def check (P : Program) (Φ : Name → BEffs) : CCtx → Stmt → Option (BEffs × List Name)
  | _, .op o => some (BEffs.ofEff o.effect, [])
  | Γ, .seq s₁ s₂ =>
    match check P Φ Γ s₁, check P Φ Γ s₂ with
    | some (R₁, U₁), some (R₂, U₂) => some (R₁.union R₂, U₁ ++ U₂)
    | _, _ => none
  | Γ, .choice s₁ s₂ =>
    match check P Φ Γ s₁, check P Φ Γ s₂ with
    | some (R₁, U₁), some (R₂, U₂) => some (R₁.union R₂, U₁ ++ U₂)
    | _, _ => none
  | Γ, .letClosure x k body rest =>
    match check P Φ Γ body with
    | some (Rb, Ub) => if fitsB k Rb Ub then check P Φ ((x, .known Rb Ub) :: Γ) rest else none
    | none => none
  | Γ, .letAlias x y rest =>
    match lookup Γ y with
    | some b => check P Φ ((x, b) :: Γ) rest
    | none => none
  | Γ, .letOpaque x k body rest =>
    match check P Φ Γ body with
    | some (Rb, Ub) => if fitsB k Rb Ub then check P Φ ((x, .known (opaqueB k) []) :: Γ) rest else none
    | none => none
  | Γ, .callClosure x =>
    match lookup Γ x with
    | some b => some (b.R, b.U)
    | none => none
  | Γ, .callFun f args =>
    match P f with
    | some d =>
      match argsCheck Γ d.params args with
      | some (R, U) => some ((Φ f).union R, U)
      | none => none
    | none => none

/-! ## The checker agrees with the rules -/

theorem argsCheck_sound {Γ : CCtx} :
    ∀ {ps as R U}, argsCheck Γ ps as = some (R, U) → ArgsOk (CCtx.toCtx Γ) ps as R.toEffs U
  | [], [], R, U, h => by
    simp [argsCheck] at h
    obtain ⟨rfl, rfl⟩ := h
    rw [BEffs.toEffs_none]
    exact ArgsOk.nil
  | (_, none) :: ps, a :: as, R, U, h => by
    simp only [argsCheck] at h
    split at h
    · rename_i b R' U' hb hrest
      simp at h
      obtain ⟨rfl, rfl⟩ := h
      have hrec := argsCheck_sound hrest
      have hl := CCtx.lookup_toCtx hb
      rw [BEffs.toEffs_union, ← CBinding.toBinding_R, ← CBinding.toBinding_U]
      exact ArgsOk.unannotated hl hrec
    · cases h
  | (_, some c) :: ps, a :: as, R, U, h => by
    simp only [argsCheck] at h
    split at h
    · rename_i b hb
      split at h
      · rename_i hfit
        simp only [Bool.and_eq_true, List.isEmpty_iff] at hfit
        obtain ⟨hU, hR⟩ := hfit
        have hsub := BEffs.sub_sound hR
        rw [Ceiling.ballows_toEffs, ← CBinding.toBinding_R] at hsub
        exact ArgsOk.annotated (CCtx.lookup_toCtx hb) (by rw [CBinding.toBinding_U]; exact hU) hsub
          (argsCheck_sound h)
      · cases h
    · cases h
  | [], _ :: _, _, _, h => by simp [argsCheck] at h
  | _ :: _, [], _, _, h => by simp [argsCheck] at h

theorem check_sound {P : Program} {Φ : Name → BEffs} :
    ∀ {s Γ R U}, check P Φ Γ s = some (R, U) →
    Typed P (fun f => (Φ f).toEffs) (CCtx.toCtx Γ) s R.toEffs U := by
  intro s
  induction s with
  | op o =>
    intro Γ R U h
    simp [check] at h
    obtain ⟨rfl, rfl⟩ := h
    have heq : (BEffs.ofEff o.effect).toEffs = o.effs := BEffs.toEffs_ofEff o.effect
    rw [heq]
    exact Typed.op
  | seq s₁ s₂ ih₁ ih₂ =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i R₁ U₁ R₂ U₂ h₁ h₂
      simp at h
      obtain ⟨rfl, rfl⟩ := h
      rw [BEffs.toEffs_union]
      exact Typed.seq (ih₁ h₁) (ih₂ h₂)
    · cases h
  | choice s₁ s₂ ih₁ ih₂ =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i R₁ U₁ R₂ U₂ h₁ h₂
      simp at h
      obtain ⟨rfl, rfl⟩ := h
      rw [BEffs.toEffs_union]
      exact Typed.choice (ih₁ h₁) (ih₂ h₂)
    · cases h
  | letClosure x k body rest ihb ihr =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i Rb Ub hb
      split at h
      · rename_i hfit
        exact Typed.letClosure (ihb hb) (fitsB_sound hfit) (ihr (Γ := (x, .known Rb Ub) :: Γ) h)
      · cases h
    · cases h
  | letAlias x y rest ih =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      exact Typed.letAlias (CCtx.lookup_toCtx hb) (ih (Γ := (x, b) :: Γ) h)
    · cases h
  | letOpaque x k body rest ihb ihr =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i Rb Ub hb
      split at h
      · rename_i hfit
        have hr := ihr (Γ := (x, .known (opaqueB k) []) :: Γ) h
        simp only [CCtx.toCtx, CBinding.toBinding, opaqueB_toEffs] at hr
        exact Typed.letOpaque (ihb hb) (fitsB_sound hfit) hr
      · cases h
    · cases h
  | callClosure x =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      simp at h
      obtain ⟨rfl, rfl⟩ := h
      have ht := Typed.callClosure (P := P) (Φ := fun f => (Φ f).toEffs) (CCtx.lookup_toCtx hb)
      rw [CBinding.toBinding_R, CBinding.toBinding_U] at ht
      exact ht
    · cases h
  | callFun f args =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i d hd
      split at h
      · rename_i R' U' hargs
        simp at h
        obtain ⟨rfl, rfl⟩ := h
        rw [BEffs.toEffs_union]
        exact Typed.callFun hd (argsCheck_sound hargs)
      · cases h
    · cases h

/-! ## Whole programs -/

/-- Every function in `fs` checks, with its body's effects within `Φ`. -/
def programOkB (P : Program) (Φ : Name → BEffs) (fs : List Name) : Bool :=
  fs.all fun f =>
    match P f with
    | some d =>
      match check P Φ (cparamCtx d.params) d.body with
      | some (R, _) => R.sub (Φ f)
      | none => false
    | none => true

theorem programOkB_sound {P : Program} {Φ : Name → BEffs} {fs : List Name}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (h : programOkB P Φ fs = true) :
    ProgramOk P (fun f => (Φ f).toEffs) := by
  intro f d hd
  have hf := List.all_eq_true.mp h f (hcover f d hd)
  simp only [hd] at hf
  split at hf
  · rename_i R U hc
    refine ⟨R.toEffs, U, ?_, BEffs.sub_sound hf⟩
    have ht := check_sound hc
    rw [cparamCtx_toCtx] at ht
    exact ht
  · cases hf

/-- The confirmation rule, as a function. -/
def confirmedB : Stmt → Bool
  | .op o => o.confirmed
  | .seq s₁ s₂ => confirmedB s₁ && confirmedB s₂
  | .choice s₁ s₂ => confirmedB s₁ && confirmedB s₂
  | .letClosure _ _ body rest => confirmedB body && confirmedB rest
  | .letAlias _ _ rest => confirmedB rest
  | .letOpaque _ _ body rest => confirmedB body && confirmedB rest
  | .callClosure _ => true
  | .callFun _ _ => true

theorem confirmedB_sound : ∀ {s}, confirmedB s = true → Confirmed s
  | .op o, h => Confirmed.op h
  | .seq s₁ s₂, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.seq (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .choice s₁ s₂, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.choice (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .letClosure _ _ body rest, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.letClosure (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .letAlias _ _ rest, h => Confirmed.letAlias (confirmedB_sound h)
  | .letOpaque _ _ body rest, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.letOpaque (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .callClosure _, _ => Confirmed.callClosure
  | .callFun _ _, _ => Confirmed.callFun

/-- Every function body in `fs` carries its confirmations. -/
def programConfirmedB (P : Program) (fs : List Name) : Bool :=
  fs.all fun f =>
    match P f with
    | some d => confirmedB d.body
    | none => true

theorem programConfirmedB_sound {P : Program} {fs : List Name}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (h : programConfirmedB P fs = true) :
    ProgramConfirmed P := by
  intro f d hd
  have hf := List.all_eq_true.mp h f (hcover f d hd)
  simp only [hd] at hf
  exact confirmedB_sound hf

/-! ## What an accepted program keeps -/

theorem accept_effects_sound {P : Program} {Φ : Name → BEffs} {fs : List Name} {s R U t}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programOkB P Φ fs = true)
    (hc : check P Φ [] s = some (R, U)) (h : Eval P [] [] s t) :
    ∀ ev, ev ∈ t → R.toEffs ev.effect :=
  program_effects_sound (programOkB_sound hcover hok) h (check_sound (Γ := []) hc)

theorem accept_ceilings_sound {P : Program} {Φ : Name → BEffs} {fs : List Name} {s R U t}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programOkB P Φ fs = true)
    (hc : check P Φ [] s = some (R, U)) (h : Eval P [] [] s t) :
    ∀ ev, ev ∈ t → ∀ c, c ∈ ev.ceilings → c.allows ev.effect :=
  program_ceilings_sound (programOkB_sound hcover hok) h (check_sound (Γ := []) hc)

theorem accept_confirmed {P : Program} {fs : List Name} {s t}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programConfirmedB P fs = true)
    (hc : confirmedB s = true) (h : Eval P [] [] s t) :
    ∀ ev, ev ∈ t → ev.confirmed = true :=
  program_confirmed (programConfirmedB_sound hcover hok) h (confirmedB_sound hc)

/-! ## Signatures -/

/-- One round of computing each function's effect from its body. -/
def sigStep (P : Program) (Φ : Name → BEffs) (f : Name) : BEffs :=
  match P f with
  | some d =>
    match check P Φ (cparamCtx d.params) d.body with
    | some (R, _) => R
    | none => BEffs.none
  | none => BEffs.none

/-- The effects of each function, by iterating from nothing. Each round
that changes anything adds at least one of three flags to one function, so
`3 * n + 1` rounds reach the fixed point the Rust checker computes; whether
it was reached is not assumed, since `programOkB` checks the result. -/
def signature (P : Program) (fs : List Name) : Name → BEffs :=
  (List.range (3 * fs.length + 1)).foldl (fun Φ _ => sigStep P Φ) (fun _ => BEffs.none)

end NucleScript
