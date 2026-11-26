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

def cinert : CBinding := .known BEffs.none [] []

def cuntraced : CBinding := .known BEffs.all ["*"] ["*"]

theorem cinert_toBinding : cinert.toBinding = inert := by
  simp [cinert, inert, CBinding.toBinding, BEffs.toEffs_none]

theorem cuntraced_toBinding : cuntraced.toBinding = untraced := by
  simp [cuntraced, untraced, CBinding.toBinding, BEffs.toEffs_all]

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

theorem cparamBinding_toBinding (p : Name) (k : Option Ceiling) :
    (cparamBinding p k).toBinding = paramBinding p k := by
  cases k <;> simp [cparamBinding, paramBinding, CBinding.toBinding, Ceiling.ballows_toEffs]

def cfparamCtx : List (Name × Option Ceiling) → List Ty → CCtx
  | [], _ => []
  | (p, k) :: ps, [] => (p, cparamBinding p k) :: cfparamCtx ps []
  | (p, k) :: ps, t :: ts => (p, if t.isFn then cparamBinding p k else cinert) :: cfparamCtx ps ts

theorem cfparamCtx_toCtx : ∀ ps ts, CCtx.toCtx (cfparamCtx ps ts) = fparamCtx ps ts
  | [], _ => rfl
  | (p, k) :: ps, [] => by
    simp only [cfparamCtx, fparamCtx, CCtx.toCtx, cparamBinding_toBinding, cfparamCtx_toCtx ps []]
  | (p, k) :: ps, t :: ts => by
    cases ht : t.isFn <;>
      simp [cfparamCtx, fparamCtx, CCtx.toCtx, ht, cparamBinding_toBinding, cinert_toBinding,
        cfparamCtx_toCtx ps ts]

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
  | Γ, .callFun f args _ =>
    match P f with
    | some d =>
      match argsCheck Γ false (d.params.map Prod.snd) args with
      | some (R, U) => some ((Φ f).union R, U)
      | none => none
    | none => none
  | Γ, .letVal x _ rest => check P Φ ((x, cinert) :: Γ) rest
  | Γ, .tryOk x _ y rest =>
    match lookup Γ y with
    | some _ => check P Φ ((x, cuntraced) :: Γ) rest
    | none => none
  | Γ, .matchTag y _ z s₁ s₂ =>
    match lookup Γ y with
    | some b =>
      match check P Φ ((z, cuntraced) :: Γ) s₁, check P Φ ((y, b) :: Γ) s₂ with
      | some (R₁, U₁), some (R₂, U₂) => some (R₁.union R₂, U₁ ++ U₂)
      | _, _ => none
    | none => none
  | _, .noMatch _ => some (BEffs.none, [])

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
  | callFun f args inst =>
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
  | letVal x ex rest ih =>
    intro Γ R U h
    simp only [check] at h
    have hr := ih (Γ := (x, cinert) :: Γ) h
    simp only [CCtx.toCtx, cinert_toBinding] at hr
    exact Typed.letVal hr
  | tryOk x ty y rest ih =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      have hr := ih (Γ := (x, cuntraced) :: Γ) h
      simp only [CCtx.toCtx, cuntraced_toBinding] at hr
      exact Typed.tryOk (CCtx.lookup_toCtx hb) hr
    · cases h
  | matchTag y tag z s₁ s₂ ih₁ ih₂ =>
    intro Γ R U h
    simp only [check] at h
    split at h
    · rename_i b hb
      split at h
      · rename_i R₁ U₁ R₂ U₂ h₁ h₂
        simp at h
        obtain ⟨rfl, rfl⟩ := h
        have hr₁ := ih₁ (Γ := (z, cuntraced) :: Γ) h₁
        have hr₂ := ih₂ (Γ := (y, b) :: Γ) h₂
        simp only [CCtx.toCtx, cuntraced_toBinding] at hr₁
        simp only [CCtx.toCtx] at hr₂
        rw [BEffs.toEffs_union]
        exact Typed.matchTag (CCtx.lookup_toCtx hb) hr₁ hr₂
      · cases h
    · cases h
  | noMatch y =>
    intro Γ R U h
    simp [check] at h
    obtain ⟨rfl, rfl⟩ := h
    rw [BEffs.toEffs_none]
    exact Typed.noMatch

/-! ## Whole programs -/

/-- Every function in `fs` checks, with its body's effects within `Φ`. -/
def programOkB (P : Program) (Φ : Name → BEffs) (fs : List Name) : Bool :=
  fs.all fun f =>
    match P f with
    | some d =>
      match check P Φ (cfparamCtx d.params d.ptypes) d.body with
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
    rw [cfparamCtx_toCtx] at ht
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
  | .callFun _ _ _ => true
  | .loop body => confirmedB body
  | .stop => true
  | .letVal _ _ rest => confirmedB rest
  | .tryOk _ _ _ rest => confirmedB rest
  | .matchTag _ _ _ s₁ s₂ => confirmedB s₁ && confirmedB s₂
  | .noMatch _ => true

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
  | .callFun _ _ _, _ => Confirmed.callFun
  | .loop _, h => Confirmed.loop (confirmedB_sound h)
  | .stop, _ => Confirmed.stop
  | .letVal _ _ _, h => Confirmed.letVal (confirmedB_sound h)
  | .tryOk _ _ _ _, h => Confirmed.tryOk (confirmedB_sound h)
  | .matchTag _ _ _ _ _, h => by
    simp only [confirmedB, Bool.and_eq_true] at h
    exact Confirmed.matchTag (confirmedB_sound h.1) (confirmedB_sound h.2)
  | .noMatch _, _ => Confirmed.noMatch

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

/-- `TyFits`, computed. -/
def tyFitsB : Ty → Ty → Bool
  | .enum e r, .enum e' r' => decide (e = e') && r.all fun x => decide (x ∈ r')
  | .result t e r, .result t' e' r' =>
    (decide (t = t') || tyFitsB t t') && (decide (e = e') || tyFitsB e e') && r.all fun x => decide (x ∈ r')
  | t, t' => decide (t = t')

theorem tyFitsB_sound : ∀ {t t' : Ty}, tyFitsB t t' = true → TyFits t t'
  | .enum e r, .enum e' r', h => by
    simp only [tyFitsB, Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true] at h
    obtain ⟨rfl, hr⟩ := h
    exact TyFits.enum hr
  | .result t e r, .result t' e' r', h => by
    simp only [tyFitsB, Bool.and_eq_true, Bool.or_eq_true, decide_eq_true_eq, List.all_eq_true] at h
    obtain ⟨⟨ht, he⟩, hr⟩ := h
    refine TyFits.result ?_ ?_ hr
    · rcases ht with rfl | ht
      · exact TyFits.refl
      · exact tyFitsB_sound ht
    · rcases he with rfl | he
      · exact TyFits.refl
      · exact tyFitsB_sound he
  | .fn _, _, h => by
    simp only [tyFitsB, decide_eq_true_eq] at h
    subst h
    exact TyFits.refl
  | .pool _, _, h => by
    simp only [tyFitsB, decide_eq_true_eq] at h
    subst h
    exact TyFits.refl
  | .base _, _, h => by
    simp only [tyFitsB, decide_eq_true_eq] at h
    subst h
    exact TyFits.refl
  | .enum _ _, .fn _, h => by simp [tyFitsB] at h
  | .enum _ _, .pool _, h => by simp [tyFitsB] at h
  | .enum _ _, .base _, h => by simp [tyFitsB] at h
  | .enum _ _, .result _ _ _, h => by simp [tyFitsB] at h
  | .result _ _ _, .fn _, h => by simp [tyFitsB] at h
  | .result _ _ _, .pool _, h => by simp [tyFitsB] at h
  | .result _ _ _, .base _, h => by simp [tyFitsB] at h
  | .result _ _ _, .enum _ _, h => by simp [tyFitsB] at h

def argsShapedB (Δ : Shapes) : List Ty → List Name → Bool
  | [], [] => true
  | t :: ts, a :: as =>
    (match lookup Δ a with
     | some t' => tyFitsB t' t
     | none => false) && argsShapedB Δ ts as
  | _, _ => false

theorem argsShapedB_sound {Δ : Shapes} :
    ∀ {ts as}, argsShapedB Δ ts as = true → ArgsShaped Δ ts as
  | [], [], _ => ArgsShaped.nil
  | t :: ts, a :: as, h => by
    simp only [argsShapedB, Bool.and_eq_true] at h
    obtain ⟨h₁, h₂⟩ := h
    split at h₁
    · rename_i t' ht'
      exact ArgsShaped.cons ht' (tyFitsB_sound h₁) (argsShapedB_sound h₂)
    · cases h₁
  | [], _ :: _, h => by simp [argsShapedB] at h
  | _ :: _, [], h => by simp [argsShapedB] at h

def Ty.exhaustedB : Ty → Bool
  | .enum _ rem => rem.isEmpty
  | .result _ _ rem => rem.isEmpty
  | _ => false

theorem Ty.exhaustedB_sound : ∀ {t : Ty}, t.exhaustedB = true → t.exhausted
  | .enum _ rem, h => by simpa [Ty.exhaustedB, Ty.exhausted] using h
  | .result _ _ rem, h => by simpa [Ty.exhaustedB, Ty.exhausted] using h
  | .fn _, h => by simp [Ty.exhaustedB] at h
  | .pool _, h => by simp [Ty.exhaustedB] at h
  | .base _, h => by simp [Ty.exhaustedB] at h

/-- The shape rule, as a function. -/
def shapedB (P : Program) (En : Enums) : Shapes → Stmt → Bool
  | _, .op _ => true
  | _, .stop => true
  | Δ, .seq s₁ s₂ => shapedB P En Δ s₁ && shapedB P En Δ s₂
  | Δ, .choice s₁ s₂ => shapedB P En Δ s₁ && shapedB P En Δ s₂
  | Δ, .loop s => shapedB P En Δ s
  | Δ, .letClosure x _ ps body rest =>
    shapedB P En (paramShapes ps [] ++ Δ) body && shapedB P En ((x, .fn (ps.map Prod.snd)) :: Δ) rest
  | Δ, .letOpaque x _ ps body rest =>
    shapedB P En (paramShapes ps [] ++ Δ) body && shapedB P En ((x, .fn (ps.map Prod.snd)) :: Δ) rest
  | Δ, .letAlias x y rest =>
    match lookup Δ y with
    | some t => shapedB P En ((x, t) :: Δ) rest
    | none => false
  | Δ, .callClosure x args pks =>
    decide (lookup Δ x = some (.fn pks)) && decide (args.length = pks.length) &&
      argsShapedB Δ (List.replicate args.length (.fn [])) args
  | Δ, .callFun f args inst =>
    match P f with
    | some d => argsShapedB Δ ((wantOf d.params d.ptypes).map (Ty.subst (instFun inst))) args
    | none => false
  | Δ, .letVal x ex rest =>
    match ex.type En with
    | some t => shapedB P En ((x, t) :: Δ) rest
    | none => false
  | Δ, .tryOk x ty y rest =>
    match lookup Δ y with
    | some (.result t _ _) => tyFitsB t ty && shapedB P En ((x, ty) :: Δ) rest
    | _ => false
  | Δ, .matchTag y tag z s₁ s₂ =>
    match lookup Δ y with
    | some t =>
      match armType En t tag with
      | some pt => shapedB P En ((z, pt) :: Δ) s₁ && shapedB P En ((y, t.without tag) :: Δ) s₂
      | none => false
    | none => false
  | Δ, .noMatch y =>
    match lookup Δ y with
    | some t => t.exhaustedB
    | none => false

theorem shapedB_sound {P : Program} {En : Enums} : ∀ {s Δ}, shapedB P En Δ s = true → Shaped P En Δ s := by
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
  | callFun f args inst =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i d hd
      exact Shaped.callFun hd (argsShapedB_sound h)
    · cases h
  | letVal x ex rest ih =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i t ht
      exact Shaped.letVal ht (ih h)
    · cases h
  | tryOk x ty y rest ih =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i t e rem hy
      simp only [Bool.and_eq_true] at h
      exact Shaped.tryOk hy (tyFitsB_sound h.1) (ih h.2)
    · cases h
  | matchTag y tag z s₁ s₂ ih₁ ih₂ =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i t ht
      split at h
      · rename_i pt hpt
        simp only [Bool.and_eq_true] at h
        exact Shaped.matchTag ht hpt (ih₁ h.1) (ih₂ h.2)
      · cases h
    · cases h
  | noMatch y =>
    intro Δ h
    simp only [shapedB] at h
    split at h
    · rename_i t ht
      exact Shaped.noMatch ht (Ty.exhaustedB_sound h)
    · cases h

/-- Every function body in `fs` is shaped. -/
def programShapedB (P : Program) (En : Enums) (fs : List Name) : Bool :=
  fs.all fun f =>
    match P f with
    | some d => shapedB P En (paramShapes d.params d.ptypes) d.body
    | none => true

theorem programShapedB_sound {P : Program} {En : Enums} {fs : List Name}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (h : programShapedB P En fs = true) : ShapeOk P En := by
  intro f d hd
  have hf := List.all_eq_true.mp h f (hcover f d hd)
  simp only [hd] at hf
  exact shapedB_sound hf

theorem accept_shapes_sound {P : Program} {En : Enums} {fs : List Name} {s : Stmt}
    (hcover : ∀ f d, P f = some d → f ∈ fs) (hok : programShapedB P En fs = true)
    (hs : shapedB P En [] s = true) : ¬ Stuck P [] s :=
  program_shapes_sound (programShapedB_sound hcover hok) (shapedB_sound hs)

/-! ## Signatures -/

/-- One round of computing each function's effect from its body. -/
def sigStep (P : Program) (Φ : Name → BEffs) (f : Name) : BEffs :=
  match P f with
  | some d =>
    match check P Φ (cfparamCtx d.params d.ptypes) d.body with
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
