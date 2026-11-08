//! Runs programs the checker accepts and compares what they did with what
//! the checker promised (see docs/soundness.md):
//!
//! 1. every delete that ran carried `confirm physical_key`;
//! 2. no top-level declaration did more than `nucle explain`'s effect for
//!    it (`effects::effect_summary`) says it can;
//! 3. the interpreter never met a value the type checker should have ruled
//!    out, and no closure passed through a `Fn(...) -> T confirm ...` slot
//!    ran an operation beyond that ceiling (both are `RunTrace::faults`).
//!
//! The programs are the shipped examples plus several hundred generated
//! from a fixed seed: functions, closures held in `let`s and passed to
//! higher-order functions with and without effect ceilings, closures
//! returned from functions and passed on under other names, generic
//! functions over a pool's profile, recursion, `Result` functions with `?`
//! and `match`, loops, branches, pipelines and `retain` requirements, with
//! an occasional missing confirmation or wrong ceiling the checker has to
//! reject.

use nucle_codec::stable_rng::StableRng;
use nucle_lang::effects::effect_summary;
use nucle_lang::{compile, execute_program, take_run_trace, CompileError, Effect, RunTrace};
use nucle_vfs::syscall::NucleOS;
use std::path::{Path, PathBuf};

fn rank(effect: Effect) -> u8 {
    match effect {
        Effect::Pure => 0,
        Effect::Sequencing => 1,
        Effect::Synthesis => 2,
        Effect::Destructive => 3,
    }
}

/// What one accepted program did, and every way it broke a promise.
struct Outcome {
    trace: RunTrace,
    violations: Vec<String>,
    /// Top-level declarations whose static effect was above what ran.
    over_approximated: usize,
    declarations_that_ran: usize,
}

/// `None` when the checker rejects `source`.
fn run_monitored(source: &str, base_dir: &Path) -> Option<Outcome> {
    let mut plan = match compile(source) {
        Ok(plan) => plan,
        Err(CompileError::Type(_)) | Err(CompileError::Parse(_)) | Err(CompileError::Lex(_)) => return None,
        Err(other) => panic!("unexpected compile failure: {other}"),
    };
    let promised = effect_summary(&plan.program);
    let mut os = NucleOS::new(100);
    // A run may stop early (a delete of a file an earlier delete already
    // removed); what it did up to there must still keep every promise.
    let _ = execute_program(&mut os, &mut plan, base_dir);
    let trace = take_run_trace();

    let mut violations = Vec::new();
    let mut worst = vec![None::<Effect>; promised.declarations.len()];
    for seen in &trace.effects {
        if seen.effect == Effect::Destructive && !seen.confirmed {
            violations.push(format!("delete of {} ran without confirm physical_key", seen.target));
        }
        let declared = &promised.declarations[seen.declaration];
        if rank(seen.effect) > rank(declared.effect) {
            violations.push(format!(
                "declaration {} ({} '{}') is {} per nucle explain but ran a {} operation on {}",
                seen.declaration, declared.kind, declared.name, declared.effect, seen.effect, seen.target
            ));
        }
        let slot = &mut worst[seen.declaration];
        if slot.is_none_or(|w| rank(seen.effect) > rank(w)) {
            *slot = Some(seen.effect);
        }
    }
    violations.extend(trace.faults.iter().map(|f| format!("type fault: {f}")));
    let declarations_that_ran = worst.iter().filter(|w| w.is_some()).count();
    let over_approximated = worst
        .iter()
        .zip(&promised.declarations)
        .filter(|(w, d)| w.is_some_and(|w| rank(d.effect) > rank(w)))
        .count();
    Some(Outcome { trace, violations, over_approximated, declarations_that_ran })
}

#[test]
fn shipped_examples_keep_their_effect_and_type_promises_when_run() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("docs/examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&examples)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "nsl"))
        .collect();
    files.sort();

    let (mut ran, mut operations) = (0, 0);
    let mut violations = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let Some(outcome) = run_monitored(&source, &examples) else { continue };
        ran += 1;
        operations += outcome.trace.effects.len();
        violations.extend(outcome.violations.into_iter().map(|v| format!("{}: {v}", path.file_name().unwrap().to_string_lossy())));
    }
    eprintln!("{ran} of {} examples compiled and ran {operations} stores and deletes", files.len());
    assert!(ran >= 20, "only {ran} examples compiled and ran");
    assert!(operations >= 40, "the examples ran only {operations} stores and deletes");
    assert!(violations.is_empty(), "{} broken promises:\n{}", violations.len(), violations.join("\n"));
}

/// Builds random NucleScript programs from a seed.
struct Gen {
    rng: StableRng,
    next_name: usize,
    void_fns: Vec<String>,
    file_fns: Vec<String>,
    result_fns: Vec<String>,
    /// Higher-order functions and the ceiling on their `Fn()` parameter.
    hofs: Vec<(String, Ceiling)>,
    /// Functions returning a closure the caller cannot see into.
    makers: Vec<String>,
    /// Functions taking a pool: generic over its profile, or `Pool<Illumina>`.
    generic_fns: Vec<String>,
    /// Functions that call themselves until the call-depth limit stops them.
    recursive_fns: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Ceiling {
    None,
    Hardware,
    PhysicalKey,
}

impl Ceiling {
    fn suffix(self) -> &'static str {
        match self {
            Ceiling::None => "",
            Ceiling::Hardware => " confirm hardware",
            Ceiling::PhysicalKey => " confirm physical_key",
        }
    }
}

/// What a generated body does, so a closure's type can name a ceiling
/// that fits it.
#[derive(Default)]
struct BodyEffects {
    deletes: bool,
    stores: bool,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Gen {
            rng: StableRng::seed_from_u64(seed),
            next_name: 0,
            void_fns: vec![],
            file_fns: vec![],
            result_fns: vec![],
            hofs: vec![],
            makers: vec![],
            generic_fns: vec![],
            recursive_fns: vec![],
        }
    }

    fn name(&mut self, prefix: &str) -> String {
        self.next_name += 1;
        format!("{prefix}{}", self.next_name)
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.gen_f64() < p
    }

    fn pick<'a>(&mut self, items: &'a [String]) -> &'a str {
        &items[self.rng.gen_range(0..items.len())]
    }

    /// Half the time, a higher-order function to pass a closure to.
    fn some_hof(&mut self) -> Option<String> {
        if self.hofs.is_empty() || !self.chance(0.5) {
            return None;
        }
        let i = self.rng.gen_range(0..self.hofs.len());
        Some(self.hofs[i].0.clone())
    }

    fn file(&mut self) -> String {
        format!("\"f{}.txt\"", self.rng.gen_range(0..4))
    }

    fn pool(&mut self) -> String {
        format!("p{}", self.rng.gen_range(0..2))
    }

    /// Usually `confirm physical_key`; now and then nothing, which the
    /// checker must reject.
    fn delete_confirm(&mut self) -> &'static str {
        if self.chance(0.04) {
            ""
        } else {
            " confirm physical_key"
        }
    }

    /// One operation a closure body can hold, kept to what `ceiling`
    /// allows except for the occasional deliberate mismatch.
    fn simple_op(&mut self, ceiling: Ceiling, effects: &mut BodyEffects) -> String {
        // A call reaches whatever that function does, closures it binds
        // included; whether that fits `ceiling` is for the checker to work
        // out.
        if !self.void_fns.is_empty() && self.chance(0.2) {
            let f = self.pick(&self.void_fns.clone()).to_string();
            let v = self.name("done");
            return format!("let {v}: Void = {f}()");
        }
        let fits = !self.chance(0.05);
        let want_delete = match (ceiling, fits) {
            (Ceiling::Hardware, true) => false,
            (Ceiling::PhysicalKey, true) => self.chance(0.7),
            (Ceiling::Hardware, false) => true,
            (Ceiling::PhysicalKey, false) => false,
            (Ceiling::None, _) => self.chance(0.5),
        };
        let (file, pool) = (self.file(), self.pool());
        if want_delete {
            effects.deletes = true;
            let confirm = self.delete_confirm();
            return format!("delete {file} from {pool}{confirm}");
        }
        match (ceiling, fits) {
            (Ceiling::PhysicalKey, true) => {
                if self.chance(0.5) {
                    format!("simulate store {file} into {pool}")
                } else {
                    format!("retrieve from {pool}")
                }
            }
            _ => {
                effects.stores = true;
                format!("store {file} into {pool}")
            }
        }
    }

    /// The ceiling a closure's own type declares for a body with
    /// `effects`: none, the one that fits, or (rarely) the wrong one.
    fn closure_ceiling(&mut self, effects: &BodyEffects) -> Ceiling {
        if self.chance(0.5) {
            return Ceiling::None;
        }
        let fitting = if effects.deletes && effects.stores {
            // No single ceiling covers both; the checker must refuse
            // whichever is written.
            if self.chance(0.5) {
                Ceiling::Hardware
            } else {
                Ceiling::PhysicalKey
            }
        } else if effects.deletes {
            Ceiling::PhysicalKey
        } else {
            Ceiling::Hardware
        };
        if self.chance(0.05) {
            if fitting == Ceiling::Hardware {
                Ceiling::PhysicalKey
            } else {
                Ceiling::Hardware
            }
        } else {
            fitting
        }
    }

    fn closure_literal(&mut self, ceiling: Ceiling, indent: &str, effects: &mut BodyEffects) -> String {
        let count = 1 + self.rng.gen_range(0..2);
        let mut body = String::new();
        for _ in 0..count {
            let op = self.simple_op(ceiling, effects);
            body.push_str(&format!("{indent}    {op}\n"));
        }
        format!("fn() -> Void {{\n{body}{indent}}}")
    }

    /// One statement in a function body (`in_result_fn` allows `?`) or at
    /// top level; `result_vars` are `Result<DnaFile, Str>` bindings in
    /// scope.
    fn statement(&mut self, indent: &str, in_result_fn: bool, result_vars: &mut Vec<String>) -> String {
        loop {
            let (file, pool) = (self.file(), self.pool());
            match self.rng.gen_range(0..18) {
                0 | 1 => return format!("{indent}store {file} into {pool}\n"),
                2 => return format!("{indent}simulate store {file} into {pool}\n"),
                3 | 4 => {
                    let confirm = self.delete_confirm();
                    return format!("{indent}delete {file} from {pool}{confirm}\n");
                }
                5 => return format!("{indent}retrieve from {pool}\n"),
                6 => {
                    let v = self.name("stored");
                    result_vars.push(v.clone());
                    return format!("{indent}let {v}: Result<DnaFile, Str> = store {file} into {pool}\n");
                }
                7 => {
                    let v = self.name("removed");
                    let confirm = self.delete_confirm();
                    return format!("{indent}let {v}: Result<Void, Str> = delete {file} from {pool}{confirm}\n");
                }
                8 if !self.void_fns.is_empty() => {
                    let f = self.pick(&self.void_fns.clone()).to_string();
                    let v = self.name("done");
                    return format!("{indent}let {v}: Void = {f}()\n");
                }
                9 if !self.file_fns.is_empty() => {
                    let f = self.pick(&self.file_fns.clone()).to_string();
                    let v = self.name("done");
                    return format!("{indent}let {v}: Void = {f}({file})\n");
                }
                10 => {
                    let mut effects = BodyEffects::default();
                    let literal = self.closure_literal(Ceiling::None, indent, &mut effects);
                    let ceiling = self.closure_ceiling(&effects);
                    let (c, v) = (self.name("step"), self.name("done"));
                    return format!("{indent}let {c}: Fn() -> Void{} = {literal}\n{indent}let {v}: Void = {c}()\n", ceiling.suffix());
                }
                11 if !self.hofs.is_empty() => {
                    let (hof, ceiling) = self.hofs[self.rng.gen_range(0..self.hofs.len())].clone();
                    let mut effects = BodyEffects::default();
                    let literal = self.closure_literal(ceiling, indent, &mut effects);
                    let v = self.name("done");
                    if self.chance(0.5) {
                        return format!("{indent}let {v}: Void = {hof}({literal})\n");
                    }
                    let c = self.name("action");
                    return format!("{indent}let {c}: Fn() -> Void{} = {literal}\n{indent}let {v}: Void = {hof}({c})\n", ceiling.suffix());
                }
                12 if !self.result_fns.is_empty() => {
                    let f = self.pick(&self.result_fns.clone()).to_string();
                    if in_result_fn && self.chance(0.5) {
                        let v = self.name("file");
                        return format!("{indent}let {v}: DnaFile = {f}()?\n");
                    }
                    let v = self.name("outcome");
                    result_vars.push(v.clone());
                    return format!("{indent}let {v}: Result<DnaFile, Str> = {f}()\n");
                }
                14 if !self.makers.is_empty() => {
                    // A closure out of a function: called, or handed to a
                    // higher-order function, whose ceiling it cannot meet.
                    let maker = self.pick(&self.makers.clone()).to_string();
                    let (g, v) = (self.name("made"), self.name("done"));
                    let call = match self.some_hof() {
                        Some(hof) => format!("{hof}({g})"),
                        None => format!("{g}()"),
                    };
                    return format!("{indent}let {g}: Fn() -> Void = {maker}()\n{indent}let {v}: Void = {call}\n");
                }
                15 => {
                    // A closure under a second name.
                    let mut effects = BodyEffects::default();
                    let literal = self.closure_literal(Ceiling::None, indent, &mut effects);
                    let (c, h, v) = (self.name("step"), self.name("alias"), self.name("done"));
                    let call = match self.some_hof() {
                        Some(hof) => format!("{hof}({h})"),
                        None => format!("{h}()"),
                    };
                    return format!("{indent}let {c}: Fn() -> Void = {literal}\n{indent}let {h}: Fn() -> Void = {c}\n{indent}let {v}: Void = {call}\n");
                }
                16 if !self.generic_fns.is_empty() => {
                    let f = self.pick(&self.generic_fns.clone()).to_string();
                    let v = self.name("done");
                    return format!("{indent}let {v}: Void = {f}({pool})\n");
                }
                17 if !self.recursive_fns.is_empty() && self.chance(0.3) => {
                    let f = self.pick(&self.recursive_fns.clone()).to_string();
                    let v = self.name("done");
                    return format!("{indent}let {v}: Void = {f}()\n");
                }
                13 if in_result_fn && !result_vars.is_empty() => {
                    let r = self.pick(&result_vars.clone()).to_string();
                    let v = self.name("saved");
                    return format!(
                        "{indent}let {v}: DnaFile = match {r} {{\n{indent}    Ok(file) => file,\n{indent}    Err(reason) => (store {file} into {pool})?\n{indent}}}\n"
                    );
                }
                _ => continue,
            }
        }
    }

    fn function(&mut self) -> String {
        let mut result_vars = Vec::new();
        match self.rng.gen_range(0..8) {
            7 => {
                // A concrete profile: called with the Twist pool, the
                // checker must refuse; with the Illumina pool, the run
                // checks the pool it got.
                let (name, file) = (self.name("into"), self.file());
                self.generic_fns.push(name.clone());
                format!("fn {name}(target: Pool<Illumina>) returns Void {{
    store {file} into target
}}
")
            }
            4 => {
                let name = self.name("make");
                let mut effects = BodyEffects::default();
                let literal = self.closure_literal(Ceiling::None, "    ", &mut effects);
                self.makers.push(name.clone());
                format!("fn {name}() returns Fn() -> Void {{\n    let made: Fn() -> Void = {literal}\n}}\n")
            }
            5 => {
                let name = self.name("put");
                let file = self.file();
                let mut body = format!("    store {file} into target\n");
                if self.chance(0.4) {
                    let confirm = self.delete_confirm();
                    body.push_str(&format!("    delete {file} from target{confirm}\n"));
                }
                self.generic_fns.push(name.clone());
                format!("fn {name}<P>(target: Pool<P>) returns Void {{\n{body}}}\n")
            }
            6 => {
                let (name, file, pool) = (self.name("spin"), self.file(), self.pool());
                let v = self.name("again");
                self.recursive_fns.push(name.clone());
                format!("fn {name}() returns Void {{\n    store {file} into {pool}\n    let {v}: Void = {name}()\n}}\n")
            }
            0 => {
                let name = self.name("task");
                let mut body = String::new();
                for _ in 0..1 + self.rng.gen_range(0..4) {
                    body.push_str(&self.statement("    ", false, &mut result_vars));
                }
                let pool = self.pool();
                self.void_fns.push(name.clone());
                format!("fn {name}() returns Void {{\n{body}    retrieve from {pool}\n}}\n")
            }
            1 => {
                let name = self.name("archive");
                let pool = self.pool();
                let mut body = format!("    store target into {pool}\n");
                if self.chance(0.4) {
                    let confirm = self.delete_confirm();
                    body.push_str(&format!("    delete target from {pool}{confirm}\n"));
                }
                self.file_fns.push(name.clone());
                format!("fn {name}(target: File) returns Void {{\n{body}}}\n")
            }
            2 => {
                let name = self.name("attempt");
                let mut body = String::new();
                for _ in 0..self.rng.gen_range(0..4) {
                    body.push_str(&self.statement("    ", true, &mut result_vars));
                }
                let (file, pool) = (self.file(), self.pool());
                let last = self.name("last");
                self.result_fns.push(name.clone());
                format!("fn {name}() returns Result<DnaFile, Str> {{\n{body}    let {last}: Result<DnaFile, Str> = store {file} into {pool}\n}}\n")
            }
            _ => {
                let name = self.name("with_action");
                let ceiling = match self.rng.gen_range(0..3) {
                    0 => Ceiling::None,
                    1 => Ceiling::Hardware,
                    _ => Ceiling::PhysicalKey,
                };
                let v = self.name("done");
                self.hofs.push((name.clone(), ceiling));
                format!("fn {name}(action: Fn() -> Void{}) returns Void {{\n    let {v}: Void = action()\n}}\n", ceiling.suffix())
            }
        }
    }

    fn program(&mut self) -> String {
        let mut out = String::from(
            "pool p0: DnaPool { codec: Ternary, redundancy: 2x, profile: Illumina }\npool p1: DnaPool { codec: Ternary, redundancy: 2x, profile: Twist }\n\
             pool p2: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist, storage: encapsulated at 4C }\n",
        );
        for pool in ["p0", "p1"] {
            for file in 0..4 {
                out.push_str(&format!("store \"f{file}.txt\" into {pool}\n"));
            }
        }
        for _ in 0..2 + self.rng.gen_range(0..4) {
            out.push_str(&self.function());
        }
        let mut result_vars = Vec::new();
        for _ in 0..3 + self.rng.gen_range(0..6) {
            match self.rng.gen_range(0..9) {
                8 => {
                    let (file, years) = (self.file(), [100, 10_000, 1_000_000_000][self.rng.gen_range(0..3)]);
                    out.push_str(&format!("store {file} into p2 {{ retain: {years}y }}\n"));
                }
                0 => {
                    let (a, b, pool) = (self.file(), self.file(), self.pool());
                    out.push_str(&format!("for target in [{a}, {b}] {{\n    store target into {pool}\n}}\n"));
                }
                1 => {
                    let condition = if self.chance(0.5) { "3 > 2" } else { "1 > 2" };
                    let then = self.statement("    ", false, &mut result_vars);
                    let otherwise = self.statement("    ", false, &mut result_vars);
                    out.push_str(&format!("if {condition} {{\n{then}}} else {{\n{otherwise}}}\n"));
                }
                2 => {
                    let (name, file, pool) = (self.name("backup"), self.file(), self.pool());
                    out.push_str(&format!(
                        "pipeline {name} {{\n    encode {file} using Ternary,\n    protect with redundancy 2x,\n    store into {pool}\n}}\n"
                    ));
                }
                _ => out.push_str(&self.statement("", false, &mut result_vars)),
            }
        }
        out
    }
}

#[test]
fn generated_programs_keep_their_effect_and_type_promises_when_run() {
    let dir = std::env::temp_dir().join(format!("nsl-soundness-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for file in 0..4u8 {
        let bytes: Vec<u8> = (0..48u8).map(|i| i.wrapping_mul(37).wrapping_add(file * 11)).collect();
        std::fs::write(dir.join(format!("f{file}.txt")), bytes).unwrap();
    }

    // NSL_SOUNDNESS_PROGRAMS runs a longer sweep by hand; CI runs 300.
    let programs: u64 = std::env::var("NSL_SOUNDNESS_PROGRAMS").ok().and_then(|n| n.parse().ok()).unwrap_or(300);
    let (mut accepted, mut deletes, mut stores, mut over, mut ran) = (0, 0, 0, 0, 0);
    let mut violations = Vec::new();
    for seed in 0..programs {
        let source = Gen::new(seed).program();
        let Some(outcome) = run_monitored(&source, &dir) else { continue };
        accepted += 1;
        deletes += outcome.trace.effects.iter().filter(|e| e.effect == Effect::Destructive).count();
        stores += outcome.trace.effects.iter().filter(|e| e.effect == Effect::Synthesis).count();
        over += outcome.over_approximated;
        ran += outcome.declarations_that_ran;
        if !outcome.violations.is_empty() {
            violations.push(format!("seed {seed}:\n  {}\n--- program ---\n{source}", outcome.violations.join("\n  ")));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "{accepted} of {programs} programs accepted; {stores} stores and {deletes} deletes ran; \
         {over} of {ran} top-level declarations that ran were given a stronger effect than they used"
    );
    assert!(violations.is_empty(), "{} programs broke a promise:\n{}", violations.len(), violations.iter().take(5).cloned().collect::<Vec<_>>().join("\n\n"));
    assert!(accepted >= programs / 3, "only {accepted} of {programs} generated programs were accepted");
    assert!(deletes as u64 >= programs / 3 && stores as u64 >= programs * 3, "too few operations ran: {deletes} deletes, {stores} stores");
}
