//! # ReAct Agent Planner
//!
//! Implements a rule-based ReAct (Reason + Act) loop:
//!
//! ```text
//! Input → Parse → Reason → Select Tool → Build ToolCall → Execute → Observe
//! ```
//!
//! No external LLM dependency — uses pattern matching and keyword
//! extraction for natural language → tool call translation.

use crate::tools::{ToolName, ToolCall};
use std::fmt;

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// A plan is a sequence of tool calls to execute.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Human-readable description of what this plan does.
    pub description: String,
    /// Ordered list of tool calls to execute.
    pub steps: Vec<ToolCall>,
}

impl Plan {
    /// Create an empty plan.
    pub fn new(description: &str) -> Self {
        Self {
            description: description.to_string(),
            steps: Vec::new(),
        }
    }

    /// Add a step to the plan.
    pub fn step(mut self, call: ToolCall) -> Self {
        self.steps.push(call);
        self
    }

    /// Number of steps.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the plan is empty.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Plan: {}", self.description)?;
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, step)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

/// The ReAct planner: translates natural language commands into plans.
pub struct Planner;

impl Planner {
    /// Plan a natural language command.
    ///
    /// Returns a Plan with one or more tool calls, or an error if
    /// the command couldn't be understood.
    pub fn plan(input: &str) -> Result<Plan, String> {
        let lower = input.to_lowercase();
        let tokens: Vec<&str> = lower.split_whitespace().collect();

        if tokens.is_empty() {
            return Err("empty command".into());
        }

        // ── Store / Write ──
        if Self::matches_any(&lower, &["store", "save", "write", "encode", "upload"]) {
            return Self::plan_store(input, &lower);
        }

        // ── Retrieve / Read ──
        if Self::matches_any(&lower, &["retrieve", "read", "get", "load", "decode", "download"]) {
            return Self::plan_retrieve(input, &lower);
        }

        // ── Search / Find ──
        if Self::matches_any(&lower, &["search", "find", "query", "look"]) {
            return Self::plan_search(input, &lower);
        }

        // ── Delete / Remove ──
        if Self::matches_any(&lower, &["delete", "remove", "rm", "erase"]) {
            return Self::plan_delete(input, &lower);
        }

        // ── Status / Info ──
        if Self::matches_any(&lower, &["status", "info", "stats", "stat", "pool", "health"]) {
            return Ok(Plan::new("Get pool status")
                .step(ToolCall::new(ToolName::PoolStatus)));
        }

        // ── List ──
        if Self::matches_any(&lower, &["list", "ls", "dir", "files"]) {
            return Ok(Plan::new("List all files")
                .step(ToolCall::new(ToolName::ListFiles)));
        }

        // ── Help ──
        if Self::matches_any(&lower, &["help", "tools", "commands"]) {
            return Ok(Plan::new("Show help")
                .step(ToolCall::new(ToolName::PoolStatus)));
        }

        Err(format!("could not understand command: '{}'", input))
    }

    /// Plan a store operation.
    fn plan_store(input: &str, lower: &str) -> Result<Plan, String> {
        // Extract filename (look for quoted string or word after "as" / file-like tokens)
        let filename = Self::extract_filename(input)
            .unwrap_or_else(|| "untitled.dat".to_string());

        // Extract redundancy
        let redundancy = Self::extract_redundancy(lower);

        // Extract data (everything that isn't a keyword)
        let data = Self::extract_data(input);

        let mut call = ToolCall::new(ToolName::StoreFile)
            .arg("filename", &filename);

        if !data.is_empty() {
            call = call.arg("data", &data);
        }

        call = call.arg("redundancy", &redundancy.to_string());

        let desc = format!(
            "Store '{}' with {}× redundancy",
            filename, redundancy
        );

        Ok(Plan::new(&desc).step(call))
    }

    /// Plan a retrieve operation.
    fn plan_retrieve(input: &str, _lower: &str) -> Result<Plan, String> {
        let filename = Self::extract_filename(input)
            .ok_or("could not determine filename to retrieve")?;

        Ok(Plan::new(&format!("Retrieve '{}'", filename))
            .step(ToolCall::new(ToolName::RetrieveFile)
                .arg("filename", &filename)))
    }

    /// Plan a search operation.
    fn plan_search(input: &str, lower: &str) -> Result<Plan, String> {
        // Extract the search query (everything after "search"/"find")
        let query = Self::extract_after_keyword(lower, &["search", "find", "query", "look for"])
            .unwrap_or_else(|| input.to_string());

        Ok(Plan::new(&format!("Search for '{}'", query))
            .step(ToolCall::new(ToolName::SearchFiles)
                .arg("query", &query)
                .arg("top_k", "5")))
    }

    /// Plan a delete operation.
    fn plan_delete(input: &str, _lower: &str) -> Result<Plan, String> {
        let filename = Self::extract_filename(input)
            .ok_or("could not determine filename to delete")?;

        Ok(Plan::new(&format!("Delete '{}'", filename))
            .step(ToolCall::new(ToolName::DeleteFile)
                .arg("filename", &filename)))
    }

    // -----------------------------------------------------------------------
    // Extraction helpers
    // -----------------------------------------------------------------------

    /// Check if the input contains any of the given keywords.
    fn matches_any(input: &str, keywords: &[&str]) -> bool {
        keywords.iter().any(|k| {
            input.split_whitespace().any(|w| w == *k)
        })
    }

    /// Extract a filename from the input.
    ///
    /// Looks for:
    /// 1. Quoted strings: "filename.txt" or 'filename.txt'
    /// 2. Words with file extensions: readme.txt, photo.jpg
    /// 3. Word after "as": store data as myfile.txt
    fn extract_filename(input: &str) -> Option<String> {
        // Check for quoted strings
        if let Some(start) = input.find('"') {
            if let Some(end) = input[start + 1..].find('"') {
                return Some(input[start + 1..start + 1 + end].to_string());
            }
        }
        if let Some(start) = input.find('\'') {
            if let Some(end) = input[start + 1..].find('\'') {
                return Some(input[start + 1..start + 1 + end].to_string());
            }
        }

        // Check for "as <filename>"
        let lower = input.to_lowercase();
        if let Some(pos) = lower.find(" as ") {
            let after = input[pos + 4..].trim();
            let word = after.split_whitespace().next();
            if let Some(w) = word {
                if w.contains('.') {
                    return Some(w.to_string());
                }
            }
        }

        // Look for words with common file extensions
        let extensions = [
            ".txt", ".bin", ".dat", ".csv", ".json", ".xml", ".md",
            ".jpg", ".png", ".gif", ".pdf", ".doc", ".zip", ".tar",
            ".log", ".html", ".rs", ".py", ".js", ".toml", ".yaml",
        ];

        for word in input.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-');
            if extensions.iter().any(|ext| clean.to_lowercase().ends_with(ext)) {
                return Some(clean.to_string());
            }
        }

        None
    }

    /// Extract redundancy level from input.
    ///
    /// Looks for patterns like "3x redundancy", "redundancy 4", "with 2x"
    fn extract_redundancy(lower: &str) -> usize {
        // Pattern: Nx redundancy
        for word in lower.split_whitespace() {
            if word.ends_with('x') || word.ends_with("×") {
                let num_str = word.trim_end_matches('x').trim_end_matches('×');
                if let Ok(n) = num_str.parse::<usize>() {
                    return n;
                }
            }
        }

        // Pattern: "redundancy N" or "parity N"
        let tokens: Vec<&str> = lower.split_whitespace().collect();
        for (i, &token) in tokens.iter().enumerate() {
            if (token == "redundancy" || token == "parity") && i + 1 < tokens.len() {
                if let Ok(n) = tokens[i + 1].parse::<usize>() {
                    return n;
                }
            }
        }

        // Default: 2 parity strands
        2
    }

    /// Extract data content from input (everything after data-like keywords).
    fn extract_data(input: &str) -> String {
        // Look for content in quotes after "data" or "content"
        let lower = input.to_lowercase();
        for keyword in &["data", "content", "text", "message"] {
            if let Some(pos) = lower.find(keyword) {
                let after = &input[pos + keyword.len()..];
                let trimmed = after.trim().trim_start_matches([':', '=', ' ']);
                if !trimmed.is_empty() {
                    // Take quoted content or first sentence
                    if trimmed.starts_with('"') {
                        if let Some(end) = trimmed[1..].find('"') {
                            return trimmed[1..1 + end].to_string();
                        }
                    }
                    return trimmed.to_string();
                }
            }
        }
        String::new()
    }

    /// Extract text after a keyword.
    fn extract_after_keyword(input: &str, keywords: &[&str]) -> Option<String> {
        for keyword in keywords {
            if let Some(pos) = input.find(keyword) {
                let after = input[pos + keyword.len()..].trim();
                if !after.is_empty() {
                    return Some(after.to_string());
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_store_basic() {
        let plan = Planner::plan("store readme.txt").unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].tool, ToolName::StoreFile);
        assert_eq!(plan.steps[0].get_arg("filename"), Some("readme.txt"));
    }

    #[test]
    fn test_plan_store_with_redundancy() {
        let plan = Planner::plan("store data.bin with 4x redundancy").unwrap();
        assert_eq!(plan.steps[0].get_arg("redundancy"), Some("4"));
    }

    #[test]
    fn test_plan_store_quoted_filename() {
        let plan = Planner::plan("save \"my file.txt\"").unwrap();
        assert_eq!(plan.steps[0].get_arg("filename"), Some("my file.txt"));
    }

    #[test]
    fn test_plan_retrieve() {
        let plan = Planner::plan("retrieve readme.txt").unwrap();
        assert_eq!(plan.steps[0].tool, ToolName::RetrieveFile);
        assert_eq!(plan.steps[0].get_arg("filename"), Some("readme.txt"));
    }

    #[test]
    fn test_plan_search() {
        let plan = Planner::plan("search for readme files").unwrap();
        assert_eq!(plan.steps[0].tool, ToolName::SearchFiles);
    }

    #[test]
    fn test_plan_delete() {
        let plan = Planner::plan("delete old_data.csv").unwrap();
        assert_eq!(plan.steps[0].tool, ToolName::DeleteFile);
        assert_eq!(plan.steps[0].get_arg("filename"), Some("old_data.csv"));
    }

    #[test]
    fn test_plan_status() {
        let plan = Planner::plan("pool status").unwrap();
        assert_eq!(plan.steps[0].tool, ToolName::PoolStatus);
    }

    #[test]
    fn test_plan_list() {
        let plan = Planner::plan("list files").unwrap();
        assert_eq!(plan.steps[0].tool, ToolName::ListFiles);
    }

    #[test]
    fn test_plan_unknown() {
        assert!(Planner::plan("juggle bananas").is_err());
    }

    #[test]
    fn test_extract_redundancy() {
        assert_eq!(Planner::extract_redundancy("3x redundancy"), 3);
        assert_eq!(Planner::extract_redundancy("with parity 5"), 5);
        assert_eq!(Planner::extract_redundancy("no special keywords"), 2); // default
    }

    #[test]
    fn test_plan_display() {
        let plan = Planner::plan("store test.txt").unwrap();
        let display = format!("{}", plan);
        assert!(display.contains("Plan:"));
        assert!(display.contains("store_file"));
    }
}
