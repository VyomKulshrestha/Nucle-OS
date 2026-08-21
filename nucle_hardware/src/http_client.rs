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

pub(crate) fn validate_job_id(id: &str, provider: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
        return Err(format!("{provider} returned an invalid job identifier"));
    }
    Ok(())
}

pub(crate) fn encode_path_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
