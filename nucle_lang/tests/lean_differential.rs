//! Runs the Rust checker and the Lean checker on the same programs and
//! fails if they disagree.
//!
//! The Lean rules in `proofs/NucleScriptCore.lean` were written by hand
//! from `effects.rs` and `typeck.rs`, and `proofs/Checker.lean` computes
//! them (with a proof that it agrees with the rules). This test generates
//! programs in the core those rules cover, writes each one twice, as
//! NucleScript and in the prefix code `proofs/Main.lean` reads, and
//! compares the verdicts: accepted or not, and the worst effect reported.
//!
//! It needs the Lean checker built (`lake build` in `proofs/`) and its path
//! in `NSL_LEAN_CHECKER`; without that it prints a note and passes. CI's
//! `proofs` job builds it and runs this test. `NSL_DIFFERENTIAL_PROGRAMS`
//! sets the number of programs (default 1,000).

use nucle_codec::stable_rng::StableRng;
use nucle_lang::effects::effect_summary;
use nucle_lang::typeck::DiagnosticLevel;
use nucle_lang::{check_source, compile, Effect};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq)]
enum Ceil {
    None,
    Hardware,
    Synthesis,
    Sequencing,
    Key,
}

impl Ceil {
    fn nsl(self) -> &'static str {
        match self {
            Ceil::None => "",
            Ceil::Hardware => " confirm hardware",
            Ceil::Synthesis => " confirm synthesis",
            Ceil::Sequencing => " confirm sequencing",
            Ceil::Key => " confirm physical_key",
        }
    }

    fn lean(self) -> &'static str {
        match self {
            Ceil::None => "none",
            Ceil::Hardware => "hw",
            Ceil::Synthesis => "syn",
            Ceil::Sequencing => "sqn",
            Ceil::Key => "key",
        }
    }
}

/// A pool's profile, or a generic function's type parameter.
#[derive(Clone, PartialEq)]
enum PTy {
    Prof(&'static str),
    Var(&'static str),
}

/// A value's type: a closure's is the ceilings of the closures it takes.
#[derive(Clone, PartialEq)]
enum Ty {
    Fn(Vec<Ceil>),
    Pool(PTy),
    /// `DnaFile`, `Void` or `Str`.
    Base(&'static str),
    Res(Box<Ty>, Box<Ty>),
    /// The program's one enum, `Job { Keep, Wipe(Str) }`.
    Job,
}

impl Ty {
    fn result(ok: &'static str) -> Ty {
        Ty::Res(Box::new(Ty::Base(ok)), Box::new(Ty::Base("Str")))
    }

    /// The NucleScript type, with `k` as the ceiling of a closure's.
    fn nsl(&self, k: Ceil) -> String {
        match self {
            Ty::Fn(cs) => format!("Fn({}) -> Void{}", cs.iter().map(|c| format!("Fn() -> Void{}", c.nsl())).collect::<Vec<_>>().join(", "), k.nsl()),
            Ty::Pool(PTy::Prof(p)) | Ty::Pool(PTy::Var(p)) => format!("Pool<{p}>"),
            Ty::Base(b) => b.to_string(),
            Ty::Res(t, e) => format!("Result<{}, {}>", t.nsl(Ceil::None), e.nsl(Ceil::None)),
            Ty::Job => "Job".to_string(),
        }
    }

    fn lean(&self, out: &mut Vec<String>) {
        match self {
            Ty::Fn(cs) => {
                out.extend(["fn".to_string(), cs.len().to_string()]);
                out.extend(cs.iter().map(|c| c.lean().to_string()));
            }
            Ty::Pool(PTy::Prof(p)) => out.extend(["pool".to_string(), p.to_string()]),
            Ty::Pool(PTy::Var(v)) => out.extend(["poolv".to_string(), v.to_string()]),
            Ty::Base(b) => out.extend(["base".to_string(), b.to_string()]),
            Ty::Res(t, e) => {
                out.push("res".to_string());
                t.lean(out);
                e.lean(out);
            }
            Ty::Job => out.extend(["enum".to_string(), "Job".to_string()]),
        }
    }
}

#[derive(Clone)]
enum Op {
    Store,
    SimulateStore,
    Retrieve,
    Delete(bool),
    Synthesise(bool),
    Sequence(bool),
}

/// The profiles a call gives a generic function's type parameters.
type Inst = Vec<(&'static str, PTy)>;

/// A value bound by `let` that is not a closure.
#[derive(Clone)]
enum Value {
    /// What `store` (`Result<DnaFile, Str>`) or `delete` (`Result<Void,
    /// Str>`) gives back.
    Outcome(Op),
    /// `Job::Keep` or `Job::Wipe("why")`.
    Variant(&'static str),
}

/// A statement of the core calculus.
#[derive(Clone)]
enum St {
    Op(Op),
    /// Nothing, written for Lean as an operation without an effect.
    Skip,
    Seq(Box<St>, Box<St>),
    /// Either side may run: written in NucleScript as a `match` whose arms
    /// call one helper function each.
    Alt(Box<St>, Box<St>),
    /// A closure with its own parameters, each taking a closure, with or
    /// without a ceiling.
    Closure(String, Ceil, Vec<(String, Ceil)>, Box<St>, Box<St>),
    Alias(String, String, Box<St>),
    /// A closure the checker cannot see into: written as a function that
    /// returns it.
    Opaque(String, Ceil, Vec<(String, Ceil)>, Box<St>, Box<St>),
    /// A call to a closure, passing it closures that take none, with the
    /// ceilings of the closure's parameters as its type gives them.
    Call(String, Vec<String>, Vec<Ceil>),
    /// A call to a function, with the profiles its type parameters take,
    /// and, for a function returning a `Result`, the name that holds it
    /// and what follows.
    CallFn(String, Vec<String>, Inst, Option<(String, Box<St>)>),
    /// Runs zero or more times: written as a loop over a pool's files.
    Loop(Box<St>),
    Let(String, Value, Box<St>),
    /// `let x: T = y?`.
    Try(String, Ty, String, Box<St>),
    /// A `match` on an enum or a `Result`: each arm calls a helper
    /// function, and a wildcard arm may end it.
    Match(String, Vec<(&'static str, St)>, Option<Box<St>>),
}

struct Function {
    name: String,
    /// Each parameter's name, ceiling (a closure's), and type.
    params: Vec<(String, Ceil, Ty)>,
    /// Whether it returns `Result<DnaFile, Str>` rather than `Void`.
    returns_result: bool,
    body: St,
}

impl Function {
    fn generic(&self) -> bool {
        self.params.iter().any(|(_, _, t)| matches!(t, Ty::Pool(PTy::Var(_))))
    }
}

/// The pools the program declares, with their profiles.
const POOLS: [(&str, &str); 2] = [("p0", "Illumina"), ("p1", "Twist")];

/// The variants of `Job`, and of a `Result`.
const JOB_TAGS: [&str; 2] = ["Keep", "Wipe"];
const RESULT_TAGS: [&str; 2] = ["Ok", "Err"];

struct Gen {
    rng: StableRng,
    next: usize,
    functions: Vec<Function>,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Gen { rng: StableRng::seed_from_u64(seed), next: 0, functions: Vec::new() }
    }

    fn name(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.gen_f64() < p
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.rng.gen_range(0..items.len())].clone()
    }

    fn ceiling(&mut self) -> Ceil {
        match self.rng.gen_range(0..8) {
            0 => Ceil::Hardware,
            1 => Ceil::Synthesis,
            2 => Ceil::Sequencing,
            3 => Ceil::Key,
            _ => Ceil::None,
        }
    }

    fn confirmed(&mut self) -> bool {
        // Now and then a confirmation is missing, which both checkers must refuse.
        !self.chance(0.05)
    }

    fn op(&mut self) -> Op {
        let confirmed = self.confirmed();
        match self.rng.gen_range(0..9) {
            0 | 1 => Op::Store,
            2 => Op::SimulateStore,
            3 => Op::Retrieve,
            4 | 5 => Op::Delete(confirmed),
            6 | 7 => Op::Synthesise(confirmed),
            _ => Op::Sequence(confirmed),
        }
    }

    /// A statement that names nothing local: the body of a branch, a match
    /// arm or a closure returned by a function, all written as functions
    /// of their own.
    fn closed(&mut self, depth: u32) -> St {
        let zero_arity: Vec<String> = self.functions.iter().filter(|f| f.params.is_empty() && !f.returns_result).map(|f| f.name.clone()).collect();
        if depth > 0 && self.chance(0.3) {
            return St::Seq(Box::new(self.closed(depth - 1)), Box::new(self.closed(depth - 1)));
        }
        if !zero_arity.is_empty() && self.chance(0.2) {
            let f = self.pick(&zero_arity);
            return St::CallFn(f, vec![], vec![], None);
        }
        St::Op(self.op())
    }

    /// A closure's own parameters, sometimes none, each with a ceiling now
    /// and then. When everything in `scope` is a closure parameter, one of
    /// those names is now and then reused, so the closure's parameter
    /// hides it.
    fn own_params(&mut self, scope: &[(String, Ty)]) -> Vec<(String, Ceil)> {
        if !self.chance(0.4) {
            return Vec::new();
        }
        let only_params = !scope.is_empty() && scope.iter().all(|(x, t)| (x.starts_with('q') || x.starts_with('k')) && matches!(t, Ty::Fn(_)));
        let mut params: Vec<(String, Ceil)> = Vec::new();
        for _ in 0..self.rng.gen_range(1..3) {
            let ceiling = if self.chance(0.5) { self.ceiling() } else { Ceil::None };
            if only_params && self.chance(0.4) {
                let reused = scope[self.rng.gen_range(0..scope.len())].0.clone();
                if !params.iter().any(|(p, _)| *p == reused) {
                    params.push((reused, ceiling));
                    continue;
                }
            }
            params.push((self.name("k"), ceiling));
        }
        params
    }

    /// One closure from `scope` of each type in `types`, to pass to a call,
    /// or `None` if the scope has none of some type.
    fn args(&mut self, scope: &[(String, Ty)], types: &[Ty]) -> Option<Vec<String>> {
        types
            .iter()
            .map(|ty| {
                let fitting: Vec<String> = scope.iter().filter(|(_, t)| t == ty).map(|(x, _)| x.clone()).collect();
                if fitting.is_empty() {
                    None
                } else {
                    Some(self.pick(&fitting))
                }
            })
            .collect()
    }

    /// Arguments for a call to `f` from `scope`, and the profiles its type
    /// parameters take: the first pool passed for a `Pool<P>` fixes `P`,
    /// as the Rust checker infers it. Now and then a pool of another
    /// profile is passed, which both checkers must refuse.
    fn fn_args(&mut self, scope: &[(String, Ty)], params: &[(String, Ceil, Ty)]) -> Option<(Vec<String>, Inst)> {
        let mut inst: Inst = Vec::new();
        let mut args = Vec::new();
        for (_, _, want) in params {
            let pools: Vec<(String, PTy)> = scope.iter().filter_map(|(x, t)| if let Ty::Pool(p) = t { Some((x.clone(), p.clone())) } else { None }).collect();
            let arg = match want {
                Ty::Pool(PTy::Var(v)) => match inst.iter().find(|(w, _)| w == v).map(|(_, p)| p.clone()) {
                    Some(bound) => {
                        let fitting: Vec<String> = pools.iter().filter(|(_, p)| *p == bound || self.chance(0.03)).map(|(x, _)| x.clone()).collect();
                        if fitting.is_empty() {
                            return None;
                        }
                        self.pick(&fitting)
                    }
                    None => {
                        if pools.is_empty() {
                            return None;
                        }
                        let (x, p) = self.pick(&pools);
                        inst.push((v, p));
                        x
                    }
                },
                Ty::Pool(PTy::Prof(_)) if self.chance(0.03) && !pools.is_empty() => self.pick(&pools).0,
                _ => self.args(scope, std::slice::from_ref(want))?.remove(0),
            };
            args.push(arg);
        }
        Some((args, inst))
    }

    /// A statement in a scope holding the names `scope`, each with its
    /// type; `in_result` when a `?` may end the function it is in.
    fn stmt(&mut self, scope: &[(String, Ty)], depth: u32, in_result: bool) -> St {
        loop {
            match self.rng.gen_range(0..17) {
                10 if depth > 0 => return St::Loop(Box::new(self.stmt(scope, depth - 1, false))),
                0 | 1 => return St::Op(self.op()),
                2 if depth > 0 => {
                    return St::Seq(Box::new(self.stmt(scope, depth - 1, in_result)), Box::new(self.stmt(scope, depth - 1, in_result)));
                }
                3 if depth > 0 => return St::Alt(Box::new(self.closed(1)), Box::new(self.closed(1))),
                4 | 5 if depth > 0 => {
                    let x = self.name("c");
                    let ceiling = self.ceiling();
                    let params = self.own_params(scope);
                    let mut body_scope: Vec<(String, Ty)> = scope.iter().filter(|(x, _)| !params.iter().any(|(p, _)| p == x)).cloned().collect();
                    body_scope.extend(params.iter().map(|(p, _)| (p.clone(), Ty::Fn(Vec::new()))));
                    let body = self.stmt(&body_scope, depth - 1, false);
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), Ty::Fn(params.iter().map(|(_, c)| *c).collect())));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Closure(x, ceiling, params, Box::new(body), Box::new(rest));
                }
                6 if depth > 0 => {
                    // Another name for a closure or an enum value.
                    let named: Vec<(String, Ty)> = scope.iter().filter(|(_, t)| matches!(t, Ty::Fn(_) | Ty::Job)).cloned().collect();
                    if named.is_empty() {
                        continue;
                    }
                    let x = self.name("a");
                    let (y, ty) = self.pick(&named);
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), ty));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Alias(x, y, Box::new(rest));
                }
                7 if depth > 0 => {
                    let x = self.name("o");
                    let ceiling = self.ceiling();
                    let params = self.own_params(&[]);
                    // Written inside a function of its own, the body sees
                    // only its own parameters.
                    let own: Vec<(String, Ty)> = params.iter().map(|(p, _)| (p.clone(), Ty::Fn(Vec::new()))).collect();
                    let body = if own.is_empty() { self.closed(1) } else { self.stmt(&own, 1, false) };
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), Ty::Fn(params.iter().map(|(_, c)| *c).collect())));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Opaque(x, ceiling, params, Box::new(body), Box::new(rest));
                }
                8 => {
                    let callable: Vec<(String, Vec<Ceil>)> = scope.iter().filter_map(|(x, t)| if let Ty::Fn(cs) = t { Some((x.clone(), cs.clone())) } else { None }).collect();
                    if callable.is_empty() {
                        continue;
                    }
                    let (x, ty) = self.pick(&callable);
                    let Some(mut args) = self.args(scope, &vec![Ty::Fn(Vec::new()); ty.len()]) else { continue };
                    // Now and then a call passes one closure too many, which
                    // both checkers must refuse.
                    if self.chance(0.02) {
                        if let Some(extra) = self.args(scope, &[Ty::Fn(Vec::new())]) {
                            args.extend(extra);
                        }
                    }
                    return St::Call(x, args, ty);
                }
                9 if !self.functions.is_empty() => {
                    let f = self.rng.gen_range(0..self.functions.len());
                    let (name, params, returns_result) = (self.functions[f].name.clone(), self.functions[f].params.clone(), self.functions[f].returns_result);
                    let Some((args, inst)) = self.fn_args(scope, &params) else { continue };
                    let bound = if returns_result && depth > 0 {
                        let v = self.name("got");
                        let mut inner = scope.to_vec();
                        inner.push((v.clone(), Ty::result("DnaFile")));
                        Some((v, Box::new(self.stmt(&inner, depth - 1, in_result))))
                    } else if returns_result {
                        continue;
                    } else {
                        None
                    };
                    return St::CallFn(name, args, inst, bound);
                }
                11 if depth > 0 => {
                    // What a store or a delete gives back.
                    let x = self.name("r");
                    let (op, ty) = if self.chance(0.5) { (Op::Store, Ty::result("DnaFile")) } else { (Op::Delete(self.confirmed()), Ty::result("Void")) };
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), ty));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Let(x, Value::Outcome(op), Box::new(rest));
                }
                12 if depth > 0 => {
                    let x = self.name("j");
                    let tag = self.pick(&JOB_TAGS);
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), Ty::Job));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Let(x, Value::Variant(tag), Box::new(rest));
                }
                13 | 16 if depth > 0 && in_result => {
                    let results: Vec<(String, Ty)> = scope.iter().filter(|(_, t)| matches!(t, Ty::Res(..))).cloned().collect();
                    if results.is_empty() {
                        // Store or delete first, and unwrap what it gives back.
                        let r = self.name("r");
                        let (op, ty) = if self.chance(0.5) { (Op::Store, Ty::result("DnaFile")) } else { (Op::Delete(self.confirmed()), Ty::result("Void")) };
                        let mut inner = scope.to_vec();
                        inner.push((r.clone(), ty));
                        let rest = self.stmt(&inner, depth, in_result);
                        return St::Let(r, Value::Outcome(op), Box::new(rest));
                    }
                    let (y, ty) = self.pick(&results);
                    let Ty::Res(ok, _) = ty else { unreachable!() };
                    // Now and then the declared type is not what `?` gives,
                    // which both checkers must refuse.
                    let declared = if self.chance(0.03) { Ty::Base(if *ok == Ty::Base("Void") { "DnaFile" } else { "Void" }) } else { *ok };
                    let x = self.name("u");
                    let mut inner = scope.to_vec();
                    inner.push((x.clone(), declared.clone()));
                    let rest = self.stmt(&inner, depth - 1, in_result);
                    return St::Try(x, declared, y, Box::new(rest));
                }
                14 | 15 if depth > 0 => {
                    let matchable: Vec<(String, Ty)> = scope.iter().filter(|(_, t)| matches!(t, Ty::Job | Ty::Res(..))).cloned().collect();
                    if matchable.is_empty() {
                        continue;
                    }
                    let (y, ty) = self.pick(&matchable);
                    let tags = if ty == Ty::Job { JOB_TAGS } else { RESULT_TAGS };
                    let mut arms: Vec<(&'static str, St)> = tags.iter().map(|t| (*t, self.closed(1))).collect();
                    let mut wildcard = None;
                    if self.chance(0.25) {
                        // A wildcard arm for the last variant.
                        arms.pop();
                        wildcard = Some(Box::new(self.closed(1)));
                    } else if self.chance(0.05) {
                        // An arm left out, which both checkers must refuse.
                        let i = self.rng.gen_range(0..arms.len());
                        arms.remove(i);
                    }
                    return St::Match(y, arms, wildcard);
                }
                _ => continue,
            }
        }
    }

    fn param_type(&mut self) -> Ty {
        match self.rng.gen_range(0..10) {
            // A closure that is sometimes passed a closure itself, with or
            // without a ceiling: `Fn(Fn() -> Void confirm synthesis) -> Void`.
            0..=4 => Ty::Fn(if self.chance(0.3) { vec![if self.chance(0.5) { self.ceiling() } else { Ceil::None }] } else { Vec::new() }),
            5 => Ty::Pool(PTy::Prof(self.pick(&POOLS).1)),
            6 | 7 => Ty::Pool(PTy::Var("P")),
            8 => Ty::result("DnaFile"),
            _ => Ty::Job,
        }
    }

    fn program(&mut self) -> St {
        for _ in 0..self.rng.gen_range(0..4) {
            let name = self.name("f");
            let params: Vec<(String, Ceil, Ty)> = (0..self.rng.gen_range(0..3))
                .map(|i| {
                    let ty = self.param_type();
                    let ceiling = if matches!(ty, Ty::Fn(_)) { self.ceiling() } else { Ceil::None };
                    (format!("q{i}"), ceiling, ty)
                })
                .collect();
            let returns_result = self.chance(0.25);
            let scope: Vec<(String, Ty)> = params.iter().map(|(p, _, t)| (p.clone(), t.clone())).collect();
            let mut body = self.stmt(&scope, 2, returns_result);
            if returns_result {
                // The value the function returns: its last binding.
                let last = St::Let(self.name("last"), Value::Outcome(Op::Store), Box::new(St::Skip));
                body = St::Seq(Box::new(body), Box::new(last));
            }
            self.functions.push(Function { name, params, returns_result, body });
        }
        let scope: Vec<(String, Ty)> = POOLS.iter().map(|(p, prof)| (p.to_string(), Ty::Pool(PTy::Prof(prof)))).collect();
        self.stmt(&scope, 4, false)
    }
}

/// Writes a statement as NucleScript, collecting the helper functions that
/// branches, match arms and returned closures need.
struct Writer {
    helpers: Vec<String>,
    next: usize,
    /// The type of each name and a closure's ceiling, so another name for
    /// it can repeat them.
    types: std::collections::HashMap<String, (Ty, Ceil)>,
}

/// A closure literal's parameter list.
fn param_list(params: &[(String, Ceil)]) -> String {
    params.iter().map(|(p, c)| format!("{p}: Fn() -> Void{}", c.nsl())).collect::<Vec<_>>().join(", ")
}

/// A closure's type, from its own parameters.
fn own_type(params: &[(String, Ceil)]) -> Ty {
    Ty::Fn(params.iter().map(|(_, c)| *c).collect())
}

impl Writer {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}_{}", self.next)
    }

    /// An operation, as a statement or, for `store` and `delete`, as the
    /// expression a `let` binds.
    fn op_text(&mut self, op: &Op) -> String {
        match op {
            Op::Store => "store \"f.txt\" into p0".to_string(),
            Op::SimulateStore => "simulate store \"f.txt\" into p0".to_string(),
            Op::Retrieve => "retrieve from p0".to_string(),
            Op::Delete(c) => format!("delete \"f.txt\" from p0{}", if *c { " confirm physical_key" } else { "" }),
            Op::Synthesise(c) => {
                let v = self.fresh("made");
                let confirm = if *c { if self.next.is_multiple_of(2) { " confirm hardware" } else { " confirm synthesis" } } else { "" };
                format!("let {v}: Pool<Twist, 0.03%> = synthesise p0 via Twist{confirm}")
            }
            Op::Sequence(c) => {
                let v = self.fresh("read");
                let confirm = if *c { if self.next.is_multiple_of(2) { " confirm hardware" } else { " confirm sequencing" } } else { "" };
                format!("let {v}: Pool<Illumina, 0.35%> = sequence noisy via Illumina{confirm}")
            }
        }
    }

    /// A helper function holding a closed statement, called from a branch
    /// or a match arm.
    fn helper(&mut self, body: &St) -> String {
        let f = self.fresh("arm");
        let b = self.stmt(body, "    ");
        self.helpers.push(format!("fn {f}() returns Void {{\n{b}}}\n"));
        f
    }

    fn stmt(&mut self, st: &St, indent: &str) -> String {
        let deeper = format!("{indent}    ");
        match st {
            St::Op(op) => format!("{indent}{}\n", self.op_text(op)),
            St::Skip => String::new(),
            St::Seq(a, b) => format!("{}{}", self.stmt(a, indent), self.stmt(b, indent)),
            St::Alt(a, b) => {
                let (side, v) = (self.fresh("side"), self.fresh("done"));
                let (fa, fb) = (self.helper(a), self.helper(b));
                format!("{indent}let {side}: Side = Side::L\n{indent}let {v}: Void = match {side} {{\n{indent}    L => {fa}(),\n{indent}    R => {fb}(),\n{indent}}}\n")
            }
            St::Closure(x, k, params, body, rest) => {
                self.types.insert(x.clone(), (own_type(params), *k));
                // A parameter may hide a name in scope; its type comes back after.
                let hidden: Vec<(String, Option<(Ty, Ceil)>)> = params.iter().map(|(p, _)| (p.clone(), self.types.get(p).cloned())).collect();
                for (p, c) in params {
                    self.types.insert(p.clone(), (Ty::Fn(Vec::new()), *c));
                }
                let b = self.stmt(body, &deeper);
                for (p, old) in hidden {
                    match old {
                        Some(t) => {
                            self.types.insert(p, t);
                        }
                        None => {
                            self.types.remove(&p);
                        }
                    }
                }
                let ty = own_type(params).nsl(*k);
                format!("{indent}let {x}: {ty} = fn({}) -> Void {{\n{b}{indent}}}\n{}", param_list(params), self.stmt(rest, indent))
            }
            St::Alias(x, y, rest) => {
                let (ty, k) = self.types.get(y).cloned().unwrap_or((Ty::Fn(Vec::new()), Ceil::None));
                let written = ty.nsl(k);
                self.types.insert(x.clone(), (ty, k));
                format!("{indent}let {x}: {written} = {y}\n{}", self.stmt(rest, indent))
            }
            St::Opaque(x, k, params, body, rest) => {
                self.types.insert(x.clone(), (own_type(params), *k));
                for (p, c) in params {
                    self.types.insert(p.clone(), (Ty::Fn(Vec::new()), *c));
                }
                let maker = self.fresh("make");
                let b = self.stmt(body, "        ");
                let ty = own_type(params).nsl(*k);
                self.helpers.push(format!("fn {maker}() returns {ty} {{\n    let made: {ty} = fn({}) -> Void {{\n{b}    }}\n}}\n", param_list(params)));
                format!("{indent}let {x}: {ty} = {maker}()\n{}", self.stmt(rest, indent))
            }
            St::Call(x, args, _) => {
                let v = self.fresh("done");
                format!("{indent}let {v}: Void = {x}({})\n", args.join(", "))
            }
            St::CallFn(f, args, _, bound) => match bound {
                None => {
                    let v = self.fresh("done");
                    format!("{indent}let {v}: Void = {f}({})\n", args.join(", "))
                }
                Some((v, rest)) => {
                    self.types.insert(v.clone(), (Ty::result("DnaFile"), Ceil::None));
                    format!("{indent}let {v}: Result<DnaFile, Str> = {f}({})\n{}", args.join(", "), self.stmt(rest, indent))
                }
            },
            St::Loop(body) => {
                let file = self.fresh("file");
                let b = self.stmt(body, &deeper);
                format!("{indent}for {file} in files of p0 {{\n{b}{indent}}}\n")
            }
            St::Let(x, value, rest) => {
                let line = match value {
                    Value::Outcome(op) => {
                        let ty = if matches!(op, Op::Store) { Ty::result("DnaFile") } else { Ty::result("Void") };
                        let text = format!("{indent}let {x}: {} = {}\n", ty.nsl(Ceil::None), self.op_text(op));
                        self.types.insert(x.clone(), (ty, Ceil::None));
                        text
                    }
                    Value::Variant(tag) => {
                        self.types.insert(x.clone(), (Ty::Job, Ceil::None));
                        let value = if *tag == "Wipe" { "Job::Wipe(\"why\")".to_string() } else { format!("Job::{tag}") };
                        format!("{indent}let {x}: Job = {value}\n")
                    }
                };
                format!("{line}{}", self.stmt(rest, indent))
            }
            St::Try(x, ty, y, rest) => format!("{indent}let {x}: {} = {y}?\n{}", ty.nsl(Ceil::None), self.stmt(rest, indent)),
            St::Match(y, arms, wildcard) => {
                let v = self.fresh("done");
                let mut lines = String::new();
                for (tag, body) in arms {
                    let f = self.helper(body);
                    let pattern = if *tag == "Keep" { tag.to_string() } else { format!("{tag}({})", self.fresh("held")) };
                    lines.push_str(&format!("{indent}    {pattern} => {f}(),\n"));
                }
                if let Some(body) = wildcard {
                    let f = self.helper(body);
                    lines.push_str(&format!("{indent}    _ => {f}(),\n"));
                }
                format!("{indent}let {v}: Void = match {y} {{\n{lines}{indent}}}\n")
            }
        }
    }
}

fn lean_op(op: &Op) -> &'static str {
    match op {
        Op::Store => "store",
        Op::SimulateStore => "sim",
        Op::Retrieve => "get",
        Op::Delete(true) => "del1",
        Op::Delete(false) => "del0",
        Op::Synthesise(true) => "syn1",
        Op::Synthesise(false) => "syn0",
        Op::Sequence(true) => "sqn1",
        Op::Sequence(false) => "sqn0",
    }
}

fn lean_stmt(st: &St, out: &mut Vec<String>, next: &mut usize) {
    match st {
        St::Op(op) => out.extend(["op".to_string(), lean_op(op).to_string()]),
        St::Skip => out.extend(["op".to_string(), "get".to_string()]),
        St::Seq(a, b) | St::Alt(a, b) => {
            out.push(if matches!(st, St::Seq(..)) { "seq" } else { "alt" }.to_string());
            lean_stmt(a, out, next);
            lean_stmt(b, out, next);
        }
        St::Closure(x, k, params, body, rest) | St::Opaque(x, k, params, body, rest) => {
            out.extend([if matches!(st, St::Closure(..)) { "clo" } else { "opq" }.to_string(), x.clone(), k.lean().to_string(), params.len().to_string()]);
            for (p, c) in params {
                out.extend([p.clone(), c.lean().to_string()]);
            }
            lean_stmt(body, out, next);
            lean_stmt(rest, out, next);
        }
        St::Alias(x, y, rest) => {
            out.extend(["alias".to_string(), x.clone(), y.clone()]);
            lean_stmt(rest, out, next);
        }
        St::Call(x, args, ty) => {
            out.extend(["call".to_string(), x.clone(), args.len().to_string()]);
            out.extend(args.iter().cloned());
            out.push(ty.len().to_string());
            out.extend(ty.iter().map(|c| c.lean().to_string()));
        }
        St::CallFn(f, args, inst, bound) => {
            let mut call = vec!["callf".to_string(), f.clone(), args.len().to_string()];
            call.extend(args.iter().cloned());
            call.push(inst.len().to_string());
            for (v, p) in inst {
                match p {
                    PTy::Prof(p) => call.extend([v.to_string(), "prof".to_string(), p.to_string()]),
                    PTy::Var(q) => call.extend([v.to_string(), "var".to_string(), q.to_string()]),
                }
            }
            match bound {
                None => out.extend(call),
                Some((v, rest)) => {
                    // The value the function returns is a `Result` of its
                    // declared type, which the core does not follow.
                    out.push("seq".to_string());
                    out.extend(call);
                    out.extend(["val".to_string(), v.clone(), "out".to_string(), "DnaFile".to_string()]);
                    lean_stmt(rest, out, next);
                }
            }
        }
        St::Loop(body) => {
            out.push("loop".to_string());
            lean_stmt(body, out, next);
        }
        St::Let(x, value, rest) => {
            match value {
                Value::Outcome(op) => {
                    out.extend(["seq".to_string(), "op".to_string(), lean_op(op).to_string()]);
                    let ok = if matches!(op, Op::Store) { "DnaFile" } else { "Void" };
                    out.extend(["val".to_string(), x.clone(), "out".to_string(), ok.to_string()]);
                }
                Value::Variant(tag) => {
                    let carried = if *tag == "Wipe" { "Str" } else { "Void" };
                    out.extend(["val".to_string(), x.clone(), "var".to_string(), "Job".to_string(), tag.to_string(), carried.to_string()]);
                }
            }
            lean_stmt(rest, out, next);
        }
        St::Try(x, ty, y, rest) => {
            out.extend(["try".to_string(), x.clone()]);
            ty.lean(out);
            out.push(y.clone());
            lean_stmt(rest, out, next);
        }
        St::Match(y, arms, wildcard) => {
            for (tag, body) in arms {
                *next += 1;
                out.extend(["mt".to_string(), y.clone(), tag.to_string(), format!("z{next}")]);
                lean_stmt(body, out, next);
            }
            match wildcard {
                Some(body) => lean_stmt(body, out, next),
                None => out.extend(["nomatch".to_string(), y.clone()]),
            }
        }
    }
}

/// The program as NucleScript and as the Lean checker's input line.
fn render(gen: &Gen, main: &St) -> (String, String) {
    let mut w = Writer { helpers: Vec::new(), next: 0, types: Default::default() };
    let mut functions = String::new();
    let mut lean = vec!["enum", "Job", "2", "Keep", "Void", "Wipe", "Str"].into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut next = 0;
    for f in &gen.functions {
        for (p, k, t) in &f.params {
            w.types.insert(p.clone(), (t.clone(), *k));
        }
        let params: Vec<String> = f.params.iter().map(|(p, k, t)| format!("{p}: {}", t.nsl(*k))).collect();
        let body = w.stmt(&f.body, "    ");
        let generic = if f.generic() { "<P>" } else { "" };
        let returns = if f.returns_result { "Result<DnaFile, Str>" } else { "Void" };
        functions.push_str(&format!("fn {}{generic}({}) returns {returns} {{\n{body}}}\n", f.name, params.join(", ")));
        lean.extend(["fn".to_string(), f.name.clone(), f.params.len().to_string()]);
        for (p, k, t) in &f.params {
            lean.extend([p.clone(), k.lean().to_string()]);
            t.lean(&mut lean);
        }
        lean_stmt(&f.body, &mut lean, &mut next);
    }
    let body = w.stmt(main, "");
    lean.push("main".to_string());
    for (p, prof) in POOLS {
        lean.extend(["val".to_string(), p.to_string(), "pool".to_string(), prof.to_string()]);
    }
    lean_stmt(main, &mut lean, &mut next);
    let source = format!(
        "enum Side {{\n    L,\n    R,\n}}\nenum Job {{\n    Keep,\n    Wipe(Str),\n}}\n\
         pool p0: DnaPool {{ codec: Ternary, redundancy: 3x, profile: Illumina }}\n\
         pool p1: DnaPool {{ codec: Ternary, redundancy: 3x, profile: Twist }}\n\
         let noisy: Pool<Illumina, 0.35%> = simulate p0 under Illumina\n{functions}{}{body}",
        w.helpers.join("")
    );
    (source, lean.join(" "))
}

/// The Rust checker's verdict, in the Lean checker's words.
fn rust_verdict(source: &str) -> String {
    let report = check_source(source);
    if report.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error) {
        return "reject".to_string();
    }
    let plan = compile(source).expect("a program check_source accepts compiles");
    let summary = effect_summary(&plan.program);
    // Only what the program itself runs: a function declaration is not a
    // call, and the Lean program's effect leaves uncalled functions out.
    let worst = summary
        .declarations
        .iter()
        .filter(|d| d.kind != "function")
        .map(|d| d.effect)
        .max_by_key(|e| match e {
            Effect::Pure => 0,
            Effect::Sequencing => 1,
            Effect::Synthesis => 2,
            Effect::Destructive => 3,
        })
        .unwrap_or(Effect::Pure);
    format!("accept {worst:?}")
}

#[test]
fn the_rust_and_lean_checkers_agree_on_generated_programs() {
    let Ok(checker) = std::env::var("NSL_LEAN_CHECKER") else {
        eprintln!("NSL_LEAN_CHECKER is not set: build proofs/ with `lake build` and point it at .lake/build/bin/nsl-check to run this test");
        return;
    };
    let programs: u64 = std::env::var("NSL_DIFFERENTIAL_PROGRAMS").ok().and_then(|n| n.parse().ok()).unwrap_or(1000);
    let rendered: Vec<(String, String)> = (0..programs)
        .map(|seed| {
            let mut gen = Gen::new(seed);
            let main = gen.program();
            render(&gen, &main)
        })
        .collect();

    let mut child = Command::new(&checker).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("the Lean checker runs");
    // Feed the programs from another thread: the checker answers each line
    // as it reads it, so writing them all before reading any answers would
    // fill its output pipe and stall both sides.
    let input: Vec<String> = rendered.iter().map(|(_, lean)| lean.clone()).collect();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        for line in input {
            writeln!(stdin, "{line}").unwrap();
        }
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let lean_verdicts: Vec<String> = String::from_utf8(output.stdout).unwrap().lines().map(str::to_string).collect();
    assert_eq!(lean_verdicts.len(), rendered.len(), "the Lean checker answered {} of {} programs", lean_verdicts.len(), rendered.len());

    let (mut accepted, mut mismatches) = (0, Vec::new());
    for (seed, ((source, lean), lean_verdict)) in rendered.iter().zip(&lean_verdicts).enumerate() {
        let rust = rust_verdict(source);
        if rust.starts_with("accept") {
            accepted += 1;
        }
        if &rust != lean_verdict {
            let errors: Vec<String> = check_source(source).diagnostics.into_iter().filter(|d| d.level == DiagnosticLevel::Error).map(|d| format!("{}: {}", d.code, d.message)).collect();
            mismatches.push(format!("seed {seed}: Rust says {rust}, Lean says {lean_verdict}\n  Rust errors: {errors:?}\n  core: {lean}\n--- NucleScript ---\n{source}"));
        }
    }
    eprintln!("{} programs: {accepted} accepted by both checkers' rules, {} disagreements", rendered.len(), mismatches.len());
    assert!(accepted as u64 >= programs / 5, "only {accepted} of {programs} programs were accepted");
    assert!(mismatches.is_empty(), "{} disagreements:\n{}", mismatches.len(), mismatches.iter().take(4).cloned().collect::<Vec<_>>().join("\n\n"));
}
