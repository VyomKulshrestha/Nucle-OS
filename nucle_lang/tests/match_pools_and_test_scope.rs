//! Pool-valued `match` expressions and `test` block scoping.
//!
//! Each case here was found by writing larger NucleScript programs that
//! combine features the smaller per-feature tests exercise one at a time:
//! a `match` whose arms recover a pool at different coverages, a generic
//! function returning that match's pool, and a `test` asserting on a pool
//! the program simulated at top level.

use nucle_lang::check_source;

fn errors(src: &str) -> Vec<String> {
    check_source(src)
        .diagnostics
        .into_iter()
        .filter(|d| d.level == nucle_lang::typeck::DiagnosticLevel::Error)
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

const PRELUDE: &str = "\
pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
enum Depth {
    Shallow,
    Deep(Str),
}
let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
";

#[test]
fn consensus_vote_in_a_match_arm_is_a_recovered_pool() {
    let src = format!(
        "{PRELUDE}
let plan: Depth = Depth::Shallow
let recovered: Pool<Recovered> = match plan {{
    Shallow => consensus_vote(noisy, coverage: 10x),
    Deep(note) => consensus_vote(noisy, coverage: 10x),
}}
"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn arms_at_different_coverages_join_at_the_worse_error_rate() {
    // 2x coverage leaves more residual error than 30x; the match can only
    // promise the worse of the two, so annotating the better rate fails
    // and annotating the worse one passes.
    let worse = check_source(&format!("{PRELUDE}let r: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)\n"));
    assert!(worse.diagnostics.iter().all(|d| d.level != nucle_lang::typeck::DiagnosticLevel::Error));
    let better_src = format!(
        "{PRELUDE}
let plan: Depth = Depth::Shallow
let r: Pool<Recovered, 0.0001%> = match plan {{
    Shallow => consensus_vote(noisy, coverage: 2x),
    Deep(note) => consensus_vote(noisy, coverage: 30x),
}}
"
    );
    let errs = errors(&better_src);
    assert!(errs.iter().any(|e| e.starts_with("E-BINDING-TYPE-MISMATCH")), "{errs:?}");
}

#[test]
fn match_arms_in_different_pool_states_still_mismatch() {
    let src = format!(
        "{PRELUDE}
let plan: Depth = Depth::Shallow
let r: Pool<Recovered> = match plan {{
    Shallow => consensus_vote(noisy, coverage: 10x),
    Deep(note) => simulate archive under Illumina,
}}
"
    );
    let errs = errors(&src);
    assert!(errs.iter().any(|e| e.starts_with("E-MATCH-ARM-TYPE-MISMATCH")), "{errs:?}");
}

#[test]
fn a_generic_function_can_return_a_pool_valued_match() {
    let src = format!(
        "{PRELUDE}
fn polish<P>(reads: Pool<P>, depth: Depth) returns Pool<Recovered> {{
    let out: Pool<Recovered> = match depth {{
        Shallow => consensus_vote(reads, coverage: 10x),
        Deep(note) => consensus_vote(reads, coverage: 30x),
    }}
}}
let recovered: Pool<Recovered> = polish(noisy, Depth::Deep(\"R10.4.1\"))
"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn a_pool_valued_match_binding_can_drive_a_condition() {
    let src = format!(
        "{PRELUDE}
let plan: Depth = Depth::Shallow
let r: Pool<Recovered> = match plan {{
    Shallow => consensus_vote(noisy, coverage: 10x),
    _ => consensus_vote(noisy, coverage: 20x),
}}
assert r < noisy, \"consensus must lower the error rate\"
"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn a_test_block_sees_top_level_simulated_pools() {
    let src = format!(
        "{PRELUDE}
let deep: Pool<Recovered> = consensus_vote(noisy, coverage: 20x)
test \"consensus helps\" {{
    assert deep < noisy, \"recovered pool must be cleaner than raw reads\"
}}
"
    );
    assert!(errors(&src).is_empty(), "{:?}", errors(&src));
}

#[test]
fn a_test_block_still_cannot_see_another_tests_locals() {
    let src = format!(
        "{PRELUDE}
test \"first\" {{
    let local: Pool<Recovered> = consensus_vote(noisy, coverage: 20x)
}}
test \"second\" {{
    assert local < 1.0
}}
"
    );
    let errs = errors(&src);
    assert!(errs.iter().any(|e| e.starts_with("E-CONDITION-UNDECLARED")), "{errs:?}");
}

#[test]
fn a_failing_assertion_on_a_top_level_pool_is_reported() {
    let src = format!(
        "{PRELUDE}
test \"wrong expectation\" {{
    assert noisy < 0.01, \"illumina reads are not that clean\"
}}
"
    );
    let errs = errors(&src);
    assert!(errs.iter().any(|e| e.starts_with("E-ASSERTION-FAILED")), "{errs:?}");
}
