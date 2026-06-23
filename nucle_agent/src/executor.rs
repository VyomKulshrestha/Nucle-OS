//! # Pipeline Executor
//!
//! Executes planned tool calls through the NucleOS VFS layer.
//! The executor is the bridge between the agent's plan and the
//! actual DNA storage operations.
//!
//! ```text
//! Plan → Executor → NucleOS (VFS) → Result
//! ```

use crate::tools::{ToolName, ToolCall, ToolResult};
use crate::planner::{Plan, Planner};
use nucle_vfs::syscall::NucleOS;
use std::fmt;

// ---------------------------------------------------------------------------
// Execution Result
// ---------------------------------------------------------------------------

/// Result of executing an entire plan.
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    /// The plan that was executed.
    pub plan_description: String,
    /// Results for each step.
    pub step_results: Vec<StepResult>,
    /// Whether all steps succeeded.
    pub success: bool,
}

/// Result of a single step in the plan.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// The tool that was called.
    pub tool: String,
    /// The tool's result.
    pub result: ToolResult,
}

impl fmt::Display for ExecutionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icon = if self.success { "✓" } else { "✗" };
        writeln!(f, "[{}] {}", icon, self.plan_description)?;
        for (i, step) in self.step_results.iter().enumerate() {
            writeln!(f, "  Step {}: [{}] {}", i + 1, step.tool, step.result)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Executes plans against a NucleOS instance.
pub struct Executor;

impl Executor {
    /// Execute a plan against the given NucleOS instance.
    pub fn execute(os: &mut NucleOS, plan: &Plan) -> ExecutionReport {
        let mut step_results = Vec::new();
        let mut all_success = true;

        for call in &plan.steps {
            let result = Self::execute_tool(os, call);
            if !result.success {
                all_success = false;
            }
            step_results.push(StepResult {
                tool: call.tool.as_str().to_string(),
                result,
            });
        }

        ExecutionReport {
            plan_description: plan.description.clone(),
            step_results,
            success: all_success,
        }
    }

    /// Execute a single tool call.
    fn execute_tool(os: &mut NucleOS, call: &ToolCall) -> ToolResult {
        match call.tool {
            ToolName::StoreFile => Self::exec_store(os, call),
            ToolName::RetrieveFile => Self::exec_retrieve(os, call),
            ToolName::SearchFiles => Self::exec_search(os, call),
            ToolName::PoolStatus => Self::exec_status(os),
            ToolName::DeleteFile => Self::exec_delete(os, call),
            ToolName::ListFiles => Self::exec_list(os),
        }
    }

    /// Execute store_file tool.
    fn exec_store(os: &mut NucleOS, call: &ToolCall) -> ToolResult {
        let filename = match call.require_arg("filename") {
            Ok(f) => f,
            Err(e) => return ToolResult::err(&e),
        };

        let data = call.get_arg("data").unwrap_or("").as_bytes();

        let redundancy: usize = call.get_arg("redundancy")
            .and_then(|r| r.parse().ok())
            .unwrap_or(2);

        match os.dna_write(filename, data, redundancy) {
            Ok(result) => ToolResult::ok_with_data(
                &format!("{}", result),
                &format!(
                    "file_id={}, strands={}, parity={}",
                    result.file_id, result.data_strand_count, result.parity_strand_count
                ),
            ),
            Err(e) => ToolResult::err(&e),
        }
    }

    /// Execute retrieve_file tool.
    fn exec_retrieve(os: &mut NucleOS, call: &ToolCall) -> ToolResult {
        let filename = match call.require_arg("filename") {
            Ok(f) => f,
            Err(e) => return ToolResult::err(&e),
        };

        match os.dna_read(filename) {
            Ok(data) => {
                let text = String::from_utf8_lossy(&data);
                ToolResult::ok_with_data(
                    &format!("Retrieved '{}' ({} bytes)", filename, data.len()),
                    &format!("{}", text),
                )
            }
            Err(e) => ToolResult::err(&e),
        }
    }

    /// Execute search_files tool.
    fn exec_search(os: &mut NucleOS, call: &ToolCall) -> ToolResult {
        let query = match call.require_arg("query") {
            Ok(q) => q,
            Err(e) => return ToolResult::err(&e),
        };

        let top_k: usize = call.get_arg("top_k")
            .and_then(|k| k.parse().ok())
            .unwrap_or(5);

        let results = os.dna_search(query, top_k);

        if results.is_empty() {
            return ToolResult::ok("No matching files found");
        }

        let mut output = format!("Found {} results:\n", results.len());
        for (i, r) in results.iter().enumerate() {
            output.push_str(&format!("  {}. {}\n", i + 1, r));
        }

        ToolResult::ok_with_data(
            &format!("Found {} matching files", results.len()),
            &output,
        )
    }

    /// Execute pool_status tool.
    fn exec_status(os: &mut NucleOS) -> ToolResult {
        let status = os.dna_stat();
        ToolResult::ok_with_data(
            &format!("{} files, {} strands", status.file_count, status.total_strands),
            &format!("{}", status),
        )
    }

    /// Execute delete_file tool.
    fn exec_delete(os: &mut NucleOS, call: &ToolCall) -> ToolResult {
        let filename = match call.require_arg("filename") {
            Ok(f) => f,
            Err(e) => return ToolResult::err(&e),
        };

        match os.dna_delete(filename) {
            Ok(result) => ToolResult::ok(
                &format!("Deleted '{}' ({} strands removed)", result.filename, result.strands_removed),
            ),
            Err(e) => ToolResult::err(&e),
        }
    }

    /// Execute list_files tool.
    fn exec_list(os: &mut NucleOS) -> ToolResult {
        let status = os.dna_stat();

        if status.files.is_empty() {
            return ToolResult::ok("No files stored");
        }

        let mut output = format!("{} files:\n", status.file_count);
        for fi in &status.files {
            output.push_str(&format!(
                "  {} ({} B, {}d+{}p strands, {:.1}×)\n",
                fi.filename, fi.size, fi.data_strands, fi.parity_strands, fi.redundancy
            ));
        }

        ToolResult::ok_with_data(
            &format!("{} files in pool", status.file_count),
            &output,
        )
    }

    // -----------------------------------------------------------------------
    // Convenience: plan + execute in one call
    // -----------------------------------------------------------------------

    /// Parse a natural language command, plan it, and execute it.
    pub fn run(os: &mut NucleOS, command: &str) -> Result<ExecutionReport, String> {
        let plan = Planner::plan(command)?;
        Ok(Self::execute(os, &plan))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_store_and_retrieve() {
        let mut os = NucleOS::new(10);

        // Store
        let report = Executor::run(&mut os, "store \"test.txt\" data \"hello world\"").unwrap();
        assert!(report.success);

        // Retrieve
        let report = Executor::run(&mut os, "retrieve test.txt").unwrap();
        assert!(report.success);
    }

    #[test]
    fn test_execute_status() {
        let mut os = NucleOS::new(10);
        let report = Executor::run(&mut os, "pool status").unwrap();
        assert!(report.success);
    }

    #[test]
    fn test_execute_list_empty() {
        let mut os = NucleOS::new(10);
        let report = Executor::run(&mut os, "list files").unwrap();
        assert!(report.success);
        assert!(report.step_results[0].result.message.contains("No files"));
    }

    #[test]
    fn test_execute_store_list_delete() {
        let mut os = NucleOS::new(10);

        // Store directly via NucleOS for reliable data
        os.dna_write("notes.txt", b"my notes", 0).unwrap();

        // List
        let report = Executor::run(&mut os, "list files").unwrap();
        let data = report.step_results[0].result.data.as_ref().unwrap();
        assert!(data.contains("notes.txt"), "list should contain notes.txt, got: {}", data);

        // Delete
        let report = Executor::run(&mut os, "delete notes.txt").unwrap();
        assert!(report.success);

        // List again — empty
        let report = Executor::run(&mut os, "list files").unwrap();
        assert!(report.step_results[0].result.message.contains("No files"));
    }

    #[test]
    fn test_execute_search() {
        let mut os = NucleOS::new(10);
        Executor::run(&mut os, "store readme.txt data \"hello\"").unwrap();

        let report = Executor::run(&mut os, "search readme").unwrap();
        assert!(report.success);
    }

    #[test]
    fn test_execute_retrieve_nonexistent() {
        let mut os = NucleOS::new(10);
        let report = Executor::run(&mut os, "retrieve missing.txt").unwrap();
        assert!(!report.success);
    }

    #[test]
    fn test_execution_report_display() {
        let mut os = NucleOS::new(10);
        let report = Executor::run(&mut os, "status").unwrap();
        let display = format!("{}", report);
        assert!(display.contains("✓"));
    }

    #[test]
    fn test_store_with_redundancy() {
        let mut os = NucleOS::new(10);

        // Use direct NucleOS API to verify ECC integration
        os.dna_write("backup.dat", b"important backup data", 4).unwrap();

        let status = os.dna_stat();
        assert!(status.parity_strands > 0, "should have parity strands with redundancy=4");
        assert!(status.redundancy > 1.0);
    }
}
