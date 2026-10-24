//! # Parity Planning
//!
//! Reed-Solomon parity can replace any strand that is missing, but only up
//! to its parity count per stripe, and no decoder can rebuild a strand that
//! no read covers. How many strands go unread is a property of the channel:
//! dropout, the spread of reads across strands, and how many reads each
//! strand gets. Through the measured low-fidelity array channel at 3 reads
//! per strand about a fifth of strands get no read at all, so 16 parity
//! strands for 64 data strands lose the file more often than not.
//!
//! A strand can be lost three ways, counted in [`StrandOutcomes`]: no read
//! survives, reads survive but none decodes, or it decodes to the wrong
//! strand. The first two leave a gap Reed-Solomon fills with one parity
//! strand; the third is an error it has to find as well as fix, which costs
//! two. [`unread_share`] measures the first by sending sample strands
//! through the same [`NoiseEngine`] the archive would see; the caller
//! measures the other two with its own decoder. Losses are independent in
//! the simulator, so a stripe of `n` strands with `p` parity survives when
//! twice its wrong strands plus its missing ones come to at most `p`, a
//! trinomial tail, and [`parity_for`] picks the smallest parity per stripe
//! that keeps the chance of losing any stripe under a target.

use crate::decay::binomial_cdf;
use crate::noise::{NoiseEngine, SimulationConfig};
use nucle_codec::base::{DnaStrand, Nucleotide, StrandCollection};
use nucle_codec::stable_rng::StableRng;

/// Symbols per Reed-Solomon codeword over GF(256), data and parity together;
/// mirrors the striping of `nucle_ecc::reed_solomon::ReedSolomon`.
pub const RS_MAX_SYMBOLS: usize = 256;

/// Sample strands a planner should simulate: enough to put a loss rate near
/// a fifth within about a percentage point.
pub const DEFAULT_LOSS_SAMPLES: usize = 4000;

/// The share of `strands` that get no read `usable` accepts after passing
/// through `config`, its coverage included. Seeded independently of
/// `config`, so planning never shifts the noise a later write or read sees.
pub fn unread_share(config: &SimulationConfig, strands: &[DnaStrand], usable: impl Fn(&DnaStrand) -> bool) -> f64 {
    if strands.is_empty() {
        return 0.0;
    }
    let mut probe = config.clone();
    probe.seed = config.seed ^ 0x9E37_79B9_7F4A_7C15;
    let result = NoiseEngine::new(probe).simulate(&StrandCollection::from_strands(strands.to_vec(), 0));
    let coverage = u64::from(config.coverage_depth.max(1));
    let mut read = vec![false; strands.len()];
    for s in result.pool.strands.iter().filter(|s| s.is_intact && !s.is_truncated) {
        let slot = (s.strand_id / coverage) as usize;
        if slot < read.len() && !read[slot] && usable(&s.sequence) {
            read[slot] = true;
        }
    }
    read.iter().filter(|&&r| !r).count() as f64 / strands.len() as f64
}

/// The chance that a random strand of `strand_len` nucleotides gets no
/// intact read through `config`, from `samples` strands.
pub fn strand_loss_probability(config: &SimulationConfig, strand_len: usize, samples: usize) -> f64 {
    let mut rng = StableRng::seed_from_u64(0x0D15_EA5E ^ strand_len as u64);
    let strands: Vec<DnaStrand> = (0..samples)
        .map(|_| DnaStrand::new((0..strand_len).map(|_| Nucleotide::ALL[rng.gen_range(0..4)]).collect()))
        .collect();
    unread_share(config, &strands, |_| true)
}

/// How each strand of a file fares through a channel, as shares of all
/// strands.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StrandOutcomes {
    /// No usable read.
    pub unread: f64,
    /// Read, but no read or consensus decodes.
    pub undecoded: f64,
    /// Decoded to something other than the strand written.
    pub wrong: f64,
}

impl StrandOutcomes {
    /// A channel whose only loss is strands nobody reads.
    pub fn unread_only(unread: f64) -> Self {
        Self { unread, ..Self::default() }
    }

    /// From the unread share and a decode trial: of `attempts` strands that
    /// had reads, `undecoded` came back with nothing and `wrong` with the
    /// wrong strand. The trial is far smaller than the unread sample, so
    /// each of its rates is taken at the upper end of its 95% Wilson
    /// interval: none failing in 200 tries still allows about 2%.
    pub fn measured(unread: f64, attempts: usize, undecoded: usize, wrong: usize) -> Self {
        let read = 1.0 - unread;
        Self {
            unread,
            undecoded: read * wilson_upper(undecoded, attempts),
            wrong: read * wilson_upper(wrong, attempts),
        }
    }

    /// Strands that leave a gap: unread or undecoded.
    pub fn missing(&self) -> f64 {
        (self.unread + self.undecoded).min(1.0)
    }
}

/// The upper end of the 95% Wilson score interval for `hits` in `trials`;
/// zero with no trials, since there is then nothing to bound.
fn wilson_upper(hits: usize, trials: usize) -> f64 {
    if trials == 0 {
        return 0.0;
    }
    let (n, p, z2) = (trials as f64, hits as f64 / trials as f64, 1.96f64 * 1.96);
    let centre = p + z2 / (2.0 * n);
    let spread = (z2 * (p * (1.0 - p) / n + z2 / (4.0 * n * n))).sqrt();
    ((centre + spread) / (1.0 + z2 / n)).min(1.0)
}

/// P(X = k) for X ~ Binomial(n, p), in log space.
fn binomial_pmf(n: usize, k: usize, p: f64) -> f64 {
    if k > n {
        return 0.0;
    }
    if p <= 0.0 || p >= 1.0 {
        let certain = if p <= 0.0 { 0 } else { n };
        return if k == certain { 1.0 } else { 0.0 };
    }
    let ln_choose: f64 = (0..k).map(|i| ((n - i) as f64).ln() - ((i + 1) as f64).ln()).sum();
    (ln_choose + k as f64 * p.ln() + (n - k) as f64 * (1.0 - p).ln()).exp()
}

/// The chance a stripe of `n` strands with `parity` of them parity can be
/// rebuilt: twice the wrong strands plus the missing ones at most `parity`.
fn stripe_survival(n: usize, parity: usize, outcomes: &StrandOutcomes) -> f64 {
    let (missing, wrong) = (outcomes.missing(), outcomes.wrong.min(1.0));
    if wrong <= 0.0 {
        return binomial_cdf(n, parity, missing);
    }
    // Given e wrong strands, each of the rest is missing with the
    // conditional probability missing / (1 - wrong).
    let missing_given_right = if wrong < 1.0 { (missing / (1.0 - wrong)).min(1.0) } else { 0.0 };
    (0..=(parity / 2).min(n))
        .map(|e| binomial_pmf(n, e, wrong) * binomial_cdf(n - e, parity - 2 * e, missing_given_right))
        .sum::<f64>()
        .min(1.0)
}

/// The stripe sizes (data strands per stripe) Reed-Solomon uses for
/// `data_strands` at `parity` parity strands per stripe.
fn stripes(data_strands: usize, parity: usize) -> impl Iterator<Item = usize> {
    let per_stripe = RS_MAX_SYMBOLS.saturating_sub(parity).max(1);
    let full = data_strands / per_stripe;
    let rest = data_strands % per_stripe;
    std::iter::repeat_n(per_stripe, full).chain((rest > 0).then_some(rest))
}

/// The chance a file of `data_strands` strands with `parity` parity strands
/// per stripe cannot be rebuilt when its strands fare as `outcomes` says,
/// independently.
pub fn failure_probability(data_strands: usize, parity: usize, outcomes: &StrandOutcomes) -> f64 {
    let survive: f64 = stripes(data_strands, parity).map(|k| stripe_survival(k + parity, parity, outcomes)).product();
    (1.0 - survive).max(0.0)
}

/// The smallest parity per stripe that keeps [`failure_probability`] at or
/// below `max_failure`, or `None` if no parity a stripe can hold does.
pub fn parity_for(data_strands: usize, outcomes: &StrandOutcomes, max_failure: f64) -> Option<usize> {
    if data_strands == 0 || (outcomes.missing() <= 0.0 && outcomes.wrong <= 0.0) {
        return Some(0);
    }
    (0..RS_MAX_SYMBOLS).find(|&p| failure_probability(data_strands, p, outcomes) <= max_failure)
}

/// A parity plan for one file through one channel.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParityPlan {
    /// Data strands the file encodes to.
    pub data_strands: usize,
    /// How its strands fare through the channel.
    pub outcomes: StrandOutcomes,
    /// Parity strands per Reed-Solomon stripe.
    pub parity: usize,
    /// Chance the file still cannot be rebuilt at that parity.
    pub failure_probability: f64,
}

impl ParityPlan {
    /// [`parity_for`] as a plan; `None` when even a stripe that is nearly
    /// all parity cannot meet `max_failure`.
    pub fn new(data_strands: usize, outcomes: StrandOutcomes, max_failure: f64) -> Option<Self> {
        let parity = parity_for(data_strands, &outcomes, max_failure)?;
        Some(Self {
            data_strands,
            outcomes,
            parity,
            failure_probability: failure_probability(data_strands, parity, &outcomes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::HardwareProfile;

    #[test]
    fn measured_low_fidelity_reads_leave_a_fifth_of_strands_unread_at_three_reads() {
        let at = |coverage| {
            strand_loss_probability(&SimulationConfig::realistic(HardwareProfile::ArrayLowFidelity, coverage, 1), 192, 4000)
        };
        let (three, six) = (at(3), at(6));
        // The diagnosis measured 21.7% and 11.7% on 640 HEDGES strands.
        assert!((0.17..0.27).contains(&three), "{three}");
        assert!((0.08..0.16).contains(&six), "{six}");
        let pristine = SimulationConfig::realistic(HardwareProfile::Pristine, 3, 1);
        assert_eq!(strand_loss_probability(&pristine, 192, 1000), 0.0);
        // Reads the caller cannot use count as no read.
        let strands = vec![DnaStrand::new(vec![Nucleotide::A; 40]); 10];
        assert_eq!(unread_share(&pristine, &strands, |_| false), 1.0);
    }

    #[test]
    fn parity_grows_with_loss_and_meets_its_target() {
        let fifth = StrandOutcomes::unread_only(0.217);
        // 64 data strands at a fifth lost: 16 parity fails more often than not.
        assert!(failure_probability(64, 16, &fifth) > 0.5);
        let p = parity_for(64, &fifth, 1e-3).unwrap();
        assert!(failure_probability(64, p, &fifth) <= 1e-3);
        assert!(failure_probability(64, p - 1, &fifth) > 1e-3, "{p} is not the smallest");
        let plan = ParityPlan::new(64, fifth, 1e-3).unwrap();
        assert_eq!((plan.data_strands, plan.parity), (64, p));
        assert!(parity_for(64, &StrandOutcomes::unread_only(0.05), 1e-3).unwrap() < p);
        assert_eq!(parity_for(64, &StrandOutcomes::default(), 1e-3), Some(0));
        // Nine strands in ten lost still fits, with ten data strands to a
        // stripe; a channel that reads nothing does not.
        assert_eq!(parity_for(64, &StrandOutcomes::unread_only(0.9), 1e-3), Some(246));
        assert_eq!(parity_for(64, &StrandOutcomes::unread_only(1.0), 1e-3), None);
    }

    #[test]
    fn a_wrong_strand_costs_two_parity_and_a_missing_one_costs_one() {
        let missing = StrandOutcomes { undecoded: 0.05, ..StrandOutcomes::default() };
        let wrong = StrandOutcomes { wrong: 0.05, ..StrandOutcomes::default() };
        // With wrong strands only, odd parity buys nothing over the even
        // parity below it.
        assert!((stripe_survival(70, 7, &wrong) - stripe_survival(70, 6, &wrong)).abs() < 1e-12);
        assert!(stripe_survival(70, 6, &wrong) < stripe_survival(70, 6, &missing));
        let (p_missing, p_wrong) = (parity_for(64, &missing, 1e-3).unwrap(), parity_for(64, &wrong, 1e-3).unwrap());
        assert!(p_wrong > p_missing + p_missing / 2, "{p_wrong} vs {p_missing}");
        // The trinomial tail agrees with the binomial one when nothing is wrong.
        assert!((stripe_survival(80, 16, &missing) - binomial_cdf(80, 16, 0.05)).abs() < 1e-12);
    }

    #[test]
    fn a_clean_decode_trial_still_allows_a_little_failure() {
        let o = StrandOutcomes::measured(0.2, 200, 0, 0);
        assert!((0.014..0.016).contains(&o.undecoded), "{o:?}");
        assert_eq!(o.undecoded, o.wrong);
        assert!(StrandOutcomes::measured(0.2, 200, 10, 0).undecoded > 0.8 * 10.0 / 200.0);
        assert_eq!(StrandOutcomes::measured(1.0, 0, 0, 0).missing(), 1.0);
    }

    #[test]
    fn large_files_split_into_stripes_like_reed_solomon() {
        assert_eq!(stripes(600, 56).collect::<Vec<_>>(), vec![200, 200, 200]);
        assert_eq!(stripes(64, 16).collect::<Vec<_>>(), vec![64]);
        // More stripes, each a chance to fail.
        let tenth = StrandOutcomes::unread_only(0.1);
        assert!(failure_probability(600, 40, &tenth) > failure_probability(150, 40, &tenth));
    }
}
