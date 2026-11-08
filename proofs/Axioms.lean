import NucleScriptCore

/-! Lists the axioms each theorem rests on. CI fails if `sorryAx` appears,
which would mean a step was left unproved. -/

#print axioms NucleScript.program_effects_sound
#print axioms NucleScript.program_ceilings_sound
#print axioms NucleScript.program_confirmed
