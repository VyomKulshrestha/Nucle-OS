//! Abstract syntax tree for NucleScript.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Declaration {
    Import(ImportDecl),
    Pool(PoolDecl),
    Strand(StrandDecl),
    Sequence(SequenceDecl),
    Let(LetDecl),
    Operation(Operation),
    Pipeline(PipelineDecl),
    Function(FunctionDecl),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub source: String,
    pub items: Vec<ImportItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
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
pub struct LetDecl {
    pub name: String,
    pub annotation: TypeExpr,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub name: String,
    pub params: Vec<FnParam>,
    pub return_type: TypeExpr,
    pub body: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnParam {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeExpr {
    Pool(PoolType),
    Strand,
    Sequence,
    File,
    DnaFile,
    Recovery,
    Void,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolType {
    pub state: PoolState,
    pub error_rate_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolState {
    Profile(Profile),
    Amplified,
    Recovered,
}

impl PoolState {
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(profile) = Profile::parse(value) {
            return Some(Self::Profile(profile));
        }
        match value.to_ascii_lowercase().as_str() {
            "amplified" => Some(Self::Amplified),
            "recovered" => Some(Self::Recovered),
            _ => None,
        }
    }
}

impl std::fmt::Display for PoolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Profile(profile) => write!(f, "{}", profile),
            Self::Amplified => write!(f, "Amplified"),
            Self::Recovered => write!(f, "Recovered"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    SimulatePool { pool: String, profile: Profile },
    SynthesizePool {
        source: String,
        profile: Profile,
        confirmed: bool,
    },
    SequencePool {
        source: String,
        profile: Profile,
        confirmed: bool,
    },
    ConsensusVote { source: String, coverage: usize },
    FunctionCall { name: String, args: Vec<Expr> },
    Protect { data: String, guarantee: String },
    Variable(String),
    StringLiteral(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Pure,
    Synthesis,
    Sequencing,
    Destructive,
}

impl std::fmt::Display for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Pure => "Pure",
            Self::Synthesis => "Synthesis",
            Self::Sequencing => "Sequencing",
            Self::Destructive => "Destructive",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Store(StoreOp),
    Retrieve(RetrieveOp),
    Delete(DeleteOp),
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
pub struct DeleteOp {
    pub file: String,
    pub pool: String,
    pub confirmed: bool,
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
