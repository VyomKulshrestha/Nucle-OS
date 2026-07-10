//! LSP protocol adapter over `nucle_lang::analyze`.
//!
//! Deliberately thin: every method here converts between LSP wire types
//! and `nucle_lang`'s own `Diagnostic`/`SymbolTable`/`Span` shapes. It does
//! not re-implement or duplicate any compiler logic -- if `nucle check`
//! and this server ever disagreed about whether a program is valid, that
//! would be a bug in this file's conversion, not a second source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use nucle_lang::ast::{Span, TypeExpr};
use nucle_lang::{analyze, Diagnostic as NucleDiagnostic, DiagnosticLevel, SymbolTable};

pub struct Backend {
    client: Client,
    documents: Arc<Mutex<HashMap<Url, String>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client, documents: Arc::new(Mutex::new(HashMap::new())) }
    }

    async fn on_change(&self, uri: Url, text: String) {
        let analysis = analyze(&text);
        let diagnostics = analysis.report.diagnostics.iter().map(to_lsp_diagnostic).collect();
        self.documents.lock().await.insert(uri.clone(), text);
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
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
        self.on_change(params.text_document.uri, params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full document sync (declared in `initialize`), so the last
        // reported change already contains the entire new text.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.lock().await.remove(&params.text_document.uri);
        self.client.publish_diagnostics(params.text_document.uri, vec![], None).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(uri) else { return Ok(None) };
        let Some(word) = word_at_position(text, position) else { return Ok(None) };
        let analysis = analyze(text);
        Ok(hover_text_for(&word, &analysis.symbols)
            .map(|contents| Hover { contents: HoverContents::Scalar(MarkedString::String(contents)), range: None }))
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else { return Ok(None) };
        let Some(word) = word_at_position(text, position) else { return Ok(None) };
        let analysis = analyze(text);
        Ok(definition_span_for(&word, &analysis.symbols)
            .map(|span| GotoDefinitionResponse::Scalar(Location { uri, range: span_to_range(span) })))
    }

    async fn document_symbol(&self, params: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let documents = self.documents.lock().await;
        let Some(text) = documents.get(&uri) else { return Ok(None) };
        let analysis = analyze(text);
        Ok(Some(DocumentSymbolResponse::Nested(document_symbols(&analysis.symbols))))
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
/// line/UTF-16-character; NucleScript identifiers are ASCII, so treating
/// `character` as a plain char index is exact). Expands in both
/// directions from the cursor so it works whether the cursor sits inside,
/// at the start of, or immediately after the word.
fn word_at_position(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let col = (position.character as usize).min(chars.len());
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
    Some(chars[start..end].iter().collect())
}

fn hover_text_for(name: &str, symbols: &SymbolTable) -> Option<String> {
    if let Some(pool) = symbols.pools.get(name) {
        return Some(format!(
            "```nuclescript\npool {}: DnaPool {{ codec: {}, redundancy: {}x, profile: {} }}\n```",
            name, pool.codec, pool.redundancy, pool.profile
        ));
    }
    if let Some(func) = symbols.functions.get(name) {
        let params: Vec<String> = func.params.iter().map(|p| format!("{}: {}", p.name, describe_type(&p.ty))).collect();
        let type_params = if func.type_params.is_empty() { String::new() } else { format!("<{}>", func.type_params.join(", ")) };
        return Some(format!(
            "```nuclescript\nfn {}{}({}) -> {}\n```",
            name, type_params, params.join(", "), describe_type(&func.return_type)
        ));
    }
    if symbols.strands.contains_key(name) {
        return Some(format!("```nuclescript\nstrand {}: Strand\n```", name));
    }
    if symbols.sequences.contains_key(name) {
        return Some(format!("```nuclescript\nseq {}: Sequence\n```", name));
    }
    if let Some(binding) = symbols.bindings.get(name) {
        return Some(format!("```nuclescript\nlet {}: {}\n```", name, describe_type(&binding.annotation)));
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
        TypeExpr::Result(ok, err) => format!("Result<{}, {}>", describe_type(ok), describe_type(err)),
        TypeExpr::Str => "Str".to_string(),
        TypeExpr::Fn(params, ret) => format!(
            "Fn({}) -> {}",
            params.iter().map(describe_type).collect::<Vec<_>>().join(", "),
            describe_type(ret)
        ),
    }
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
        result.push(make(name, format!("DnaPool<{}>", pool.profile), SymbolKind::VARIABLE, pool.span));
    }
    for (name, func) in &symbols.functions {
        result.push(make(name, describe_type(&func.return_type), SymbolKind::FUNCTION, func.span));
    }
    for (name, span) in &symbols.strands {
        result.push(make(name, "Strand".to_string(), SymbolKind::STRING, *span));
    }
    for (name, span) in &symbols.sequences {
        result.push(make(name, "Sequence".to_string(), SymbolKind::STRING, *span));
    }
    for (name, binding) in &symbols.bindings {
        result.push(make(name, describe_type(&binding.annotation), SymbolKind::VARIABLE, binding.span));
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
        let hover = hover_text_for("archive", &symbols).expect("expected hover text for a declared pool");
        assert!(hover.contains("pool archive"), "unexpected hover text: {hover}");
        assert!(hover.contains("Illumina"), "unexpected hover text: {hover}");
    }

    #[test]
    fn hover_returns_none_for_an_unknown_name() {
        let (_, symbols) = analysis_for(r#"pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }"#);
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

    fn analysis_for(source: &str) -> (nucle_lang::CheckReport, SymbolTable) {
        let analysis = nucle_lang::analyze(source);
        (analysis.report, analysis.symbols)
    }
}
