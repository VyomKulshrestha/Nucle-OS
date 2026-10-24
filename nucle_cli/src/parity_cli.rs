//! `nucle parity`: how much Reed-Solomon parity a file needs to come back
//! through a given synthesis-and-sequencing channel.
//!
//! Wraps `NucleOS::plan_parity`: the file's own strands go through the
//! simulated channel and the per-strand decoder `nucle retrieve` uses, and
//! the answer is the smallest parity per stripe that keeps the chance of
//! losing the file under `--max-failure`. The report sets that beside the
//! parity `nucle store` uses by default.

use nucle_synth::noise::SimulationConfig;
use nucle_synth::planning::failure_probability;
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

/// A probability as a percentage with enough digits to tell small ones
/// apart.
fn percent(p: f64) -> String {
    if p >= 0.01 || p == 0.0 {
        format!("{:.1}%", p * 100.0)
    } else {
        format!("{:.3}%", p * 100.0)
    }
}
