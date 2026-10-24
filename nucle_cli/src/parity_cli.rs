//! `nucle parity`: how much Reed-Solomon parity a file needs to come back
//! through a given synthesis-and-sequencing channel.
//!
//! Wraps `NucleOS::plan_parity`: the file's own strands go through the
//! simulated channel and the per-strand decoder `nucle retrieve` uses, and
//! the answer is the smallest parity per stripe that keeps the chance of
//! losing the file under `--max-failure`. The report sets that beside the
//! parity `nucle store` uses by default.

use nucle_synth::noise::SimulationConfig;
use nucle_synth::planning::{failure_probability, ParityPlan, RS_MAX_SYMBOLS};
use nucle_vfs::syscall::{CodecKind, NucleOS};

/// Parity per stripe `nucle store` uses without `--redundancy`.
const STORE_DEFAULT_PARITY: usize = 2;

pub struct ParityArgs {
    pub file: String,
    pub codec: String,
    pub profile: String,
    pub coverage: u32,
    pub max_failure: f64,
}

pub fn cmd_parity(args: ParityArgs, json: bool) {
    let data = match std::fs::read(&args.file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", args.file, e);
            std::process::exit(1);
        }
    };
    let Some(codec_kind) = CodecKind::from_codec_name(&args.codec) else {
        eprintln!("Unknown codec '{}'; expected ternary, fountain, yin-yang, or hedges", args.codec);
        std::process::exit(1);
    };
    if args.coverage == 0 {
        eprintln!("--coverage must be at least 1");
        std::process::exit(1);
    }

    let profile = crate::parse_hw_profile(&args.profile);
    let os = NucleOS::new(10).with_noise(SimulationConfig::realistic(profile, args.coverage, 42));
    let plan = match os.plan_parity(&data, codec_kind, args.max_failure) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("Parity plan failed: {}", e);
            std::process::exit(1);
        }
    };
    let default_failure = failure_probability(plan.data_strands, STORE_DEFAULT_PARITY, &plan.outcomes);
    let o = plan.outcomes;

    if json {
        let value = serde_json::json!({
            "file": args.file,
            "codec": args.codec,
            "profile": args.profile,
            "coverage": args.coverage,
            "max_failure": args.max_failure,
            "data_strands": plan.data_strands,
            "unread": o.unread,
            "undecoded": o.undecoded,
            "wrong": o.wrong,
            "parity": plan.parity,
            "failure_probability": plan.failure_probability,
            "default_parity": STORE_DEFAULT_PARITY,
            "default_failure_probability": default_failure,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return;
    }

    println!("Parity plan for {} ({}, {}, {} reads per strand)", args.file, args.codec, args.profile, args.coverage);
    println!("  data strands         {}", plan.data_strands);
    println!("  no usable read       {:.1}%", o.unread * 100.0);
    println!("  read, not decoded    {:.1}% (upper 95% bound)", o.undecoded * 100.0);
    println!("  read, decoded wrong  {:.1}% (upper 95% bound, costs two parity each)", o.wrong * 100.0);
    println!(
        "  parity per stripe    {:<4} chance of losing the file {}",
        plan.parity,
        percent(plan.failure_probability)
    );
    println!(
        "  store default        {:<4} chance of losing the file {}",
        STORE_DEFAULT_PARITY,
        percent(default_failure)
    );
    println!("\nStore it with: nucle store {} --codec {} --redundancy {}", args.file, args.codec, plan.parity);
}

/// The read channel `nucle store` and `nucle migrate` plan parity for,
/// from `--channel`, `--coverage` and `--max-failure`.
pub struct Channel {
    pub profile: String,
    pub coverage: u32,
    pub max_failure: f64,
}

impl Channel {
    fn planner(&self) -> NucleOS {
        if self.coverage == 0 {
            eprintln!("--coverage must be at least 1");
            std::process::exit(1);
        }
        let profile = crate::parse_hw_profile(&self.profile);
        NucleOS::new(10).with_noise(SimulationConfig::realistic(profile, self.coverage, 42))
    }

    /// The parity `data` written with `codec` needs through this channel.
    /// Exits when no parity meets the target. Warns on stderr when `codec`
    /// needs more parity than data and HEDGES would write fewer strands,
    /// and when an explicit `-r` (`given`) falls short of the plan.
    pub fn plan(&self, data: &[u8], codec: CodecKind, codec_name: &str, given: Option<usize>) -> ParityPlan {
        let planner = self.planner();
        let hedges = || {
            (codec != CodecKind::Hedges)
                .then(|| planner.plan_parity(data, CodecKind::Hedges, self.max_failure).ok())
                .flatten()
        };
        let plan = match planner.plan_parity(data, codec, self.max_failure) {
            Ok(plan) => plan,
            Err(e) => {
                eprintln!("Parity plan failed: {} ({} through {} at {} reads per strand)", e, codec_name, self.profile, self.coverage);
                if let Some(h) = hedges() {
                    eprintln!("hedges would need {} parity strands per stripe for {} data strands: try --codec hedges", h.parity, h.data_strands);
                }
                std::process::exit(1);
            }
        };
        if plan.parity > plan.data_strands {
            if let Some(h) = hedges().filter(|h| total_strands(h) < total_strands(&plan)) {
                eprintln!(
                    "warning: {} needs {} parity strands per stripe for its {} data strands through {} at {} reads per strand ({} strands in all); hedges needs {} for {} ({} in all). Consider --codec hedges.",
                    codec_name, plan.parity, plan.data_strands, self.profile, self.coverage, total_strands(&plan), h.parity, h.data_strands, total_strands(&h)
                );
            }
        }
        if let Some(given) = given.filter(|&given| given < plan.parity) {
            eprintln!(
                "warning: {} parity strands per stripe lose this file {} of the time through {} at {} reads per strand; the plan is {}.",
                given,
                percent(failure_probability(plan.data_strands, given, &plan.outcomes)),
                self.profile,
                self.coverage,
                plan.parity
            );
        }
        plan
    }

    /// One line on what `plan` chose, for text output.
    pub fn describe(&self, plan: &ParityPlan) -> String {
        format!(
            "Parity planned for {} at {} reads per strand: {} per stripe ({:.1}% of strands unread; chance of losing the file {})",
            self.profile,
            self.coverage,
            plan.parity,
            plan.outcomes.unread * 100.0,
            percent(plan.failure_probability)
        )
    }
}

/// Data and parity strands a plan writes, stripes included.
fn total_strands(plan: &ParityPlan) -> usize {
    let per_stripe = RS_MAX_SYMBOLS.saturating_sub(plan.parity).max(1);
    plan.data_strands + plan.parity * plan.data_strands.div_ceil(per_stripe)
}

/// A probability as a percentage with enough digits to tell small ones
/// apart.
fn percent(p: f64) -> String {
    if p >= 0.01 || p == 0.0 {
        format!("{:.1}%", p * 100.0)
    } else {
        format!("{:.3}%", p * 100.0)
    }
}
