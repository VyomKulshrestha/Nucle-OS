//! Abstract syntax tree for NucleScript.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Declaration {
    Pool(PoolDecl),
    Strand(StrandDecl),
    Sequence(SequenceDecl),
    Operation(Operation),
    Pipeline(PipelineDecl),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolDecl {
    pub name: String,
    pub codec: Codec,
    pub redundancy: usize,
    pub profile: Profile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrandDecl {
    pub name: String,
    pub sequence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceDecl {
    pub name: String,
    pub sequence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Store(StoreOp),
    Retrieve(RetrieveOp),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreOp {
    pub simulate: bool,
    pub file: String,
    pub pool: String,
    pub options: StoreOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreOptions {
    pub redundancy: Option<usize>,
    pub coverage: Option<usize>,
    pub tags: Vec<String>,
    pub expect_recovery_gt: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrieveOp {
    pub pool: String,
    pub query: Vec<QueryPredicate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryPredicate {
    pub field: String,
    pub op: QueryOp,
    pub value: QueryValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueryOp {
    Contains,
    Eq,
    Gt,
    Lt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueryValue {
    String(String),
    Number(f64),
    Date(String),
    SizeBytes(u64),
    Ident(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineDecl {
    pub name: String,
    pub steps: Vec<PipelineStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PipelineStep {
    Encode { path: String, codec: Codec },
    Protect { redundancy: usize },
    Store { pool: String },
    VerifyRoundtrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    YinYang,
    Ternary,
    Fountain,
}

impl Codec {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "yinyang" | "yin-yang" => Some(Self::YinYang),
            "ternary" | "ternary-rotating" | "ternary-rotating-cipher" => Some(Self::Ternary),
            "fountain" | "dna-fountain" => Some(Self::Fountain),
            _ => None,
        }
    }
}

impl std::fmt::Display for Codec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::YinYang => "YinYang",
            Self::Ternary => "Ternary",
            Self::Fountain => "Fountain",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Illumina,
    Nanopore,
    Twist,
}

impl Profile {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "illumina" => Some(Self::Illumina),
            "nanopore" | "oxfordnanopore" | "oxford-nanopore" => Some(Self::Nanopore),
            "twist" | "twistbioscience" | "twist-bioscience" => Some(Self::Twist),
            _ => None,
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Illumina => "Illumina",
            Self::Nanopore => "Nanopore",
            Self::Twist => "Twist",
        };
        write!(f, "{}", name)
    }
}
