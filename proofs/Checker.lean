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
  | known (R : BEffs) (U : List Name) (ps : List Name)
  | param (p : Name)

def CBinding.R : CBinding → BEffs
  | .known R _ _ => R
  | .param _ => BEffs.none

def CBinding.U : CBinding → List Name
  | .known _ U _ => U
  | .param p => [p]

def CBinding.ps : CBinding → List Name
  | .known _ _ ps => ps
  | .param _ => []

def CBinding.outer (b : CBinding) : List Name := outerOf b.U b.ps

def CBinding.mayCallArgs : CBinding → Bool
  | .known _ _ ps => !ps.isEmpty
  | .param _ => true

def CBinding.addsAll : CBinding → Bool
  | .known _ _ _ => false
  | .param _ => true

def CBinding.toBinding : CBinding → Binding
  | .known R U ps => .known R.toEffs U ps
  | .param p => .param p

theorem CBinding.toBinding_R (b : CBinding) : b.toBinding.R = b.R.toEffs := by
  cases b with
  | known R U ps => rfl
  | param p => exact BEffs.toEffs_none.symm

theorem CBinding.toBinding_U (b : CBinding) : b.toBinding.U = b.U := by
  cases b <;> rfl

theorem CBinding.toBinding_outer (b : CBinding) : b.toBinding.outer = b.outer := by
  cases b <;> rfl

theorem CBinding.toBinding_mayCallArgs (b : CBinding) : b.toBinding.mayCallArgs = b.mayCallArgs := by
  cases b <;> rfl

theorem CBinding.toBinding_addsAll (b : CBinding) : b.toBinding.addsAll = b.addsAll := by
  cases b <;> rfl

abbrev CCtx := List (Name × CBinding)

def CCtx.toCtx : CCtx → Ctx
  | [] => []
  | (x, b) :: Γ => (x, b.toBinding) :: CCtx.toCtx Γ

theorem CCtx.toCtx_append : ∀ (Γ₁ Γ₂ : CCtx), CCtx.toCtx (Γ₁ ++ Γ₂) = CCtx.toCtx Γ₁ ++ CCtx.toCtx Γ₂
  | [], _ => rfl
  | (x, b) :: Γ₁, Γ₂ => by simp [CCtx.toCtx, CCtx.toCtx_append Γ₁ Γ₂]

theorem CCtx.mem_toCtx {y : Name} {b : Binding} : ∀ {Γ : CCtx}, (y, b) ∈ CCtx.toCtx Γ →
    ∃ c, (y, c) ∈ Γ ∧ b = c.toBinding
  | [], h => by simp [CCtx.toCtx] at h
  | (x, c) :: Γ, h => by
    simp only [CCtx.toCtx, List.mem_cons, Prod.mk.injEq] at h
    rcases h with ⟨rfl, rfl⟩ | h
    · exact ⟨c, by simp, rfl⟩
    · obtain ⟨c', hc', rfl⟩ := CCtx.mem_toCtx h
      exact ⟨c', List.mem_cons_of_mem _ hc', rfl⟩

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
  | some c => .known c.ballows [] []

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

def CBinding.weak : CBinding → CBinding
  | .known _ _ ps => .known BEffs.all ps ps
  | .param _ => .known BEffs.all ["*"] ["*"]

/-- `weakenB`, computed. -/
def cweakenB (ps : List Name) (y : Name) (b : CBinding) : CBinding :=
  if (decide (y ∉ ps) && b.outer.any fun q => decide (q ∈ ps)) = true then b.weak else b

def cweaken (ps : List Name) : CCtx → CCtx
  | [] => []
  | (y, b) :: Γ => (y, cweakenB ps y b) :: cweaken ps Γ

theorem toBinding_cweakenB (ps : List Name) (y : Name) (b : CBinding) :
    (cweakenB ps y b).toBinding = weakenB ps y b.toBinding := by
  unfold cweakenB weakenB
  rw [CBinding.toBinding_outer]
  by_cases hc : (decide (y ∉ ps) && b.outer.any fun q => decide (q ∈ ps)) = true
  · simp only [hc, ↓reduceIte]
    cases b <;> simp [CBinding.weak, weakB, CBinding.toBinding, BEffs.toEffs_all]
  · simp only [hc, Bool.false_eq_true, ↓reduceIte]

theorem toCtx_cweaken (ps : List Name) : ∀ Γ : CCtx, CCtx.toCtx (cweaken ps Γ) = weaken ps (CCtx.toCtx Γ)
  | [] => rfl
  | (y, b) :: Γ => by
    simp only [cweaken, weaken, CCtx.toCtx, toBinding_cweakenB, toCtx_cweaken ps Γ]

/-- `ArgsOk`, computed: the arguments of a call against the ceilings of
the parameters they fill. -/
def argsCheck (Γ : CCtx) (add : Bool) : List (Option Ceiling) → List Name → Option (BEffs × List Name)
  | [], [] => some (BEffs.none, [])
  | none :: ks, a :: as =>
    match lookup Γ a, argsCheck Γ add ks as with
    | some b, some (R, U) => some (b.R.union R, b.outer ++ U)
    | _, _ => none
  | some c :: ks, a :: as =>
    match lookup Γ a, argsCheck Γ add ks as with
    | some b, some (R, U) =>
      if b.U.isEmpty && b.R.sub c.ballows then some (if add then b.R.union R else R, U) else none
    | _, _ => none
  | _, _ => none

def check (P : Program) (Φ : Name → BEffs) : CCtx → Stmt → Option (BEffs × List Name)
  | _, .op o => some (BEffs.ofEff o.effect, [])
  | _, .stop => some (BEffs.none, [])
  | Γ, .loop body => check P Φ Γ body
  | Γ, .seq s₁ s₂ =>
    match check P Φ Γ s₁, check P Φ Γ s₂ with
    | some (R₁, U₁), some (R₂, U₂) => some (R₁.union R₂, U₁ ++ U₂)
    | _, _ => none
  | Γ, .choice s₁ s₂ =>
    match check P Φ Γ s₁, check P Φ Γ s₂ with
    | some (R₁, U₁), some (R₂, U₂) => some (R₁.union R₂, U₁ ++ U₂)
    | _, _ => none
  | Γ, .letClosure x k ps body rest =>
    match check P Φ (cparamCtx ps ++ cweaken (names ps) Γ) body with
    | some (Rb, Ub) => if fitsB k Rb Ub then check P Φ ((x, .known Rb Ub (names ps)) :: Γ) rest else none
    | none => none
  | Γ, .letAlias x y rest =>
    match lookup Γ y with
    | some b => check P Φ ((x, b) :: Γ) rest
    | none => none
  | Γ, .letOpaque x k ps body rest =>
    match check P Φ (cparamCtx ps ++ cweaken (names ps) Γ) body with
    | some (Rb, Ub) =>
      if fitsB k Rb Ub then
        check P Φ ((x, .known (opaqueB k) (opaqueU k (names ps)) (opaquePs k (names ps))) :: Γ) rest
      else none
    | none => none
  | Γ, .callClosure x args pks =>
    match lookup Γ x with
    | some b =>
      match argsCheck Γ b.addsAll pks args with
      | some (R, U) =>
        if b.mayCallArgs then some (b.R.union R, b.outer ++ U) else some (b.R.union BEffs.none, b.outer ++ [])
      | none => none
    | none => none
  | Γ, .callFun f args =>
    match P f with
    | some d =>
      match argsCheck Γ false (d.params.map Prod.snd) args with
      | some (R, U) => some ((Φ f).union R, U)
      | none => none
    | none => none

/-! ## The checker agrees with the rules -/

theorem toEffs_addIf (add : Bool) (A R : BEffs) :
    (if add then A.union R else R).toEffs = addIf add A.toEffs R.toEffs := by
  cases add <;> simp [addIf, BEffs.toEffs_union]

theorem argsCheck_sound {Γ : CCtx} {add : Bool} :
    ∀ {ks as R U}, argsCheck Γ add ks as = some (R, U) → ArgsOk (CCtx.toCtx Γ) add ks as R.toEffs U
  | [], [], R, U, h => by
    simp [argsCheck] at h
    obtain ⟨rfl, rfl⟩ := h
    rw [BEffs.toEffs_none]
    exact ArgsOk.nil
  | none :: ks, a :: as, R, U, h => by
    simp only [argsCheck] at h
    split at h
    · rename_i b R' U' hb hrest
      simp at h
      obtain ⟨rfl, rfl⟩ := h
      have hrec := argsCheck_sound hrest
      have hl := CCtx.lookup_toCtx hb
      rw [BEffs.toEffs_union, ← CBinding.toBinding_R, ← CBinding.toBinding_outer]
      exact ArgsOk.unannotated hl hrec
    · cases h
  | some c :: ks, a :: as, R, U, h => by
    simp only [argsCheck] at h
    split at h
    · rename_i b R' U' hb hrest
      split at h
      · rename_i hfit
        simp only [Option.some.injEq, Prod.mk.injEq] at h
        obtain ⟨rfl, rfl⟩ := h
        simp only [Bool.and_eq_true, List.isEmpty_iff] at hfit
        obtain ⟨hU, hR⟩ := hfit
        have hsub := BEffs.sub_sound hR
        rw [Ceiling.ballows_toEffs, ← CBinding.toBinding_R] at hsub
        rw [toEffs_addIf, ← CBinding.toBinding_R]
        exact ArgsOk.annotated (CCtx.lookup_toCtx hb)
          (by rw [CBinding.toBinding_U]; exact hU) hsub (argsCheck_sound hrest)
      · cases h
    · cases h
  | [], _ :: _, _, _, h => by simp [argsCheck] at h
  | _ :: _, [], _, _, h => by simp [argsCheck] at h

theorem check_sound {P : Program} {Φ : Name → BEffs} :
    ∀ {s Γ R U}, check P Φ Γ s = some (R, U) →
    Typed P (fun f => (Φ f).toEffs) (CCtx.toCtx Γ) s R.toEffs U := by
  intro s
  induction s with
  | stop =>
    intro Γ R U h
    simp [check] at h
    obtain ⟨rfl, rfl⟩ := h
    rw [BEffs.toEffs_none]
    exact Typed.stop
  | loop body ih =>
    intro Γ R U h
    simp only [check] at h
    exact Typed.loop (ih h)
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
  | letClosure x k ps body rest ihb ihr =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i Rb Ub hb
      split at h
      · rename_i hfit
        have htb := ihb hb
        rw [CCtx.toCtx_append, cparamCtx_toCtx, toCtx_cweaken] at htb
        exact Typed.letClosure htb (fitsB_sound hfit) (ihr (Γ := (x, .known Rb Ub (names ps)) :: Γ) h)
      · cases h
    · cases h
  | letAlias x y rest ih =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      exact Typed.letAlias (CCtx.lookup_toCtx hb) (ih (Γ := (x, b) :: Γ) h)
    · cases h
  | letOpaque x k ps body rest ihb ihr =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i Rb Ub hb
      split at h
      · rename_i hfit
        have htb := ihb hb
        rw [CCtx.toCtx_append, cparamCtx_toCtx, toCtx_cweaken] at htb
        have hr := ihr (Γ := (x, .known (opaqueB k) (opaqueU k (names ps)) (opaquePs k (names ps))) :: Γ) h
        simp only [CCtx.toCtx, CBinding.toBinding, opaqueB_toEffs] at hr
        exact Typed.letOpaque htb (fitsB_sound hfit) hr
      · cases h
    · cases h
  | callClosure x args pks =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      split at h
      · rename_i R' U' hargs
        have hargs' := argsCheck_sound hargs
        rw [← CBinding.toBinding_addsAll] at hargs'
        have ht := Typed.callClosure (P := P) (Φ := fun f => (Φ f).toEffs) (CCtx.lookup_toCtx hb) hargs'
        simp only [argEffs, argUses, CBinding.toBinding_mayCallArgs, CBinding.toBinding_R,
          CBinding.toBinding_outer] at ht
        cases hm : b.mayCallArgs with
        | true =>
          simp only [hm, ite_true] at h ht
          simp only [Option.some.injEq, Prod.mk.injEq] at h
          obtain ⟨rfl, rfl⟩ := h
          rw [BEffs.toEffs_union]
          exact ht
        | false =>
          simp only [hm, Bool.false_eq_true, ite_false, Option.some.injEq, Prod.mk.injEq] at h ht
          obtain ⟨rfl, rfl⟩ := h
          rw [BEffs.toEffs_union, BEffs.toEffs_none]
          exact ht
      · cases h
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
  | .letClosure _ _ _ body rest => confirmedB body && confirmedB rest
  | .letAlias _ _ rest => confirmedB rest
  | .letOpaque _ _ _ body rest => confirmedB body && confirmedB rest
  | .callClosure _ _ _ => true
  | .callFun _ _ => true
  | .loop body => confirmedB body
  | .stop => true

theorem confirmedB_sound : ∀ {s}, confirmedB s = true → Confirmed s
  | .op o, h => Confirmed.op h
  | .seq s₁ s₂, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.seq (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .choice s₁ s₂, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.choice (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .letClosure _ _ _ body rest, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.letClosure (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .letAlias _ _ rest, h => Confirmed.letAlias (confirmedB_sound h)
  | .letOpaque _ _ _ body rest, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.letOpaque (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .callClosure _ _ _, _ => Confirmed.callClosure
  | .callFun _ _, _ => Confirmed.callFun
  | .loop _, h => Confirmed.loop (confirmedB_sound h)
  | .stop, _ => Confirmed.stop

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

theorem accept_effects_sound {P : Program} {Φ : Name → BEffs} {fs : List Name} {s R U t b}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programOkB P Φ fs = true)
    (hc : check P Φ [] s = some (R, U)) (h : Eval P [] [] s t b) :
    ∀ ev, ev ∈ t → R.toEffs ev.effect :=
  program_effects_sound (programOkB_sound hcover hok) h (check_sound (Γ := []) hc)

theorem accept_ceilings_sound {P : Program} {Φ : Name → BEffs} {fs : List Name} {s R U t b}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programOkB P Φ fs = true)
    (hc : check P Φ [] s = some (R, U)) (h : Eval P [] [] s t b) :
    ∀ ev, ev ∈ t → ∀ c, c ∈ ev.ceilings → c.allows ev.effect :=
  program_ceilings_sound (programOkB_sound hcover hok) h (check_sound (Γ := []) hc)

theorem accept_confirmed {P : Program} {fs : List Name} {s t b}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programConfirmedB P fs = true)
    (hc : confirmedB s = true) (h : Eval P [] [] s t b) :
    ∀ ev, ev ∈ t → ev.confirmed = true :=
  program_confirmed (programConfirmedB_sound hcover hok) h (confirmedB_sound hc)


/-! ## Value shapes, computed -/

def argsShapedB (Δ : Shapes) : List (List (Option Ceiling)) → List Name → Bool
  | [], [] => true
  | t :: ts, a :: as => decide (lookup Δ a = some t) && argsShapedB Δ ts as
  | _, _ => false

theorem argsShapedB_sound {Δ : Shapes} :
    ∀ {ts as}, argsShapedB Δ ts as = true → ArgsShaped Δ ts as
  | [], [], _ => ArgsShaped.nil
  | t :: ts, a :: as, h => by
    simp only [argsShapedB, Bool.and_eq_true, decide_eq_true_eq] at h
    exact ArgsShaped.cons h.1 (argsShapedB_sound h.2)
  | [], _ :: _, h => by simp [argsShapedB] at h
  | _ :: _, [], h => by simp [argsShapedB] at h

/-- The shape rule, as a function. -/
def shapedB (P : Program) : Shapes → Stmt → Bool
  | _, .op _ => true
  | _, .stop => true
  | Δ, .seq s₁ s₂ => shapedB P Δ s₁ && shapedB P Δ s₂
  | Δ, .choice s₁ s₂ => shapedB P Δ s₁ && shapedB P Δ s₂
  | Δ, .loop s => shapedB P Δ s
  | Δ, .letClosure x _ ps body rest =>
    shapedB P (paramShapes ps [] ++ Δ) body && shapedB P ((x, ps.map Prod.snd) :: Δ) rest
  | Δ, .letOpaque x _ ps body rest =>
    shapedB P (paramShapes ps [] ++ Δ) body && shapedB P ((x, ps.map Prod.snd) :: Δ) rest
  | Δ, .letAlias x y rest =>
    match lookup Δ y with
    | some t => shapedB P ((x, t) :: Δ) rest
    | none => false
  | Δ, .callClosure x args pks =>
    decide (lookup Δ x = some pks) && decide (args.length = pks.length) &&
      argsShapedB Δ (List.replicate args.length []) args
  | Δ, .callFun f args =>
    match P f with
    | some d => argsShapedB Δ (wantOf d.params d.ptypes) args
    | none => false

theorem shapedB_sound {P : Program} : ∀ {s Δ}, shapedB P Δ s = true → Shaped P Δ s := by
  intro s
  induction s with
  | op o => intro _ _; exact Shaped.op
  | stop => intro _ _; exact Shaped.stop
  | seq s₁ s₂ ih₁ ih₂ =>
    intro Δ h
    simp only [shapedB, Bool.and_eq_true] at h
    exact Shaped.seq (ih₁ h.1) (ih₂ h.2)
  | choice s₁ s₂ ih₁ ih₂ =>
    intro Δ h
    simp only [shapedB, Bool.and_eq_true] at h
    exact Shaped.choice (ih₁ h.1) (ih₂ h.2)
  | loop body ih =>
    intro Δ h
    exact Shaped.loop (ih h)
  | letClosure x k ps body rest ihb ihr =>
    intro Δ h
    simp only [shapedB, Bool.and_eq_true] at h
    exact Shaped.letClosure (ihb h.1) (ihr h.2)
  | letOpaque x k ps body rest ihb ihr =>
    intro Δ h
    simp only [shapedB, Bool.and_eq_true] at h
    exact Shaped.letOpaque (ihb h.1) (ihr h.2)
  | letAlias x y rest ih =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i t ht
      exact Shaped.letAlias ht (ih h)
    · cases h
  | callClosure x args pks =>
    intro Δ h
    simp only [shapedB, Bool.and_eq_true, decide_eq_true_eq] at h
    exact Shaped.callClosure h.1.1 h.1.2 (argsShapedB_sound h.2)
  | callFun f args =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i d hd
      exact Shaped.callFun hd (argsShapedB_sound h)
    · cases h

/-- Every function body in `fs` is shaped. -/
def programShapedB (P : Program) (fs : List Name) : Bool :=
  fs.all fun f =>
    match P f with
    | some d => shapedB P (paramShapes d.params d.ptypes) d.body
    | none => true

theorem programShapedB_sound {P : Program} {fs : List Name}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (h : programShapedB P fs = true) : ShapeOk P := by
  intro f d hd
  have hf := List.all_eq_true.mp h f (hcover f d hd)
  simp only [hd] at hf
  exact shapedB_sound hf

theorem accept_shapes_sound {P : Program} {fs : List Name} {s : Stmt}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programShapedB P fs = true)
    (hs : shapedB P [] s = true) : ¬ Stuck P [] s :=
  program_shapes_sound (programShapedB_sound hcover hok) (shapedB_sound hs)

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
