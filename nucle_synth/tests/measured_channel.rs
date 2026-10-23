//! The array-pool profiles reproduce what was measured on real reads
//! (ERP173549: ERR15127739 high fidelity, ERR15127744 low fidelity).

use nucle_codec::base::{DnaStrand, Nucleotide, StrandCollection};
use nucle_synth::errors::ErrorModel;
use nucle_synth::noise::{realistic_stage_pair, NoiseEngine, SimulationConfig};
use nucle_synth::profiles::HardwareProfile;
use nucle_synth::strand::SynthStrand;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn random_strand(rng: &mut StdRng, len: usize) -> DnaStrand {
    DnaStrand::new((0..len).map(|_| Nucleotide::ALL[rng.gen_range(0..4)]).collect())
}

/// Per-base (substitution, insertion, deletion) rates over many strands.
fn measured_rates(profile: HardwareProfile, strands: usize) -> (f64, f64, f64) {
    let model = profile.to_error_model();
    let mut rng = StdRng::seed_from_u64(7);
    let (mut s, mut i, mut d, mut bases) = (0usize, 0usize, 0usize, 0usize);
    for id in 0..strands {
        let mut strand = SynthStrand::pristine(random_strand(&mut rng, 129), id as u64);
        model.apply(&mut strand, &mut rng);
        if !strand.is_intact {
            continue;
        }
        s += strand.error_count.substitutions;
        i += strand.error_count.insertions;
        d += strand.error_count.deletions;
        bases += 129;
    }
    (s as f64 / bases as f64, i as f64 / bases as f64, d as f64 / bases as f64)
}

fn close(got: f64, want: f64, rel: f64) -> bool {
    (got - want).abs() <= rel * want
}

#[test]
fn high_fidelity_rates_match_the_measured_run() {
    // Measured R1: sub 7.47e-4, ins 5.61e-5, del 2.78e-4.
    let (s, i, d) = measured_rates(HardwareProfile::ArrayHighFidelity, 60_000);
    assert!(close(s, 7.47e-4, 0.15), "sub {s}");
    assert!(close(i, 5.61e-5, 0.30), "ins {i}");
    assert!(close(d, 2.78e-4, 0.15), "del {d}");
}

#[test]
fn low_fidelity_rates_match_the_measured_run() {
    // Measured R1: sub 9.82e-3, ins 9.97e-4, del 4.32e-3.
    let (s, i, d) = measured_rates(HardwareProfile::ArrayLowFidelity, 20_000);
    assert!(close(s, 9.82e-3, 0.10), "sub {s}");
    assert!(close(i, 9.97e-4, 0.20), "ins {i}");
    assert!(close(d, 4.32e-3, 0.10), "del {d}");
}

/// (transition share, G>T share) of substitutions, measured position by
/// position on a model with indels switched off so bases stay aligned.
fn spectrum(profile: HardwareProfile) -> (f64, f64) {
    let mut model = profile.to_error_model();
    let mut features = model.measured.clone().expect("measured profile");
    features.homopolymer_del = [0.0; 6];
    features.homopolymer_ins = [0.0; 6];
    model.measured = Some(features);
    let mut rng = StdRng::seed_from_u64(11);
    let (mut total, mut transitions, mut g_to_t) = (0usize, 0usize, 0usize);
    for id in 0..40_000 {
        let original = random_strand(&mut rng, 129);
        let mut strand = SynthStrand::pristine(original.clone(), id);
        model.apply(&mut strand, &mut rng);
        if !strand.is_intact || strand.is_truncated {
            continue;
        }
        for (&a, &b) in original.bases().iter().zip(strand.sequence.bases()) {
            if a == b {
                continue;
            }
            total += 1;
            use Nucleotide::*;
            if matches!((a, b), (A, G) | (G, A) | (C, T) | (T, C)) {
                transitions += 1;
            }
            if (a, b) == (G, T) {
                g_to_t += 1;
            }
        }
    }
    (transitions as f64 / total as f64, g_to_t as f64 / total as f64)
}

#[test]
fn substitution_spectra_match_the_measured_runs() {
    // Low fidelity: transitions are about 70% of substitutions.
    let (ts, _) = spectrum(HardwareProfile::ArrayLowFidelity);
    assert!(ts > 0.62 && ts < 0.78, "low-fidelity transition share {ts}");
    // High fidelity: G>T (oxidative damage) is the single largest class,
    // about 23% of substitutions.
    let (_, gt) = spectrum(HardwareProfile::ArrayHighFidelity);
    assert!(gt > 0.18 && gt < 0.29, "high-fidelity G>T share {gt}");
}

#[test]
fn long_homopolymers_carry_more_deletions() {
    let model = HardwareProfile::ArrayHighFidelity.to_error_model();
    let mut rng = StdRng::seed_from_u64(3);
    let deletion_rate = |template: &str, rng: &mut StdRng| {
        let strand = DnaStrand::from_str(template).unwrap();
        let mut dels = 0;
        for id in 0..40_000 {
            let mut s = SynthStrand::pristine(strand.clone(), id);
            model.apply(&mut s, rng);
            dels += s.error_count.deletions;
        }
        dels as f64 / (40_000.0 * strand.len() as f64)
    };
    let runs = deletion_rate("AAAAAAAACCCCCCCCGGGGGGGGTTTTTTTT", &mut rng);
    let mixed = deletion_rate("ACGTTGCAACGTTGCAACGTTGCAACGTTGCA", &mut rng);
    let ratio = runs / mixed;
    assert!(ratio > 1.8 && ratio < 3.0, "runs {runs} mixed {mixed} ratio {ratio}");
}

#[test]
fn coverage_spread_thins_reads_per_strand_like_a_lognormal() {
    let strands: Vec<DnaStrand> = (0..4000).map(|i| DnaStrand::from_str(if i % 2 == 0 { "ACGTACGTAC" } else { "TTGCAATGCA" }).unwrap()).collect();
    let collection = StrandCollection::from_strands(strands, 0);
    let config = SimulationConfig { coverage_log_sigma: 1.14, ..SimulationConfig::pristine().with_coverage(30) };
    let result = NoiseEngine::new(config).simulate(&collection);
    let intact_per_strand: Vec<usize> = result
        .pool
        .strands
        .chunks(30)
        .map(|copies| copies.iter().filter(|c| c.is_intact).count())
        .collect();
    let mean = intact_per_strand.iter().sum::<usize>() as f64 / intact_per_strand.len() as f64;
    // E[min(1, w)] for w lognormal with mean 1 and sigma 1.14 is about 0.57.
    assert!(mean > 30.0 * 0.50 && mean < 30.0 * 0.64, "mean intact copies {mean}");
    // Strands with no surviving copy: P(w*30 ~ 0) is small but real.
    let empty = intact_per_strand.iter().filter(|&&n| n == 0).count() as f64 / 4000.0;
    assert!(empty > 0.0002 && empty < 0.03, "empty fraction {empty}");
    // Without spread every copy survives.
    let flat = NoiseEngine::new(SimulationConfig::pristine().with_coverage(30)).simulate(&collection);
    assert_eq!(flat.pool.intact_count(), 4000 * 30);
}

#[test]
fn measured_profiles_already_include_sequencing() {
    for p in [HardwareProfile::ArrayHighFidelity, HardwareProfile::ArrayLowFidelity] {
        assert_eq!(realistic_stage_pair(p), (p, HardwareProfile::Pristine));
        assert!(SimulationConfig::realistic(p, 10, 1).coverage_log_sigma > 0.0);
    }
    assert_eq!(SimulationConfig::realistic(HardwareProfile::Illumina, 10, 1).coverage_log_sigma, 0.0);
}

#[test]
fn illumina_indels_follow_the_measured_sequencing_rate() {
    let rates = HardwareProfile::Illumina.error_rates();
    assert!(rates.insertion <= 2e-5 && rates.deletion <= 2e-5);
}
