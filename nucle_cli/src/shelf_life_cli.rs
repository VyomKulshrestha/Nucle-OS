//! `nucle shelf-life`: how long a DNA archive stays readable in storage.
//!
//! Wraps `nucle_synth::decay`. The answer for one storage temperature is the
//! number of years until the probability of recovering the whole file
//! drops below the target; a comparison table repeats it across common
//! storage temperatures so the effect of a freezer is visible at a glance.

use nucle_synth::decay::{strand_half_life_years, ShelfLifeQuery, StorageMedium};

/// Temperatures every report compares, °C: freezer, fridge, cellar, room.
const COMPARE_CELSIUS: [f64; 4] = [-20.0, 4.0, 10.0, 25.0];

pub struct ShelfLifeArgs {
    pub medium: String,
    pub celsius: f64,
    pub length: usize,
    pub data: usize,
    pub parity: usize,
    pub copies: u32,
    pub target_percent: f64,
    pub stored: Option<String>,
}

pub fn cmd_shelf_life(args: ShelfLifeArgs, pool_dir: &std::path::Path, passphrase: Option<&str>, json: bool) {
    let Some(medium) = StorageMedium::parse(&args.medium) else {
        eprintln!("Unknown storage medium '{}'. Use: encapsulated, bone, silica", args.medium);
        std::process::exit(1);
    };
    if !(0.0..100.0).contains(&args.target_percent) || args.target_percent <= 0.0 {
        eprintln!("--target must be a percentage between 0 and 100 (exclusive), got {}", args.target_percent);
        std::process::exit(1);
    }
    if args.copies == 0 || args.length == 0 || args.data == 0 {
        eprintln!("--copies, --length and --data must all be at least 1");
        std::process::exit(1);
    }

    let (data, parity, length, source) = match &args.stored {
        Some(name) => {
            let os = crate::open_pool(pool_dir, passphrase);
            let Some(file) = os.catalog.get_by_name(name) else {
                eprintln!("No stored file named '{}' in {}", name, pool_dir.display());
                std::process::exit(1);
            };
            let avg_len = os.dna_stat().avg_strand_length.round() as usize;
            // A multi-stripe file is treated as one stripe with its
            // per-stripe parity budget: conservative, never optimistic.
            let budget = if file.rs_parity_per_stripe > 0 { file.rs_parity_per_stripe } else { file.parity_strand_count };
            (file.data_strand_count, budget, avg_len.max(1), format!("stored file '{}'", name))
        }
        None => (args.data, args.parity, args.length, "parameters".to_string()),
    };

    let query = |celsius: f64| ShelfLifeQuery {
        medium,
        celsius,
        strand_len: length,
        data_strands: data,
        parity_strands: parity,
        copies_per_strand: args.copies,
        target_recovery: args.target_percent / 100.0,
    };
    let primary = query(args.celsius);
    let rate = medium.break_rate_per_nt_year(args.celsius);
    let half_life = strand_half_life_years(rate, length);
    let life = primary.shelf_life_years();
    let table: Vec<(f64, f64, f64)> = COMPARE_CELSIUS
        .iter()
        .map(|&c| {
            let r = medium.break_rate_per_nt_year(c);
            (c, strand_half_life_years(r, length), query(c).shelf_life_years())
        })
        .collect();

    if json {
        let rows: Vec<serde_json::Value> = table
            .iter()
            .map(|&(c, h, l)| serde_json::json!({ "celsius": c, "strand_half_life_years": finite(h), "shelf_life_years": finite(l) }))
            .collect();
        let out = serde_json::json!({
            "medium": medium.name(),
            "kinetics_source": medium.citation(),
            "activation_energy_kj_mol": medium.kinetics().activation_energy_kj_mol,
            "celsius": args.celsius,
            "source": source,
            "strand_length_nt": length,
            "data_strands": data,
            "parity_strands": parity,
            "copies_per_strand": args.copies,
            "target_recovery_percent": args.target_percent,
            "break_rate_per_nt_year": rate,
            "strand_half_life_years": finite(half_life),
            "shelf_life_years": finite(life),
            "by_temperature": rows,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("DNA archive shelf life");
    println!("  medium:            {} ({})", medium.name(), medium.citation());
    println!("  archive:           {} data + {} parity strands of {} nt, {} copies each ({})", data, parity, length, args.copies, source);
    println!("  storage:           {:.1} °C", args.celsius);
    println!("  break rate:        {:.3e} per nucleotide per year", rate);
    println!("  strand half-life:  {}", years(half_life));
    println!("  readable until:    {} (≥{}% chance the whole file decodes)", years(life), args.target_percent);
    println!();
    println!("  {:>7}  {:>18}  {:>18}", "°C", "strand half-life", "readable until");
    for (c, h, l) in table {
        println!("  {:>7.1}  {:>18}  {:>18}", c, years(h), years(l));
    }
    println!();
    println!("Arrhenius extrapolation from published kinetics; real archives also face humidity,");
    println!("oxidation and handling losses this model does not include. Figures in the millions");
    println!("of years or more are what the fitted rate implies, not predictions anyone has tested.");
}

fn finite(x: f64) -> serde_json::Value {
    if x.is_finite() { serde_json::json!(x) } else { serde_json::Value::Null }
}

/// Human-scale year count: "1,234 years", "3.4 thousand years", "2.1 million years".
fn years(y: f64) -> String {
    if !y.is_finite() {
        return "over 10^15 years".to_string();
    }
    if y < 1.0 {
        return format!("{:.0} days", y * 365.25);
    }
    if y < 10_000.0 {
        return format!("{:.0} years", y);
    }
    if y < 1e6 {
        return format!("{:.1} thousand years", y / 1e3);
    }
    if y < 1e9 {
        return format!("{:.1} million years", y / 1e6);
    }
    format!("{:.1} billion years", y / 1e9)
}

#[cfg(test)]
mod tests {
    use super::years;

    #[test]
    fn years_are_rendered_at_a_readable_scale() {
        assert_eq!(years(0.5), "183 days");
        assert_eq!(years(521.0), "521 years");
        assert_eq!(years(38_300.0), "38.3 thousand years");
        assert_eq!(years(6.8e6), "6.8 million years");
        assert_eq!(years(f64::INFINITY), "over 10^15 years");
    }
}
