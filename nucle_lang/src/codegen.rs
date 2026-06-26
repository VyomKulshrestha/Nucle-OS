//! VFS backend for NucleScript.

use crate::ast::*;
use crate::middle::{lower_program, MirOp};
use crate::typeck::TypeReport;
use nucle_synth::noise::SimulationConfig;
use nucle_synth::profiles::HardwareProfile;
use nucle_vfs::syscall::{NucleOS, PoolStatus};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CompiledPlan {
    pub program: Program,
    pub calls: Vec<VfsCall>,
    pub type_report: TypeReport,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VfsCall {
    Store {
        file: String,
        pool: String,
        redundancy: usize,
        simulate: bool,
        coverage: usize,
        profile: Profile,
        verify_roundtrip: bool,
    },
    Retrieve {
        pool: String,
        query: String,
    },
    Delete {
        file: String,
        pool: String,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub type_report: TypeReport,
    pub steps: Vec<String>,
    pub pool_status: PoolStatus,
}

impl std::fmt::Display for ExecutionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.type_report.has_warnings() {
            writeln!(f, "NucleScript diagnostics:")?;
            writeln!(f, "{}", self.type_report)?;
        }
        for step in &self.steps {
            writeln!(f, "{}", step)?;
        }
        writeln!(f, "\n{}", self.pool_status)
    }
}

pub fn compile_program(program: Program, type_report: TypeReport) -> CompiledPlan {
    let mir = lower_program(&program);
    let mut calls = Vec::new();
    for op in mir.ops {
        match op {
            MirOp::Store {
                file,
                pool,
                redundancy,
                simulate,
                coverage,
                profile,
                verify_roundtrip,
                ..
            } => calls.push(VfsCall::Store {
                file,
                pool,
                redundancy,
                simulate,
                coverage,
                profile,
                verify_roundtrip,
            }),
            MirOp::Retrieve { pool, query, .. } => calls.push(VfsCall::Retrieve { pool, query }),
            MirOp::Delete { file, pool, .. } => calls.push(VfsCall::Delete { file, pool }),
            MirOp::PoolSchema { .. } | MirOp::ProbabilisticBind { .. } => {}
        }
    }

    CompiledPlan { program, calls, type_report }
}

pub fn execute_program(
    os: &mut NucleOS,
    plan: &mut CompiledPlan,
    base_dir: &Path,
) -> Result<ExecutionReport, String> {
    let mut steps = Vec::new();

    for call in &plan.calls {
        match call {
            VfsCall::Store { file, pool, redundancy, simulate, coverage, profile, verify_roundtrip } => {
                let path = resolve_source_path(base_dir, file);
                let data = std::fs::read(&path)
                    .map_err(|err| format!("failed to read '{}': {}", path.display(), err))?;
                let filename = Path::new(file)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(file);

                if *simulate {
                    os.simulate_noise = true;
                    os.noise_config = SimulationConfig {
                        seed: 42,
                        coverage_depth: *coverage as u32,
                        synthesis_profile: profile_to_hardware(*profile),
                        sequencing_profile: profile_to_hardware(*profile),
                        simulate_decay: false,
                        decay_rate: 0.0,
                        storage_time: 0.0,
                    };
                }

                let result = os.dna_write(filename, &data, *redundancy)?;
                steps.push(format!("✓ store into {}: {}", pool, result));

                if *verify_roundtrip {
                    let recovered = os.dna_read(filename)?;
                    if recovered == data {
                        steps.push(format!("✓ verify roundtrip: '{}' recovered exactly", filename));
                    } else {
                        return Err(format!("roundtrip verification failed for '{}'", filename));
                    }
                }
            }
            VfsCall::Retrieve { pool, query } => {
                let results = os.dna_search(query, 10);
                if results.is_empty() {
                    steps.push(format!("- retrieve from {} where {}: no matches", pool, query));
                } else {
                    steps.push(format!("✓ retrieve from {} where {}: {} match(es)", pool, query, results.len()));
                    for result in results {
                        steps.push(format!("  - {}", result));
                    }
                }
            }
            VfsCall::Delete { file, pool } => {
                let filename = Path::new(file)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(file);
                let result = os.dna_delete(filename)?;
                steps.push(format!(
                    "delete from {}: removed '{}' ({} strands)",
                    pool, result.filename, result.strands_removed
                ));
            }
        }
    }

    Ok(ExecutionReport { type_report: plan.type_report.clone(), steps, pool_status: os.dna_stat() })
}

fn resolve_source_path(base_dir: &Path, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn profile_to_hardware(profile: Profile) -> HardwareProfile {
    match profile {
        Profile::Illumina => HardwareProfile::Illumina,
        Profile::Nanopore => HardwareProfile::OxfordNanopore,
        Profile::Twist => HardwareProfile::TwistBioscience,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_pipeline_to_store_call() {
        let pool = PoolDecl { name: "archive".into(), codec: Codec::Ternary, redundancy: 3, profile: Profile::Illumina };
        let pipeline = PipelineDecl {
            name: "backup".into(),
            steps: vec![
                PipelineStep::Encode { path: "records.tar".into(), codec: Codec::Ternary },
                PipelineStep::Protect { redundancy: 4 },
                PipelineStep::Store { pool: "archive".into() },
                PipelineStep::VerifyRoundtrip,
            ],
        };
        let program = Program {
            declarations: vec![Declaration::Pool(pool), Declaration::Pipeline(pipeline)],
        };
        let plan = compile_program(program, TypeReport::default());
        let call = plan.calls.first().unwrap();
        assert!(matches!(call, VfsCall::Store { redundancy: 4, verify_roundtrip: true, .. }));
    }
}
