//! Probabilistic type support for NucleScript.

use crate::ast::{PoolState, Profile};

#[derive(Debug, Clone, PartialEq)]
pub struct ProbPoolType {
    pub state: PoolState,
    pub error_rate_percent: f64,
}

impl ProbPoolType {
    pub fn new(state: PoolState, error_rate_percent: f64) -> Self {
        Self { state, error_rate_percent }
    }
}

pub fn profile_error_rate_percent(profile: Profile) -> f64 {
    match profile {
        Profile::Illumina => 0.35,
        Profile::Nanopore => 5.00,
        Profile::Twist => 0.03,
        Profile::Idt => 0.05,
    }
}

/// How much per-base error one real read set had left after the published
/// consensus decoder saw only the first `c` reads of each strand, as a
/// fraction of the reads' own error rate. Measured with
/// `cargo run --release -p nucle_ecc --example coverage_curve`; see
/// docs/architecture.md, "Measured consensus curve".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredConsensus {
    pub reads: &'static str,
    /// Mean per-read edit rate of the whole set, in percent.
    pub raw_error_percent: f64,
    /// `(coverage, residual / raw)`, coverages increasing. A point measured
    /// on fewer than 30 strands is left out.
    pub points: &'static [(usize, f64)],
}

impl MeasuredConsensus {
    /// This set's ratio at `coverage`: the coverage falls back to the
    /// largest one measured at or below it (the last one past the end), and
    /// the ratio is the largest measured from there up, so the curve never
    /// rises with coverage and never improves between two measurements.
    pub fn ratio(&self, coverage: usize) -> f64 {
        let Some(start) = self.points.iter().rposition(|(c, _)| *c <= coverage) else {
            return 1.0;
        };
        self.points[start..].iter().map(|(_, r)| *r).fold(0.0, f64::max)
    }
}

/// The three read sets the consensus curve is taken from.
pub const MEASURED_CONSENSUS: [MeasuredConsensus; 3] = [
    MeasuredConsensus {
        reads: "Oxford Nanopore MinION, first 50 strands of the Clustered Nanopore Reads set (Srinivasavaradhan et al. 2021)",
        raw_error_percent: 5.7610,
        points: &[
            (1, 1.16141),
            (2, 0.92787),
            (3, 0.25248),
            (4, 0.14833),
            (5, 0.07890),
            (6, 0.05365),
            (8, 0.03288),
            (10, 0.03670),
            (12, 0.02254),
            (15, 0.01618),
            (20, 0.02104),
        ],
    },
    MeasuredConsensus {
        reads: "Illumina, high-fidelity array pool (Gimpel et al. 2026, ERP173549)",
        raw_error_percent: 0.2238,
        points: &[
            (1, 1.13792),
            (2, 0.62823),
            (3, 0.18273),
            (4, 0.07918),
            (5, 0.03574),
            (6, 0.03407),
            (8, 0.02515),
            (10, 0.01829),
            (12, 0.01554),
            (15, 0.01584),
            (20, 0.01671),
            (25, 0.00906),
            (30, 0.00038),
        ],
    },
    MeasuredConsensus {
        reads: "Illumina, low-fidelity array pool (Gimpel et al. 2026, ERP173549)",
        raw_error_percent: 1.8504,
        points: &[
            (1, 1.09639),
            (2, 0.77845),
            (3, 0.19811),
            (4, 0.13998),
            (5, 0.05543),
            (6, 0.04013),
            (8, 0.01636),
            (10, 0.00846),
            (12, 0.00529),
            (15, 0.00332),
            (20, 0.00287),
            (25, 0.00098),
            (30, 0.00034),
        ],
    },
];

/// The share of a pool's per-base error a consensus of `coverage` reads
/// leaves: the worst of the three measured read sets at that coverage
/// (see [`MeasuredConsensus::ratio`]). One read leaves all of it.
pub fn consensus_residual_ratio(coverage: usize) -> f64 {
    if coverage <= 1 {
        return 1.0;
    }
    MEASURED_CONSENSUS.iter().map(|set| set.ratio(coverage)).fold(0.0, f64::max)
}

/// The error rate `consensus_vote(source, coverage)` gives its result:
/// the source's rate times [`consensus_residual_ratio`]. It is a bound
/// from measurement, not a prediction for any one pool.
pub fn consensus_error_rate_percent(input_error_percent: f64, coverage: usize) -> f64 {
    input_error_percent * consensus_residual_ratio(coverage)
}
