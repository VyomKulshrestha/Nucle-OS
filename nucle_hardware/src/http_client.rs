//! A `ureq::Agent` constructor shared by the REST-based vendor adapters
//! (`twist`, `idt`, `illumina`). Bounds every request with a connect/total
//! timeout so an unreachable or hung vendor endpoint fails a `submit`/
//! `status` call rather than blocking the caller forever -- `Provider`'s
//! interface is synchronous, so there is no outer async timeout layer to
//! rely on instead.
use std::time::Duration;

pub fn new_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).build();
    config.into()
}
