//! # Agent Tool Definitions
//!
//! Defines the available tools for the ReAct agent. Each tool wraps
//! a VFS syscall and provides structured input/output for the planner.
//!
//! Tools:
//! - `store_file` — encode and store data in DNA
//! - `retrieve_file` — read data back from DNA
//! - `search_files` — semantic file search
//! - `pool_status` — get storage pool statistics
//! - `delete_file` — remove a file from storage
//! - `list_files` — list all stored files

use std::fmt;

// ---------------------------------------------------------------------------
// Tool Registry
// ---------------------------------------------------------------------------

/// All available agent tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    StoreFile,
    RetrieveFile,
    SearchFiles,
    PoolStatus,
    DeleteFile,
    ListFiles,
}

impl ToolName {
    /// All available tools.
    pub const ALL: &'static [ToolName] = &[
        ToolName::StoreFile,
        ToolName::RetrieveFile,
        ToolName::SearchFiles,
        ToolName::PoolStatus,
        ToolName::DeleteFile,
        ToolName::ListFiles,
    ];

    /// Tool name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolName::StoreFile => "store_file",
            ToolName::RetrieveFile => "retrieve_file",
            ToolName::SearchFiles => "search_files",
            ToolName::PoolStatus => "pool_status",
            ToolName::DeleteFile => "delete_file",
            ToolName::ListFiles => "list_files",
        }
    }

    /// Parse a tool name from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "store_file" | "store" | "write" | "save" => Some(ToolName::StoreFile),
            "retrieve_file" | "retrieve" | "read" | "get" | "load" => Some(ToolName::RetrieveFile),
            "search_files" | "search" | "find" | "query" => Some(ToolName::SearchFiles),
            "pool_status" | "status" | "info" | "stats" => Some(ToolName::PoolStatus),
            "delete_file" | "delete" | "remove" | "rm" => Some(ToolName::DeleteFile),
            "list_files" | "list" | "ls" | "dir" => Some(ToolName::ListFiles),
            _ => None,
        }
    }

    /// Human-readable description of this tool.
    pub fn description(&self) -> &'static str {
        match self {
            ToolName::StoreFile =>
                "Store a file in DNA storage. Encodes data, adds ECC parity, tags with primers, and stores in the pool.",
            ToolName::RetrieveFile =>
                "Retrieve a file from DNA storage. Uses CRISPR to select strands, applies ECC recovery, and decodes.",
            ToolName::SearchFiles =>
                "Search for files by name, type, size, or semantic query. Returns ranked results.",
            ToolName::PoolStatus =>
                "Get DNA storage pool statistics: file count, strand count, nucleotides, redundancy.",
            ToolName::DeleteFile =>
                "Delete a file from DNA storage. Removes all strands, catalog entry, and search index.",
            ToolName::ListFiles =>
                "List all files currently stored in the DNA pool with their metadata.",
        }
    }

    /// Parameter specification for this tool.
    pub fn params(&self) -> Vec<ToolParam> {
        match self {
            ToolName::StoreFile => vec![
                ToolParam::required("filename", "Name for the stored file"),
                ToolParam::required("data", "Binary data to store (as text or hex)"),
                ToolParam::optional("redundancy", "Number of RS parity strands (default: 2)"),
            ],
            ToolName::RetrieveFile => vec![
                ToolParam::required("filename", "Name of the file to retrieve"),
            ],
            ToolName::SearchFiles => vec![
                ToolParam::required("query", "Search query (supports name:, type:, size: filters)"),
                ToolParam::optional("top_k", "Maximum results to return (default: 5)"),
            ],
            ToolName::PoolStatus => vec![],
            ToolName::DeleteFile => vec![
                ToolParam::required("filename", "Name of the file to delete"),
            ],
            ToolName::ListFiles => vec![],
        }
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Tool Parameters
// ---------------------------------------------------------------------------

/// A parameter for a tool.
#[derive(Debug, Clone)]
pub struct ToolParam {
    /// Parameter name.
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Whether this parameter is required.
    pub required: bool,
}

impl ToolParam {
    /// Create a required parameter.
    pub fn required(name: &'static str, description: &'static str) -> Self {
        Self { name, description, required: true }
    }

    /// Create an optional parameter.
    pub fn optional(name: &'static str, description: &'static str) -> Self {
        Self { name, description, required: false }
    }
}

// ---------------------------------------------------------------------------
// Tool Call — structured input to a tool
// ---------------------------------------------------------------------------

/// A structured call to a tool with named arguments.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Which tool to invoke.
    pub tool: ToolName,
    /// Named arguments.
    pub args: Vec<(String, String)>,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(tool: ToolName) -> Self {
        Self { tool, args: Vec::new() }
    }

    /// Add an argument.
    pub fn arg(mut self, name: &str, value: &str) -> Self {
        self.args.push((name.to_string(), value.to_string()));
        self
    }

    /// Get an argument value by name.
    pub fn get_arg(&self, name: &str) -> Option<&str> {
        self.args.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Get a required argument or return an error message.
    pub fn require_arg(&self, name: &str) -> Result<&str, String> {
        self.get_arg(name)
            .ok_or_else(|| format!("missing required argument: {}", name))
    }
}

impl fmt::Display for ToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.tool)?;
        for (i, (k, v)) in self.args.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}={:?}", k, v)?;
        }
        write!(f, ")")
    }
}

// ---------------------------------------------------------------------------
// Tool Result — structured output from a tool
// ---------------------------------------------------------------------------

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Whether the tool succeeded.
    pub success: bool,
    /// Human-readable output message.
    pub message: String,
    /// Structured data (optional).
    pub data: Option<String>,
}

impl ToolResult {
    /// Create a success result.
    pub fn ok(message: &str) -> Self {
        Self {
            success: true,
            message: message.to_string(),
            data: None,
        }
    }

    /// Create a success result with data.
    pub fn ok_with_data(message: &str, data: &str) -> Self {
        Self {
            success: true,
            message: message.to_string(),
            data: Some(data.to_string()),
        }
    }

    /// Create an error result.
    pub fn err(message: &str) -> Self {
        Self {
            success: false,
            message: message.to_string(),
            data: None,
        }
    }
}

impl fmt::Display for ToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icon = if self.success { "✓" } else { "✗" };
        write!(f, "[{}] {}", icon, self.message)?;
        if let Some(ref data) = self.data {
            write!(f, "\n{}", data)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tool Help — generate help text for all tools
// ---------------------------------------------------------------------------

/// Generate help text listing all available tools.
pub fn tools_help() -> String {
    let mut help = String::from("Available DNA Storage Tools:\n\n");
    for tool in ToolName::ALL {
        help.push_str(&format!("  {} — {}\n", tool.as_str(), tool.description()));
        for param in tool.params() {
            let marker = if param.required { "*" } else { " " };
            help.push_str(&format!("    {} {}: {}\n", marker, param.name, param.description));
        }
        help.push('\n');
    }
    help
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name_roundtrip() {
        for tool in ToolName::ALL {
            let name = tool.as_str();
            let parsed = ToolName::from_str(name).unwrap();
            assert_eq!(*tool, parsed);
        }
    }

    #[test]
    fn test_tool_aliases() {
        assert_eq!(ToolName::from_str("store"), Some(ToolName::StoreFile));
        assert_eq!(ToolName::from_str("write"), Some(ToolName::StoreFile));
        assert_eq!(ToolName::from_str("read"), Some(ToolName::RetrieveFile));
        assert_eq!(ToolName::from_str("find"), Some(ToolName::SearchFiles));
        assert_eq!(ToolName::from_str("ls"), Some(ToolName::ListFiles));
        assert_eq!(ToolName::from_str("rm"), Some(ToolName::DeleteFile));
        assert_eq!(ToolName::from_str("unknown"), None);
    }

    #[test]
    fn test_tool_call_builder() {
        let call = ToolCall::new(ToolName::StoreFile)
            .arg("filename", "test.txt")
            .arg("data", "hello world")
            .arg("redundancy", "4");

        assert_eq!(call.get_arg("filename"), Some("test.txt"));
        assert_eq!(call.get_arg("data"), Some("hello world"));
        assert_eq!(call.get_arg("redundancy"), Some("4"));
        assert_eq!(call.get_arg("missing"), None);
    }

    #[test]
    fn test_require_arg() {
        let call = ToolCall::new(ToolName::StoreFile)
            .arg("filename", "test.txt");

        assert!(call.require_arg("filename").is_ok());
        assert!(call.require_arg("missing").is_err());
    }

    #[test]
    fn test_tool_result_display() {
        let ok = ToolResult::ok("file stored successfully");
        assert!(format!("{}", ok).contains("✓"));

        let err = ToolResult::err("file not found");
        assert!(format!("{}", err).contains("✗"));
    }

    #[test]
    fn test_tools_help() {
        let help = tools_help();
        assert!(help.contains("store_file"));
        assert!(help.contains("retrieve_file"));
        assert!(help.contains("search_files"));
        assert!(help.contains("pool_status"));
    }

    #[test]
    fn test_tool_call_display() {
        let call = ToolCall::new(ToolName::StoreFile)
            .arg("filename", "readme.txt");
        let s = format!("{}", call);
        assert!(s.contains("store_file"));
        assert!(s.contains("readme.txt"));
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        for tool in ToolName::ALL {
            assert!(!tool.description().is_empty());
        }
    }
}
