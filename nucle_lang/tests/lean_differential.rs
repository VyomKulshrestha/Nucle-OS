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

#[derive(Clone)]
enum Op {
    Store,
    SimulateStore,
    Retrieve,
    Delete(bool),
    Synthesise(bool),
    Sequence(bool),
}

/// A statement of the core calculus.
#[derive(Clone)]
enum St {
    Op(Op),
    Seq(Box<St>, Box<St>),
    /// Either side may run: written in NucleScript as a `match` whose arms
    /// call one helper function each.
    Alt(Box<St>, Box<St>),
    Closure(String, Ceil, Box<St>, Box<St>),
    Alias(String, String, Box<St>),
    /// A closure the checker cannot see into: written as a function that
    /// returns it.
    Opaque(String, Ceil, Box<St>, Box<St>),
    Call(String),
    CallFn(String, Vec<String>),
}

struct Function {
    name: String,
    params: Vec<(String, Ceil)>,
    body: St,
}

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

    fn ceiling(&mut self) -> Ceil {
        match self.rng.gen_range(0..8) {
            0 => Ceil::Hardware,
            1 => Ceil::Synthesis,
            2 => Ceil::Sequencing,
            3 => Ceil::Key,
            _ => Ceil::None,
        }
    }

    fn op(&mut self) -> Op {
        // Now and then a confirmation is missing, which both checkers must refuse.
        let confirmed = !self.chance(0.05);
        match self.rng.gen_range(0..9) {
            0 | 1 => Op::Store,
            2 => Op::SimulateStore,
            3 => Op::Retrieve,
            4 | 5 => Op::Delete(confirmed),
            6 | 7 => Op::Synthesise(confirmed),
            _ => Op::Sequence(confirmed),
        }
    }

    /// A statement that names nothing local: the body of a branch or of a
    /// closure returned by a function, both written as their own functions.
    fn closed(&mut self, depth: u32) -> St {
        let zero_arity: Vec<String> = self.functions.iter().filter(|f| f.params.is_empty()).map(|f| f.name.clone()).collect();
        if depth > 0 && self.chance(0.3) {
            return St::Seq(Box::new(self.closed(depth - 1)), Box::new(self.closed(depth - 1)));
        }
        if !zero_arity.is_empty() && self.chance(0.2) {
            let f = zero_arity[self.rng.gen_range(0..zero_arity.len())].clone();
            return St::CallFn(f, vec![]);
        }
        St::Op(self.op())
    }

    /// A statement in a scope holding the callable names `scope`.
    fn stmt(&mut self, scope: &[String], depth: u32) -> St {
        loop {
            match self.rng.gen_range(0..10) {
                0 | 1 => return St::Op(self.op()),
                2 if depth > 0 => {
                    return St::Seq(Box::new(self.stmt(scope, depth - 1)), Box::new(self.stmt(scope, depth - 1)));
                }
                3 if depth > 0 => return St::Alt(Box::new(self.closed(1)), Box::new(self.closed(1))),
                4 | 5 if depth > 0 => {
                    let x = self.name("c");
                    let ceiling = self.ceiling();
                    let body = self.stmt(scope, depth - 1);
                    let mut inner = scope.to_vec();
                    inner.push(x.clone());
                    let rest = self.stmt(&inner, depth - 1);
                    return St::Closure(x, ceiling, Box::new(body), Box::new(rest));
                }
                6 if depth > 0 && !scope.is_empty() => {
                    let x = self.name("a");
                    let y = scope[self.rng.gen_range(0..scope.len())].clone();
                    let mut inner = scope.to_vec();
                    inner.push(x.clone());
                    let rest = self.stmt(&inner, depth - 1);
                    return St::Alias(x, y, Box::new(rest));
                }
                7 if depth > 0 => {
                    let x = self.name("o");
                    let ceiling = self.ceiling();
                    let body = self.closed(1);
                    let mut inner = scope.to_vec();
                    inner.push(x.clone());
                    let rest = self.stmt(&inner, depth - 1);
                    return St::Opaque(x, ceiling, Box::new(body), Box::new(rest));
                }
                8 if !scope.is_empty() => {
                    let x = scope[self.rng.gen_range(0..scope.len())].clone();
                    return St::Call(x);
                }
                9 if !self.functions.is_empty() => {
                    let f = &self.functions[self.rng.gen_range(0..self.functions.len())];
                    if f.params.len() > scope.len() {
                        continue;
                    }
                    let (name, arity) = (f.name.clone(), f.params.len());
                    let args = (0..arity).map(|_| scope[self.rng.gen_range(0..scope.len())].clone()).collect();
                    return St::CallFn(name, args);
                }
                _ => continue,
            }
        }
    }

    fn program(&mut self) -> St {
        for _ in 0..self.rng.gen_range(0..4) {
            let name = self.name("f");
            let params: Vec<(String, Ceil)> = (0..self.rng.gen_range(0..3)).map(|i| (format!("q{i}"), self.ceiling())).collect();
            let scope: Vec<String> = params.iter().map(|(p, _)| p.clone()).collect();
            let body = self.stmt(&scope, 2);
            self.functions.push(Function { name, params, body });
        }
        self.stmt(&[], 4)
    }
}

/// Writes a statement as NucleScript, collecting the helper functions that
/// branches and returned closures need.
struct Writer {
    helpers: Vec<String>,
    next: usize,
    /// The ceiling on each callable name's type, so another name for it
    /// can repeat that type.
    ceilings: std::collections::HashMap<String, Ceil>,
}

impl Writer {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}_{}", self.next)
    }

    fn op(&mut self, op: &Op, indent: &str) -> String {
        match op {
            Op::Store => format!("{indent}store \"f.txt\" into p0\n"),
            Op::SimulateStore => format!("{indent}simulate store \"f.txt\" into p0\n"),
            Op::Retrieve => format!("{indent}retrieve from p0\n"),
            Op::Delete(c) => format!("{indent}delete \"f.txt\" from p0{}\n", if *c { " confirm physical_key" } else { "" }),
            Op::Synthesise(c) => {
                let v = self.fresh("made");
                let confirm = if *c { if self.next.is_multiple_of(2) { " confirm hardware" } else { " confirm synthesis" } } else { "" };
                format!("{indent}let {v}: Pool<Twist, 0.03%> = synthesise p0 via Twist{confirm}\n")
            }
            Op::Sequence(c) => {
                let v = self.fresh("read");
                let confirm = if *c { if self.next.is_multiple_of(2) { " confirm hardware" } else { " confirm sequencing" } } else { "" };
                format!("{indent}let {v}: Pool<Illumina, 0.35%> = sequence noisy via Illumina{confirm}\n")
            }
        }
    }

    fn stmt(&mut self, st: &St, indent: &str) -> String {
        let deeper = format!("{indent}    ");
        match st {
            St::Op(op) => self.op(op, indent),
            St::Seq(a, b) => format!("{}{}", self.stmt(a, indent), self.stmt(b, indent)),
            St::Alt(a, b) => {
                let (fa, fb, side, v) = (self.fresh("left"), self.fresh("right"), self.fresh("side"), self.fresh("done"));
                let (ba, bb) = (self.stmt(a, "    "), self.stmt(b, "    "));
                self.helpers.push(format!("fn {fa}() returns Void {{\n{ba}}}\n"));
                self.helpers.push(format!("fn {fb}() returns Void {{\n{bb}}}\n"));
                format!("{indent}let {side}: Side = Side::L\n{indent}let {v}: Void = match {side} {{\n{indent}    L => {fa}(),\n{indent}    R => {fb}(),\n{indent}}}\n")
            }
            St::Closure(x, k, body, rest) => {
                self.ceilings.insert(x.clone(), *k);
                let b = self.stmt(body, &deeper);
                format!("{indent}let {x}: Fn() -> Void{} = fn() -> Void {{\n{b}{indent}}}\n{}", k.nsl(), self.stmt(rest, indent))
            }
            St::Alias(x, y, rest) => {
                let k = self.ceilings.get(y).copied().unwrap_or(Ceil::None);
                self.ceilings.insert(x.clone(), k);
                format!("{indent}let {x}: Fn() -> Void{} = {y}\n{}", k.nsl(), self.stmt(rest, indent))
            }
            St::Opaque(x, k, body, rest) => {
                self.ceilings.insert(x.clone(), *k);
                let maker = self.fresh("make");
                let b = self.stmt(body, "        ");
                self.helpers.push(format!(
                    "fn {maker}() returns Fn() -> Void{} {{\n    let made: Fn() -> Void{} = fn() -> Void {{\n{b}    }}\n}}\n",
                    k.nsl(),
                    k.nsl()
                ));
                format!("{indent}let {x}: Fn() -> Void{} = {maker}()\n{}", k.nsl(), self.stmt(rest, indent))
            }
            St::Call(x) => {
                let v = self.fresh("done");
                format!("{indent}let {v}: Void = {x}()\n")
            }
            St::CallFn(f, args) => {
                let v = self.fresh("done");
                format!("{indent}let {v}: Void = {f}({})\n", args.join(", "))
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

fn lean_stmt(st: &St, out: &mut Vec<String>) {
    match st {
        St::Op(op) => out.extend(["op".to_string(), lean_op(op).to_string()]),
        St::Seq(a, b) | St::Alt(a, b) => {
            out.push(if matches!(st, St::Seq(..)) { "seq" } else { "alt" }.to_string());
            lean_stmt(a, out);
            lean_stmt(b, out);
        }
        St::Closure(x, k, body, rest) | St::Opaque(x, k, body, rest) => {
            out.extend([if matches!(st, St::Closure(..)) { "clo" } else { "opq" }.to_string(), x.clone(), k.lean().to_string()]);
            lean_stmt(body, out);
            lean_stmt(rest, out);
        }
        St::Alias(x, y, rest) => {
            out.extend(["alias".to_string(), x.clone(), y.clone()]);
            lean_stmt(rest, out);
        }
        St::Call(x) => out.extend(["call".to_string(), x.clone()]),
        St::CallFn(f, args) => {
            out.extend(["callf".to_string(), f.clone(), args.len().to_string()]);
            out.extend(args.iter().cloned());
        }
    }
}

/// The program as NucleScript and as the Lean checker's input line.
fn render(gen: &Gen, main: &St) -> (String, String) {
    let mut w = Writer { helpers: Vec::new(), next: 0, ceilings: Default::default() };
    let mut functions = String::new();
    let mut lean = Vec::new();
    for f in &gen.functions {
        for (p, k) in &f.params {
            w.ceilings.insert(p.clone(), *k);
        }
        let params: Vec<String> = f.params.iter().map(|(p, k)| format!("{p}: Fn() -> Void{}", k.nsl())).collect();
        let body = w.stmt(&f.body, "    ");
        functions.push_str(&format!("fn {}({}) returns Void {{\n{body}}}\n", f.name, params.join(", ")));
        lean.extend(["fn".to_string(), f.name.clone(), f.params.len().to_string()]);
        for (p, k) in &f.params {
            lean.extend([p.clone(), k.lean().to_string()]);
        }
        lean_stmt(&f.body, &mut lean);
    }
    let body = w.stmt(main, "");
    lean.push("main".to_string());
    lean_stmt(main, &mut lean);
    let source = format!(
        "enum Side {{\n    L,\n    R,\n}}\npool p0: DnaPool {{ codec: Ternary, redundancy: 3x, profile: Illumina }}\n\
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
