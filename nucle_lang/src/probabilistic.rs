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
    }
}

pub fn consensus_error_rate_percent(input_error_percent: f64, coverage: usize) -> f64 {
    if coverage <= 1 {
        return input_error_percent;
    }
    input_error_percent / (coverage * coverage) as f64
}
