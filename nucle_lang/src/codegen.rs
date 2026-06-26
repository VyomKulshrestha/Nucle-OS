//! VFS backend for NucleScript.

use crate::ast::*;
use crate::typeck::TypeReport;
use nucle_synth::noise::SimulationConfig;
use nucle_synth::profiles::HardwareProfile;
use nucle_vfs::syscall::{NucleOS, PoolStatus};
use std::collections::HashMap;
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
    let pools: HashMap<_, _> = program
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Declaration::Pool(pool) => Some((pool.name.clone(), pool.clone())),
            _ => None,
        })
        .collect();

    let mut calls = Vec::new();
    for declaration in &program.declarations {
        match declaration {
            Declaration::Operation(Operation::Store(store)) => {
                if let Some(pool) = pools.get(&store.pool) {
                    calls.push(store_call(store, pool, false));
                }
            }
            Declaration::Operation(Operation::Retrieve(retrieve)) => {
                calls.push(VfsCall::Retrieve {
                    pool: retrieve.pool.clone(),
                    query: query_to_vfs_search(&retrieve.query),
                });
            }
            Declaration::Pipeline(pipeline) => {
                if let Some(call) = pipeline_to_store_call(pipeline, &pools) {
                    calls.push(call);
                }
            }
            Declaration::Pool(_) | Declaration::Strand(_) | Declaration::Sequence(_) => {}
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
        }
    }

    Ok(ExecutionReport { type_report: plan.type_report.clone(), steps, pool_status: os.dna_stat() })
}

fn store_call(store: &StoreOp, pool: &PoolDecl, verify_roundtrip: bool) -> VfsCall {
    let redundancy = store.options.redundancy.unwrap_or(pool.redundancy);
    VfsCall::Store {
        file: store.file.clone(),
        pool: store.pool.clone(),
        redundancy,
        simulate: store.simulate,
        coverage: store.options.coverage.unwrap_or(redundancy),
        profile: pool.profile,
        verify_roundtrip,
    }
}

fn pipeline_to_store_call(pipeline: &PipelineDecl, pools: &HashMap<String, PoolDecl>) -> Option<VfsCall> {
    let mut file = None;
    let mut redundancy = None;
    let mut target_pool = None;
    let mut verify_roundtrip = false;

    for step in &pipeline.steps {
        match step {
            PipelineStep::Encode { path, .. } => file = Some(path.clone()),
            PipelineStep::Protect { redundancy: value } => redundancy = Some(*value),
            PipelineStep::Store { pool } => target_pool = Some(pool.clone()),
            PipelineStep::VerifyRoundtrip => verify_roundtrip = true,
        }
    }

    let file = file?;
    let target_pool = target_pool?;
    let pool = pools.get(&target_pool)?;
    let redundancy = redundancy.unwrap_or(pool.redundancy);
    Some(VfsCall::Store {
        file,
        pool: target_pool,
        redundancy,
        simulate: false,
        coverage: redundancy,
        profile: pool.profile,
        verify_roundtrip,
    })
}

fn query_to_vfs_search(query: &[QueryPredicate]) -> String {
    if query.is_empty() {
        return "".into();
    }
    query
        .iter()
        .map(|predicate| match (&predicate.op, &predicate.value) {
            (QueryOp::Contains, QueryValue::String(value)) => format!("{}:{}", predicate.field, value),
            (QueryOp::Eq, value) => format!("{}:{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Gt, value) => format!("{}>{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Lt, value) => format!("{}<{}", predicate.field, query_value_to_string(value)),
            (QueryOp::Contains, value) => format!("{}:{}", predicate.field, query_value_to_string(value)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn query_value_to_string(value: &QueryValue) -> String {
    match value {
        QueryValue::String(value) | QueryValue::Date(value) | QueryValue::Ident(value) => value.clone(),
        QueryValue::Number(value) => value.to_string(),
        QueryValue::SizeBytes(value) => value.to_string(),
    }
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
        let mut pools = HashMap::new();
        pools.insert(pool.name.clone(), pool);
        let call = pipeline_to_store_call(&pipeline, &pools).unwrap();
        assert!(matches!(call, VfsCall::Store { redundancy: 4, verify_roundtrip: true, .. }));
    }
}
