//! LSP protocol adapter over `nucle_lang::analyze`.
//!
//! Deliberately thin: every method here converts between LSP wire types
//! and `nucle_lang`'s own `Diagnostic`/`SymbolTable`/`Span` shapes. It does
//! not re-implement or duplicate any compiler logic -- if `nucle check`
//! and this server ever disagreed about whether a program is valid, that
//! would be a bug in this file's conversion, not a second source of truth.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use nucle_lang::ast::{FnEffectAnnotation, Span, TypeExpr};
use nucle_lang::{
    analyze, Diagnostic as NucleDiagnostic, DiagnosticLevel, Lexer, SymbolTable, Token, TokenKind,
};

const KEYWORDS: &[&str] = &[
    "import",
    "from",
    "as",
    "pool",
    "strand",
    "seq",
    "let",
    "fn",
    "returns",
    "store",
    "simulate",
    "into",
    "retrieve",
    "where",
    "contains",
    "delete",
    "assert",
    "pipeline",
    "encode",
    "using",
    "protect",
    "with",
    "redundancy",
    "coverage",
    "tag",
    "tags",
    "expect",
    "recovery",
    "verify",
    "roundtrip",
    "synthesise",
    "synthesize",
    "sequence",
    "consensus_vote",
    "under",
    "via",
    "for",
    "if",
    "else",
    "in",
    "test",
    "match",
    "enum",
    "confirm",
    "hardware",
    "physical_key",
    "Ok",
    "Err",
];

const BUILTIN_TYPES: &[&str] = &[
    "DnaPool", "Pool", "Strand", "Sequence", "File", "DnaFile", "Recovery", "Void", "Result",
    "Str", "Fn",
];

const BUILTIN_FUNCTIONS: &[&str] = &["consensus_vote", "protect"];

const TOKEN_KEYWORD: u32 = 0;
const TOKEN_FUNCTION: u32 = 1;
const TOKEN_VARIABLE: u32 = 2;
const TOKEN_TYPE: u32 = 3;
const TOKEN_ENUM_MEMBER: u32 = 4;
const TOKEN_NUMBER: u32 = 5;
const TOKEN_STRING: u32 = 6;
const TOKEN_COMMENT: u32 = 7;
const TOKEN_OPERATOR: u32 = 8;

pub struct Backend {
    client: Client,
    documents: Arc<Mutex<HashMap<Url, String>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn on_change(&self, uri: Url, text: String) {
        let analysis = analyze(&text);
        let diagnostics = analysis
            .report
            .diagnostics
            .iter()
            .map(to_lsp_diagnostic)
            .collect();
        self.documents.lock().await.insert(uri.clone(), text);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                completion_provider: Some(CompletionOptions::default()),
                semantic_tokens_provider: Some(
                    SemanticTokensOptions {
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        legend: semantic_tokens_legend(),
                        range: None,
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                    }
                    .into(),
                ),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "nucle_lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "NucleScript language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.on_change(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full document sync (declared in `initialize`), so the last
        // reported change already contains the entire new text.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .await
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(uri) else {
            return Ok(None);
        };
        let Some(word) = word_at_position(text, position) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        Ok(
            hover_text_for(&word, &analysis.symbols).map(|contents| Hover {
                contents: HoverContents::Scalar(MarkedString::String(contents)),
                range: None,
            }),
        )
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else {
            return Ok(None);
        };
        let Some(word) = word_at_position(text, position) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        Ok(definition_span_for(&word, &analysis.symbols).map(|span| {
            GotoDefinitionResponse::Scalar(Location {
                uri,
                range: span_to_range(span),
            })
        }))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        Ok(Some(DocumentSymbolResponse::Nested(document_symbols(
            &analysis.symbols,
        ))))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(uri) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        Ok(Some(CompletionResponse::Array(completion_items(
            &analysis.symbols,
        ))))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else {
            return Ok(None);
        };
        let Some((name, _)) = identifier_at_position(text, position) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        if definition_span_for(&name, &analysis.symbols).is_none() || has_local_shadow(text, &name)
        {
            return Ok(None);
        }

        let declaration_range = declaration_identifier_range(text, &name);
        let locations = identifier_occurrences(text, &name)
            .into_iter()
            .filter(|range| params.context.include_declaration || Some(*range) != declaration_range)
            .map(|range| Location {
                uri: uri.clone(),
                range,
            })
            .collect();
        Ok(Some(locations))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(uri) else {
            return Ok(None);
        };
        let Some((name, range)) = identifier_at_position(text, params.position) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        if definition_span_for(&name, &analysis.symbols).is_none() || has_local_shadow(text, &name)
        {
            return Ok(None);
        }
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: name,
        }))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else {
            return Ok(None);
        };
        let Some((name, _)) = identifier_at_position(text, position) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        if definition_span_for(&name, &analysis.symbols).is_none()
            || has_local_shadow(text, &name)
            || !is_valid_identifier(&params.new_name)
            || (params.new_name != name
                && definition_span_for(&params.new_name, &analysis.symbols).is_some())
        {
            return Ok(None);
        }

        let edits = identifier_occurrences(text, &name)
            .into_iter()
            .map(|range| TextEdit {
                range,
                new_text: params.new_name.clone(),
            })
            .collect();
        let mut changes = HashMap::new();
        changes.insert(uri, edits);
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(uri) else {
            return Ok(None);
        };
        let analysis = analyze(text);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: semantic_tokens(text, &analysis.symbols),
        })))
    }
}

fn to_lsp_diagnostic(diagnostic: &NucleDiagnostic) -> Diagnostic {
    Diagnostic {
        range: span_to_range(diagnostic.span),
        severity: Some(match diagnostic.level {
            DiagnosticLevel::Error => DiagnosticSeverity::ERROR,
            DiagnosticLevel::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(diagnostic.code.clone())),
        source: Some("nuclescript".to_string()),
        message: diagnostic.message.clone(),
        ..Default::default()
    }
}

fn span_to_range(span: Span) -> Range {
    let start = Position {
        line: span.line.saturating_sub(1) as u32,
        character: span.column.saturating_sub(1) as u32,
    };
    // Guarantee end > start even for a degenerate (point) span, so editors
    // always have something visible to underline.
    let end_column = if span.end_line == span.line && span.end_column > span.column {
        span.end_column
    } else {
        span.column + 1
    };
    let end = Position {
        line: span.end_line.saturating_sub(1).max(start.line as usize) as u32,
        character: end_column.saturating_sub(1) as u32,
    };
    Range { start, end }
}

/// The identifier at `position` in `text` (LSP positions are 0-indexed
/// line/UTF-16-character). The conversion stays correct when an ASCII
/// NucleScript identifier follows non-BMP text such as a DNA emoji in a
/// string or comment. Expands in both directions from the cursor so it
/// works whether the cursor sits inside, at the start of, or immediately
/// after the word.
fn word_at_position(text: &str, position: Position) -> Option<String> {
    identifier_at_position(text, position).map(|(word, _)| word)
}

fn identifier_at_position(text: &str, position: Position) -> Option<(String, Range)> {
    let line = text.lines().nth(position.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let requested_utf16 = position.character as usize;
    let mut utf16_seen = 0;
    let mut col = 0;
    while col < chars.len() && utf16_seen < requested_utf16 {
        utf16_seen += chars[col].len_utf16();
        col += 1;
    }
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';

    let mut end = col;
    if end < chars.len() && is_word_char(chars[end]) {
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }
    }
    let mut start = col;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let start_utf16 = chars[..start].iter().map(|c| c.len_utf16() as u32).sum();
    let end_utf16 = chars[..end].iter().map(|c| c.len_utf16() as u32).sum();
    Some((
        chars[start..end].iter().collect(),
        Range {
            start: Position {
                line: position.line,
                character: start_utf16,
            },
            end: Position {
                line: position.line,
                character: end_utf16,
            },
        },
    ))
}

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !KEYWORDS
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(name))
}

fn identifier_occurrences(text: &str, name: &str) -> Vec<Range> {
    let Ok(tokens) = Lexer::new(text).tokenize() else {
        return Vec::new();
    };
    tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::Ident(value) if value == name => Some(token_range(text, token)),
            _ => None,
        })
        .collect()
}

/// A top-level symbol can only be renamed when doing so is unambiguous.
/// NucleScript currently exposes top-level symbols in `SymbolTable`, but
/// not a full scope graph for function parameters and compile-time loop/
/// match binders. Refusing the operation when one of those shadows the
/// requested name is safer than producing a text edit that changes two
/// different symbols.
fn has_local_shadow(text: &str, name: &str) -> bool {
    let Ok(tokens) = Lexer::new(text).tokenize() else {
        return true;
    };
    let mut brace_depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        let TokenKind::Ident(value) = &token.kind else {
            continue;
        };
        if value != name {
            continue;
        }
        let previous_ident = index.checked_sub(1).and_then(|i| match &tokens[i].kind {
            TokenKind::Ident(value) => Some(value.as_str()),
            _ => None,
        });
        if (previous_ident == Some("let") && brace_depth > 0) || previous_ident == Some("for") {
            return true;
        }
        let previous_is_parameter_separator = index
            .checked_sub(1)
            .is_some_and(|i| matches!(tokens[i].kind, TokenKind::LParen | TokenKind::Comma));
        let next_is_colon = tokens
            .get(index + 1)
            .is_some_and(|token| matches!(token.kind, TokenKind::Colon));
        if previous_is_parameter_separator && next_is_colon {
            return true;
        }
        let is_match_binding = index
            .checked_sub(1)
            .is_some_and(|i| matches!(tokens[i].kind, TokenKind::LParen))
            && tokens
                .get(index + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::RParen))
            && tokens
                .get(index + 2)
                .is_some_and(|token| matches!(token.kind, TokenKind::FatArrow));
        if is_match_binding {
            return true;
        }
    }
    false
}

fn hover_text_for(name: &str, symbols: &SymbolTable) -> Option<String> {
    if let Some(pool) = symbols.pools.get(name) {
        return Some(format!(
            "```nuclescript\npool {}: DnaPool {{ codec: {}, redundancy: {}x, profile: {} }}\n```",
            name, pool.codec, pool.redundancy, pool.profile
        ));
    }
    if let Some(func) = symbols.functions.get(name) {
        let params: Vec<String> = func
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, describe_type(&p.ty)))
            .collect();
        let type_params = if func.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", func.type_params.join(", "))
        };
        return Some(format!(
            "```nuclescript\nfn {}{}({}) -> {}\n```",
            name,
            type_params,
            params.join(", "),
            describe_type(&func.return_type)
        ));
    }
    if symbols.strands.contains_key(name) {
        return Some(format!("```nuclescript\nstrand {}: Strand\n```", name));
    }
    if symbols.sequences.contains_key(name) {
        return Some(format!("```nuclescript\nseq {}: Sequence\n```", name));
    }
    if let Some(decl) = symbols.enums.get(name) {
        let variants: Vec<String> = decl
            .variants
            .iter()
            .map(|v| match &v.payload {
                Some(ty) => format!("{}({})", v.name, describe_type(ty)),
                None => v.name.clone(),
            })
            .collect();
        return Some(format!(
            "```nuclescript\nenum {} {{ {} }}\n```",
            name,
            variants.join(", ")
        ));
    }
    if let Some(binding) = symbols.bindings.get(name) {
        return Some(format!(
            "```nuclescript\nlet {}: {}\n```",
            name,
            describe_type(&binding.annotation)
        ));
    }
    None
}

fn definition_span_for(name: &str, symbols: &SymbolTable) -> Option<Span> {
    if let Some(pool) = symbols.pools.get(name) {
        return Some(pool.span);
    }
    if let Some(func) = symbols.functions.get(name) {
        return Some(func.span);
    }
    if let Some(span) = symbols.strands.get(name) {
        return Some(*span);
    }
    if let Some(span) = symbols.sequences.get(name) {
        return Some(*span);
    }
    if let Some(decl) = symbols.enums.get(name) {
        return Some(decl.span);
    }
    if let Some(binding) = symbols.bindings.get(name) {
        return Some(binding.span);
    }
    None
}

fn describe_type(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Pool(pool_type) => match pool_type.error_rate_percent {
            Some(pct) => format!("Pool<{}, {:.2}%>", pool_type.state, pct),
            None => format!("Pool<{}>", pool_type.state),
        },
        TypeExpr::Strand => "Strand".to_string(),
        TypeExpr::Sequence => "Sequence".to_string(),
        TypeExpr::File => "File".to_string(),
        TypeExpr::DnaFile => "DnaFile".to_string(),
        TypeExpr::Recovery => "Recovery".to_string(),
        TypeExpr::Void => "Void".to_string(),
        TypeExpr::Result(ok, err) => {
            format!("Result<{}, {}>", describe_type(ok), describe_type(err))
        }
        TypeExpr::Str => "Str".to_string(),
        TypeExpr::Fn(params, ret, effect) => format!(
            "Fn({}) -> {}{}",
            params
                .iter()
                .map(describe_type)
                .collect::<Vec<_>>()
                .join(", "),
            describe_type(ret),
            match effect {
                Some(FnEffectAnnotation::Hardware) => " confirm hardware",
                Some(FnEffectAnnotation::PhysicalKey) => " confirm physical_key",
                None => "",
            }
        ),
        TypeExpr::Enum(name) => name.clone(),
    }
}

fn completion_items(symbols: &SymbolTable) -> Vec<CompletionItem> {
    let mut items = BTreeMap::new();
    let mut insert = |label: String, kind: CompletionItemKind, detail: String| {
        items
            .entry(label.clone())
            .or_insert_with(|| CompletionItem {
                label,
                kind: Some(kind),
                detail: Some(detail),
                ..Default::default()
            });
    };

    for builtin in BUILTIN_FUNCTIONS {
        insert(
            (*builtin).to_string(),
            CompletionItemKind::FUNCTION,
            "NucleScript standard library".to_string(),
        );
    }
    for keyword in KEYWORDS {
        insert(
            (*keyword).to_string(),
            CompletionItemKind::KEYWORD,
            "NucleScript keyword".to_string(),
        );
    }
    for ty in BUILTIN_TYPES {
        insert(
            (*ty).to_string(),
            CompletionItemKind::CLASS,
            "NucleScript type".to_string(),
        );
    }
    for (name, pool) in &symbols.pools {
        insert(
            name.clone(),
            CompletionItemKind::VARIABLE,
            format!("DnaPool<{}>", pool.profile),
        );
    }
    for (name, function) in &symbols.functions {
        let params = function
            .params
            .iter()
            .map(|param| format!("{}: {}", param.name, describe_type(&param.ty)))
            .collect::<Vec<_>>()
            .join(", ");
        insert(
            name.clone(),
            CompletionItemKind::FUNCTION,
            format!(
                "fn {}({params}) -> {}",
                name,
                describe_type(&function.return_type)
            ),
        );
    }
    for name in symbols.strands.keys() {
        insert(
            name.clone(),
            CompletionItemKind::VARIABLE,
            "Strand".to_string(),
        );
    }
    for name in symbols.sequences.keys() {
        insert(
            name.clone(),
            CompletionItemKind::VARIABLE,
            "Sequence".to_string(),
        );
    }
    for (name, binding) in &symbols.bindings {
        insert(
            name.clone(),
            CompletionItemKind::VARIABLE,
            describe_type(&binding.annotation),
        );
    }
    for (name, declaration) in &symbols.enums {
        insert(
            name.clone(),
            CompletionItemKind::ENUM,
            "NucleScript enum".to_string(),
        );
        for variant in &declaration.variants {
            insert(
                format!("{}::{}", name, variant.name),
                CompletionItemKind::ENUM_MEMBER,
                variant
                    .payload
                    .as_ref()
                    .map(describe_type)
                    .unwrap_or_else(|| name.clone()),
            );
        }
    }
    items.into_values().collect()
}

fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::KEYWORD,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::TYPE,
            SemanticTokenType::ENUM_MEMBER,
            SemanticTokenType::NUMBER,
            SemanticTokenType::STRING,
            SemanticTokenType::COMMENT,
            SemanticTokenType::OPERATOR,
        ],
        token_modifiers: Vec::new(),
    }
}

fn semantic_tokens(text: &str, symbols: &SymbolTable) -> Vec<SemanticToken> {
    let Ok(tokens) = Lexer::new(text).tokenize() else {
        return Vec::new();
    };
    let mut absolute = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let token_type = match &token.kind {
            TokenKind::Ident(name)
                if BUILTIN_FUNCTIONS.iter().any(|function| function == name)
                    || symbols.functions.contains_key(name) =>
            {
                TOKEN_FUNCTION
            }
            TokenKind::Ident(name)
                if BUILTIN_TYPES.iter().any(|ty| ty == name)
                    || symbols.enums.contains_key(name) =>
            {
                TOKEN_TYPE
            }
            TokenKind::Ident(name)
                if KEYWORDS
                    .iter()
                    .any(|keyword| keyword.eq_ignore_ascii_case(name)) =>
            {
                TOKEN_KEYWORD
            }
            TokenKind::Ident(_)
                if index > 0 && matches!(tokens[index - 1].kind, TokenKind::ColonColon) =>
            {
                TOKEN_ENUM_MEMBER
            }
            TokenKind::Ident(_) => TOKEN_VARIABLE,
            TokenKind::Number(_) => TOKEN_NUMBER,
            TokenKind::String(_) => TOKEN_STRING,
            TokenKind::DocComment(_) => TOKEN_COMMENT,
            TokenKind::Eq
            | TokenKind::Gt
            | TokenKind::Lt
            | TokenKind::Arrow
            | TokenKind::FatArrow
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Le
            | TokenKind::Ge
            | TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::Bang
            | TokenKind::Question
            | TokenKind::ColonColon => TOKEN_OPERATOR,
            _ => continue,
        };
        let range = token_range(text, token);
        let length = range.end.character.saturating_sub(range.start.character);
        if length > 0 && range.start.line == range.end.line {
            absolute.push((range.start.line, range.start.character, length, token_type));
        }
    }

    let mut previous_line = 0;
    let mut previous_start = 0;
    absolute
        .into_iter()
        .map(|(line, start, length, token_type)| {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            previous_line = line;
            previous_start = start;
            SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset: 0,
            }
        })
        .collect()
}

fn token_range(text: &str, token: &Token) -> Range {
    let line_index = token.line.saturating_sub(1);
    let line = text.lines().nth(line_index).unwrap_or_default();
    let start_char_index = token.column.saturating_sub(1);
    let start_utf16 = line
        .chars()
        .take(start_char_index)
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    let lexeme = line.chars().skip(start_char_index).collect::<String>();
    let length = token_length_utf16(&token.kind, &lexeme);
    Range {
        start: Position {
            line: line_index as u32,
            character: start_utf16,
        },
        end: Position {
            line: line_index as u32,
            character: start_utf16 + length,
        },
    }
}

fn token_length_utf16(kind: &TokenKind, source_from_token: &str) -> u32 {
    let fixed = match kind {
        TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::Colon
        | TokenKind::Comma
        | TokenKind::Eq
        | TokenKind::Gt
        | TokenKind::Lt
        | TokenKind::Bang
        | TokenKind::Question => Some(1),
        TokenKind::ColonColon
        | TokenKind::Arrow
        | TokenKind::FatArrow
        | TokenKind::EqEq
        | TokenKind::NotEq
        | TokenKind::Le
        | TokenKind::Ge
        | TokenKind::AndAnd
        | TokenKind::OrOr => Some(2),
        _ => None,
    };
    if let Some(length) = fixed {
        return length;
    }
    match kind {
        TokenKind::Ident(value) | TokenKind::Number(value) => value.encode_utf16().count() as u32,
        TokenKind::DocComment(_) => source_from_token.encode_utf16().count() as u32,
        TokenKind::String(_) => {
            let mut escaped = false;
            let mut length = 0;
            for character in source_from_token.chars() {
                length += character.len_utf16() as u32;
                if character == '"' && length > 1 && !escaped {
                    break;
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
            }
            length
        }
        TokenKind::Eof => 0,
        _ => 1,
    }
}

fn declaration_identifier_range(text: &str, name: &str) -> Option<Range> {
    let tokens = Lexer::new(text).tokenize().ok()?;
    let mut brace_depth = 0usize;
    for pair in tokens.windows(2) {
        match pair[0].kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        if brace_depth != 0 {
            continue;
        }
        let TokenKind::Ident(keyword) = &pair[0].kind else {
            continue;
        };
        let TokenKind::Ident(candidate) = &pair[1].kind else {
            continue;
        };
        if matches!(
            keyword.as_str(),
            "pool" | "strand" | "seq" | "let" | "fn" | "enum"
        ) && candidate == name
        {
            return Some(token_range(text, &pair[1]));
        }
    }
    None
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` has no replacement field yet in lsp-types
fn document_symbols(symbols: &SymbolTable) -> Vec<DocumentSymbol> {
    let mut result = Vec::new();

    let make = |name: &str, detail: String, kind: SymbolKind, span: Span| DocumentSymbol {
        name: name.to_string(),
        detail: Some(detail),
        kind,
        tags: None,
        deprecated: None,
        range: span_to_range(span),
        selection_range: span_to_range(span),
        children: None,
    };

    for (name, pool) in &symbols.pools {
        result.push(make(
            name,
            format!("DnaPool<{}>", pool.profile),
            SymbolKind::VARIABLE,
            pool.span,
        ));
    }
    for (name, func) in &symbols.functions {
        result.push(make(
            name,
            describe_type(&func.return_type),
            SymbolKind::FUNCTION,
            func.span,
        ));
    }
    for (name, span) in &symbols.strands {
        result.push(make(name, "Strand".to_string(), SymbolKind::STRING, *span));
    }
    for (name, span) in &symbols.sequences {
        result.push(make(
            name,
            "Sequence".to_string(),
            SymbolKind::STRING,
            *span,
        ));
    }
    for (name, decl) in &symbols.enums {
        let variants = decl
            .variants
            .iter()
            .map(|v| v.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        result.push(make(name, variants, SymbolKind::ENUM, decl.span));
    }
    for (name, binding) in &symbols.bindings {
        result.push(make(
            name,
            describe_type(&binding.annotation),
            SymbolKind::VARIABLE,
            binding.span,
        ));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn finds_word_when_cursor_is_inside_it() {
        let text = "store plan into target";
        assert_eq!(word_at_position(text, pos(0, 8)), Some("plan".to_string()));
    }

    #[test]
    fn finds_word_when_cursor_is_at_its_start() {
        let text = "store plan into target";
        assert_eq!(word_at_position(text, pos(0, 6)), Some("plan".to_string()));
    }

    #[test]
    fn finds_word_when_cursor_is_immediately_after_it() {
        let text = "store plan into target";
        // Column 10 is the gap right after "plan" (p=6,l=7,a=8,n=9) and
        // before the following space -- LSP's "cursor just typed this
        // word and stopped" position.
        assert_eq!(word_at_position(text, pos(0, 10)), Some("plan".to_string()));
    }

    #[test]
    fn returns_none_between_two_words() {
        let text = "store  plan";
        // Two spaces between "store" and "plan"; column 6 sits in the
        // middle of the gap, with a non-word char on both immediate
        // sides, so there's genuinely no word under the cursor (unlike
        // column 5 or 7, which are each adjacent to one of the words).
        assert_eq!(word_at_position(text, pos(0, 6)), None);
    }

    #[test]
    fn returns_none_for_a_line_beyond_the_document() {
        let text = "store plan";
        assert_eq!(word_at_position(text, pos(5, 0)), None);
    }

    #[test]
    fn hover_describes_a_declared_pool() {
        let (_, symbols) = analysis_for(
            r#"pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }"#,
        );
        let hover =
            hover_text_for("archive", &symbols).expect("expected hover text for a declared pool");
        assert!(
            hover.contains("pool archive"),
            "unexpected hover text: {hover}"
        );
        assert!(hover.contains("Illumina"), "unexpected hover text: {hover}");
    }

    #[test]
    fn hover_returns_none_for_an_unknown_name() {
        let (_, symbols) = analysis_for(
            r#"pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }"#,
        );
        assert_eq!(hover_text_for("not_a_real_symbol", &symbols), None);
    }

    #[test]
    fn definition_span_points_at_the_declaring_pool() {
        let (_, symbols) = analysis_for(
            "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }",
        );
        let span = definition_span_for("archive", &symbols).expect("expected a definition span");
        assert_eq!(span.line, 1);
    }

    #[test]
    fn completion_includes_language_and_program_symbols_in_stable_order() {
        let (_, symbols) = analysis_for(
            r#"pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
fn save(target: File) returns Result<DnaFile, Str> { store target into archive }"#,
        );
        let items = completion_items(&symbols);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"pool"));
        assert!(labels.contains(&"DnaPool"));
        assert!(labels.contains(&"consensus_vote"));
        assert!(labels.contains(&"archive"));
        assert!(labels.contains(&"save"));
        assert_eq!(
            items
                .iter()
                .find(|item| item.label == "consensus_vote")
                .and_then(|item| item.kind),
            Some(CompletionItemKind::FUNCTION)
        );
        assert!(
            labels.windows(2).all(|pair| pair[0] <= pair[1]),
            "completion order was not stable"
        );
    }

    #[test]
    fn identifier_occurrences_ignore_comments_and_strings() {
        let text = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n\
                    // archive is only prose here\n\
                    store \"archive\" into archive";
        let occurrences = identifier_occurrences(text, "archive");
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].start.line, 0);
        assert_eq!(occurrences[1].start.line, 2);
    }

    #[test]
    fn rename_is_refused_when_a_local_parameter_shadows_the_symbol() {
        let text = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n\
                    fn inspect(archive: File) returns File { let copy: File = archive }";
        assert!(has_local_shadow(text, "archive"));
        assert!(!has_local_shadow(text, "inspect"));
    }

    #[test]
    fn semantic_tokens_classify_keywords_functions_types_and_literals() {
        let source = "fn inspect(target: File) returns Str { let count: Str = \"3\" }";
        let (_, symbols) = analysis_for(source);
        let tokens = semantic_tokens(source, &symbols);
        assert!(tokens.iter().any(|token| token.token_type == TOKEN_KEYWORD));
        assert!(tokens
            .iter()
            .any(|token| token.token_type == TOKEN_FUNCTION));
        assert!(tokens.iter().any(|token| token.token_type == TOKEN_TYPE));
        assert!(tokens.iter().any(|token| token.token_type == TOKEN_STRING));
    }

    #[test]
    fn identifier_ranges_use_utf16_columns() {
        let source = "\"🧬\" archive";
        let (_, range) = identifier_at_position(
            source,
            Position {
                line: 0,
                character: 6,
            },
        )
        .unwrap();
        // quote + surrogate pair + quote + space = five UTF-16 code units
        assert_eq!(range.start.character, 5);
        assert_eq!(range.end.character, 12);
    }

    fn analysis_for(source: &str) -> (nucle_lang::CheckReport, SymbolTable) {
        let analysis = nucle_lang::analyze(source);
        (analysis.report, analysis.symbols)
    }
}
