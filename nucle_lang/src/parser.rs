//! Recursive-descent parser for NucleScript.

use crate::ast::*;
use crate::lexer::{Token, TokenKind};
use std::fmt;

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut declarations = Vec::new();
        while !self.is_eof() {
            if self.consume_comma() {
                continue;
            }
            declarations.push(self.parse_declaration()?);
        }
        Ok(Program { declarations })
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        if self.check_ident("import") {
            self.parse_import().map(Declaration::Import)
        } else if self.check_ident("pool") {
            self.parse_pool().map(Declaration::Pool)
        } else if self.check_ident("strand") {
            self.parse_strand().map(Declaration::Strand)
        } else if self.check_ident("seq") {
            self.parse_sequence_decl().map(Declaration::Sequence)
        } else if self.check_ident("let") {
            self.parse_let_decl()
        } else if self.check_ident("fn") {
            self.parse_function_decl().map(Declaration::Function)
        } else if self.check_ident("store") || self.check_ident("simulate") {
            self.parse_store().map(|op| Declaration::Operation(Operation::Store(op)))
        } else if self.check_ident("retrieve") {
            self.parse_retrieve().map(|op| Declaration::Operation(Operation::Retrieve(op)))
        } else if self.check_ident("delete") {
            self.parse_delete().map(|op| Declaration::Operation(Operation::Delete(op)))
        } else if self.check_ident("pipeline") {
            self.parse_pipeline().map(Declaration::Pipeline)
        } else {
            Err(self.error_here("expected declaration: import, pool, strand, seq, let, fn, store, retrieve, delete, simulate, or pipeline"))
        }
    }

    fn parse_function_decl(&mut self) -> Result<FunctionDecl, ParseError> {
        self.expect_ident_text("fn")?;
        let name = self.expect_ident_any("function name")?;
        self.expect(TokenKind::LParen, "'(' after function name")?;
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            let param_name = self.expect_ident_any("parameter name")?;
            self.expect(TokenKind::Colon, "':' after parameter name")?;
            let ty = self.parse_type_expr()?;
            params.push(FnParam { name: param_name, ty });
            if !self.consume_comma() && !self.check(TokenKind::RParen) {
                return Err(self.error_here("expected ',' or ')' in parameter list"));
            }
        }
        self.expect(TokenKind::RParen, "')' after parameter list")?;

        // Return type must be given explicitly with `->` or `returns` —
        // a function that truly has no return value still writes
        // `returns Void` (see docs/examples/failures/), rather than
        // silently defaulting, so a return-type typo can't compile.
        let return_type = if self.check(TokenKind::Arrow) {
            self.advance();
            self.parse_type_expr()?
        } else if self.check_ident("returns") {
            self.advance();
            self.parse_type_expr()?
        } else {
            return Err(self.error_here(format!(
                "expected '->' or 'returns' followed by a return type after parameters of function '{}'",
                name
            )));
        };

        self.expect(TokenKind::LBrace, "'{' to start function body")?;
        let mut body = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.consume_comma() {
                continue;
            }
            body.push(self.parse_declaration()?);
        }
        self.expect(TokenKind::RBrace, "'}' to end function body")?;

        Ok(FunctionDecl {
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_import(&mut self) -> Result<ImportDecl, ParseError> {
        self.expect_ident_text("import")?;
        self.expect(TokenKind::LBrace, "'{' after import")?;
        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) {
            let name = self.expect_ident_any("import item")?;
            let alias = if self.check_ident("as") {
                self.advance();
                Some(self.expect_ident_any("import alias")?)
            } else {
                None
            };
            items.push(ImportItem { name, alias });
            if !self.consume_comma() && !self.check(TokenKind::RBrace) {
                return Err(self.error_here("expected ',' or '}' in import list"));
            }
        }
        self.expect(TokenKind::RBrace, "'}' after import list")?;
        self.expect_ident_text("from")?;
        let source = self.expect_string("import source")?;
        Ok(ImportDecl { source, items })
    }

    fn parse_pool(&mut self) -> Result<PoolDecl, ParseError> {
        self.expect_ident_text("pool")?;
        let name = self.expect_ident_any("pool name")?;
        self.expect(TokenKind::Colon, "':' after pool name")?;
        self.expect_ident_text("DnaPool")?;
        self.expect(TokenKind::LBrace, "'{' to start pool schema")?;

        let mut codec = None;
        let mut redundancy = None;
        let mut profile = None;

        while !self.check(TokenKind::RBrace) {
            let key = self.expect_ident_any("pool property")?;
            self.expect(TokenKind::Colon, "':' after pool property")?;
            match key.to_ascii_lowercase().as_str() {
                "codec" => {
                    let ident = self.expect_ident_any("codec")?;
                    codec = Some(Codec::parse(&ident).ok_or_else(|| self.error_previous(format!("unknown codec '{}'", ident)))?);
                }
                "redundancy" => redundancy = Some(self.expect_multiplier("redundancy multiplier")?),
                "profile" => {
                    let ident = self.expect_ident_any("profile")?;
                    profile = Some(Profile::parse(&ident).ok_or_else(|| self.error_previous(format!("unknown profile '{}'", ident)))?);
                }
                other => return Err(self.error_previous(format!("unknown pool property '{}'", other))),
            }
            self.consume_comma();
        }
        self.expect(TokenKind::RBrace, "'}' after pool schema")?;

        Ok(PoolDecl {
            name,
            codec: codec.ok_or_else(|| self.error_here("pool missing codec"))?,
            redundancy: redundancy.unwrap_or(1),
            profile: profile.ok_or_else(|| self.error_here("pool missing profile"))?,
        })
    }

    fn parse_strand(&mut self) -> Result<StrandDecl, ParseError> {
        self.expect_ident_text("strand")?;
        let name = self.expect_ident_any("strand name")?;
        self.expect(TokenKind::Colon, "':' after strand name")?;
        self.expect_ident_text("Strand")?;
        self.expect(TokenKind::Eq, "'=' before strand literal")?;
        let sequence = self.expect_string("strand sequence")?;
        Ok(StrandDecl { name, sequence })
    }

    fn parse_sequence_decl(&mut self) -> Result<SequenceDecl, ParseError> {
        self.expect_ident_text("seq")?;
        let name = self.expect_ident_any("sequence name")?;
        self.expect(TokenKind::Colon, "':' after sequence name")?;
        self.expect_ident_text("Sequence")?;
        self.expect(TokenKind::Eq, "'=' before sequence literal")?;
        let sequence = self.expect_string("sequence literal")?;
        Ok(SequenceDecl { name, sequence })
    }

    fn parse_let_decl(&mut self) -> Result<Declaration, ParseError> {
        self.expect_ident_text("let")?;
        let name = self.expect_ident_any("binding name")?;
        if self.check(TokenKind::Colon) {
            self.advance();
            if self.check_ident("Sequence") {
                self.advance();
                self.expect(TokenKind::Eq, "'=' before binding expression")?;
                self.expect_ident_text("seq")?;
                let sequence = self.expect_string("sequence literal")?;
                return Ok(Declaration::Sequence(SequenceDecl { name, sequence }));
            }
            let annotation = self.parse_type_expr()?;
            self.expect(TokenKind::Eq, "'=' before binding expression")?;
            let expr = self.parse_expr()?;
            return Ok(Declaration::Let(LetDecl { name, annotation, expr }));
        }
        self.expect(TokenKind::Eq, "'=' before binding expression")?;
        self.expect_ident_text("seq")?;
        let sequence = self.expect_string("sequence literal")?;
        Ok(Declaration::Sequence(SequenceDecl { name, sequence }))
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        if self.check_ident("Pool") {
            self.advance();
            self.expect(TokenKind::Lt, "'<' after Pool")?;
            let state_name = self.expect_ident_any("pool profile or state")?;
            let state = PoolState::parse(&state_name)
                .ok_or_else(|| self.error_previous(format!("unknown pool profile or state '{}'", state_name)))?;
            let error_rate_percent = if self.consume_comma() {
                Some(self.expect_percent("pool error rate")?)
            } else {
                None
            };
            self.expect(TokenKind::Gt, "'>' after Pool type")?;
            Ok(TypeExpr::Pool(PoolType { state, error_rate_percent }))
        } else if self.check_ident("Strand") {
            self.advance();
            Ok(TypeExpr::Strand)
        } else if self.check_ident("Sequence") {
            self.advance();
            Ok(TypeExpr::Sequence)
        } else if self.check_ident("File") {
            self.advance();
            Ok(TypeExpr::File)
        } else if self.check_ident("DnaFile") {
            self.advance();
            Ok(TypeExpr::DnaFile)
        } else if self.check_ident("Recovery") {
            self.advance();
            Ok(TypeExpr::Recovery)
        } else if self.check_ident("Void") {
            self.advance();
            Ok(TypeExpr::Void)
        } else {
            Err(self.error_here("expected type annotation: Pool<...>, Strand, Sequence, File, DnaFile, Recovery, or Void"))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        if self.check_ident("simulate") {
            self.advance();
            let pool = self.expect_ident_any("pool name")?;
            self.expect_ident_text("under")?;
            let profile_name = self.expect_ident_any("profile")?;
            let profile = Profile::parse(&profile_name)
                .ok_or_else(|| self.error_previous(format!("unknown profile '{}'", profile_name)))?;
            Ok(Expr::SimulatePool { pool, profile })
        } else if self.check_ident("synthesise") || self.check_ident("synthesize") {
            self.advance();
            let source = self.expect_ident_any("source pool binding")?;
            self.expect_ident_text("via")?;
            let profile_name = self.expect_ident_any("profile")?;
            let profile = Profile::parse(&profile_name)
                .ok_or_else(|| self.error_previous(format!("unknown profile '{}'", profile_name)))?;
            let confirmed = self.consume_confirmation("hardware")?;
            Ok(Expr::SynthesizePool { source, profile, confirmed })
        } else if self.check_ident("sequence") {
            self.advance();
            let source = self.expect_ident_any("source pool binding")?;
            self.expect_ident_text("via")?;
            let profile_name = self.expect_ident_any("profile")?;
            let profile = Profile::parse(&profile_name)
                .ok_or_else(|| self.error_previous(format!("unknown profile '{}'", profile_name)))?;
            let confirmed = self.consume_confirmation("hardware")?;
            Ok(Expr::SequencePool { source, profile, confirmed })
        } else if self.check_ident("consensus_vote") {
            self.advance();
            self.expect(TokenKind::LParen, "'(' after consensus_vote")?;
            let source = self.expect_ident_any("source pool binding")?;
            self.expect(TokenKind::Comma, "',' after source pool binding")?;
            self.expect_ident_text("coverage")?;
            self.expect(TokenKind::Colon, "':' after coverage")?;
            let coverage = self.expect_multiplier("coverage multiplier")?;
            self.expect(TokenKind::RParen, "')' after consensus_vote")?;
            Ok(Expr::ConsensusVote { source, coverage })
        } else if self.check_ident("protect") {
            self.advance();
            let data = self.expect_ident_any("data name")?;
            self.expect_ident_text("for")?;
            let guarantee = self.expect_ident_any("guarantee name")?;
            Ok(Expr::Protect { data, guarantee })
        } else if let TokenKind::Ident(name) = &self.peek().kind {
            let name = name.clone();
            if self.tokens.get(self.index + 1).map(|t| &t.kind) == Some(&TokenKind::LParen) {
                self.advance(); // consume ident
                self.expect(TokenKind::LParen, "'(' after function name")?;
                let mut args = Vec::new();
                while !self.check(TokenKind::RParen) {
                    args.push(self.parse_expr()?);
                    if !self.consume_comma() && !self.check(TokenKind::RParen) {
                        return Err(self.error_here("expected ',' or ')' in argument list"));
                    }
                }
                self.expect(TokenKind::RParen, "')' after function arguments")?;
                Ok(Expr::FunctionCall { name, args })
            } else {
                self.advance(); // consume ident
                Ok(Expr::Variable(name))
            }
        } else if self.check(TokenKind::String(String::new())) {
            let val = self.expect_string("string literal")?;
            Ok(Expr::StringLiteral(val))
        } else {
            Err(self.error_here("expected expression"))
        }
    }

    fn consume_confirmation(&mut self, marker: &str) -> Result<bool, ParseError> {
        if !self.check_ident("confirm") {
            return Ok(false);
        }
        self.advance();
        self.expect_ident_text(marker)?;
        Ok(true)
    }

    fn parse_store(&mut self) -> Result<StoreOp, ParseError> {
        let simulate = if self.check_ident("simulate") {
            self.advance();
            self.expect_ident_text("store")?;
            true
        } else {
            self.expect_ident_text("store")?;
            false
        };
        let file = if self.check(TokenKind::String(String::new())) {
            self.expect_string("file path")?
        } else {
            self.expect_ident_any("file variable")?
        };
        self.expect_ident_text("into")?;
        let pool = self.expect_ident_any("pool name")?;
        let options = if self.check(TokenKind::LBrace) {
            self.parse_store_options()?
        } else {
            StoreOptions::default()
        };
        Ok(StoreOp { simulate, file, pool, options })
    }

    fn parse_store_options(&mut self) -> Result<StoreOptions, ParseError> {
        self.expect(TokenKind::LBrace, "'{' to start store options")?;
        let mut options = StoreOptions::default();
        while !self.check(TokenKind::RBrace) {
            if self.check_ident("expect") {
                self.advance();
                self.expect_ident_text("recovery")?;
                self.expect(TokenKind::Gt, "'>' after recovery")?;
                options.expect_recovery_gt = Some(self.expect_percent("recovery percentage")?);
            } else {
                let key = self.expect_ident_any("store option")?;
                self.expect(TokenKind::Colon, "':' after store option")?;
                match key.to_ascii_lowercase().as_str() {
                    "redundancy" => options.redundancy = Some(self.expect_multiplier("redundancy multiplier")?),
                    "coverage" => options.coverage = Some(self.expect_multiplier("coverage multiplier")?),
                    "tag" | "tags" => options.tags = self.parse_string_list()?,
                    other => return Err(self.error_previous(format!("unknown store option '{}'", other))),
                }
            }
            self.consume_comma();
        }
        self.expect(TokenKind::RBrace, "'}' after store options")?;
        Ok(options)
    }

    fn parse_retrieve(&mut self) -> Result<RetrieveOp, ParseError> {
        self.expect_ident_text("retrieve")?;
        self.expect_ident_text("from")?;
        let pool = self.expect_ident_any("pool name")?;
        let mut query = Vec::new();
        if self.check_ident("where") {
            self.advance();
            self.expect(TokenKind::LBrace, "'{' to start query")?;
            while !self.check(TokenKind::RBrace) {
                query.push(self.parse_query_predicate()?);
                self.consume_comma();
            }
            self.expect(TokenKind::RBrace, "'}' after query")?;
        }
        Ok(RetrieveOp { pool, query })
    }

    fn parse_delete(&mut self) -> Result<DeleteOp, ParseError> {
        self.expect_ident_text("delete")?;
        let file = if self.check(TokenKind::String(String::new())) {
            self.expect_string("file path")?
        } else {
            self.expect_ident_any("file variable")?
        };
        self.expect_ident_text("from")?;
        let pool = self.expect_ident_any("pool name")?;
        let confirmed = self.consume_confirmation("physical_key")?;
        Ok(DeleteOp { file, pool, confirmed })
    }

    fn parse_query_predicate(&mut self) -> Result<QueryPredicate, ParseError> {
        let field = self.expect_ident_any("query field")?;
        let op = if self.check_ident("contains") {
            self.advance();
            QueryOp::Contains
        } else if self.check(TokenKind::Gt) {
            self.advance();
            QueryOp::Gt
        } else if self.check(TokenKind::Lt) {
            self.advance();
            QueryOp::Lt
        } else if self.check(TokenKind::Eq) {
            self.advance();
            QueryOp::Eq
        } else {
            return Err(self.error_here("expected query operator: contains, >, <, or ="));
        };
        let value = self.parse_query_value()?;
        Ok(QueryPredicate { field, op, value })
    }

    fn parse_query_value(&mut self) -> Result<QueryValue, ParseError> {
        match self.advance().kind.clone() {
            TokenKind::String(value) => Ok(QueryValue::String(value)),
            TokenKind::Ident(value) => Ok(QueryValue::Ident(value)),
            TokenKind::Number(raw) => {
                if raw.contains('-') {
                    Ok(QueryValue::Date(raw))
                } else if raw.to_ascii_lowercase().ends_with("mb") {
                    let n = raw[..raw.len() - 2].parse::<u64>().map_err(|_| self.error_previous("invalid MB size literal"))?;
                    Ok(QueryValue::SizeBytes(n * 1024 * 1024))
                } else if raw.to_ascii_lowercase().ends_with("kb") {
                    let n = raw[..raw.len() - 2].parse::<u64>().map_err(|_| self.error_previous("invalid KB size literal"))?;
                    Ok(QueryValue::SizeBytes(n * 1024))
                } else {
                    Ok(QueryValue::Number(raw.parse::<f64>().map_err(|_| self.error_previous("invalid number literal"))?))
                }
            }
            _ => Err(self.error_previous("expected query value")),
        }
    }

    fn parse_pipeline(&mut self) -> Result<PipelineDecl, ParseError> {
        self.expect_ident_text("pipeline")?;
        let name = self.expect_ident_any("pipeline name")?;
        self.expect(TokenKind::LBrace, "'{' to start pipeline")?;
        let mut steps = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.check_ident("encode") {
                self.advance();
                let path = self.expect_string("path to encode")?;
                self.expect_ident_text("using")?;
                let codec_name = self.expect_ident_any("codec")?;
                let codec = Codec::parse(&codec_name).ok_or_else(|| self.error_previous(format!("unknown codec '{}'", codec_name)))?;
                steps.push(PipelineStep::Encode { path, codec });
            } else if self.check_ident("protect") {
                self.advance();
                self.expect_ident_text("with")?;
                self.expect_ident_text("redundancy")?;
                let redundancy = self.expect_multiplier("redundancy multiplier")?;
                steps.push(PipelineStep::Protect { redundancy });
            } else if self.check_ident("store") {
                self.advance();
                self.expect_ident_text("into")?;
                let pool = self.expect_ident_any("pool name")?;
                steps.push(PipelineStep::Store { pool });
            } else if self.check_ident("verify") {
                self.advance();
                self.expect_ident_text("roundtrip")?;
                steps.push(PipelineStep::VerifyRoundtrip);
            } else {
                return Err(self.error_here("expected pipeline step"));
            }
            self.consume_comma();
        }
        self.expect(TokenKind::RBrace, "'}' after pipeline")?;
        Ok(PipelineDecl { name, steps })
    }

    fn parse_string_list(&mut self) -> Result<Vec<String>, ParseError> {
        if self.check(TokenKind::String(String::new())) {
            return Ok(vec![self.expect_string("tag")?]);
        }
        self.expect(TokenKind::LBracket, "'[' to start string list")?;
        let mut values = Vec::new();
        while !self.check(TokenKind::RBracket) {
            values.push(self.expect_string("string list item")?);
            if !self.consume_comma() && !self.check(TokenKind::RBracket) {
                return Err(self.error_here("expected ',' or ']' in string list"));
            }
        }
        self.expect(TokenKind::RBracket, "']' after string list")?;
        Ok(values)
    }

    fn expect_multiplier(&mut self, what: &str) -> Result<usize, ParseError> {
        let raw = self.expect_number(what)?;
        let trimmed = raw.trim_end_matches('x').trim_end_matches('X');
        let value = trimmed.parse::<usize>().map_err(|_| self.error_previous(format!("invalid {} '{}'", what, raw)))?;
        if value == 0 {
            return Err(self.error_previous(format!("{} must be at least 1", what)));
        }
        Ok(value)
    }

    fn expect_percent(&mut self, what: &str) -> Result<f64, ParseError> {
        let raw = self.expect_number(what)?;
        let trimmed = raw.trim_end_matches('%');
        trimmed.parse::<f64>().map_err(|_| self.error_previous(format!("invalid {} '{}'", what, raw)))
    }

    fn expect_number(&mut self, what: &str) -> Result<String, ParseError> {
        match self.advance().kind.clone() {
            TokenKind::Number(value) => Ok(value),
            _ => Err(self.error_previous(format!("expected {}", what))),
        }
    }

    fn expect_string(&mut self, what: &str) -> Result<String, ParseError> {
        match self.advance().kind.clone() {
            TokenKind::String(value) => Ok(value),
            _ => Err(self.error_previous(format!("expected {} string", what))),
        }
    }

    fn expect_ident_text(&mut self, expected: &str) -> Result<(), ParseError> {
        let token = self.advance().clone();
        match &token.kind {
            TokenKind::Ident(actual) if actual.eq_ignore_ascii_case(expected) => Ok(()),
            _ => Err(ParseError {
                line: token.line,
                column: token.column,
                message: format!("expected '{}'", expected),
            }),
        }
    }

    fn expect_ident_any(&mut self, what: &str) -> Result<String, ParseError> {
        match self.advance().kind.clone() {
            TokenKind::Ident(value) => Ok(value),
            _ => Err(self.error_previous(format!("expected {}", what))),
        }
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> Result<(), ParseError> {
        if self.check(kind.clone()) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here(format!("expected {}", what)))
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(&kind)
    }

    fn check_ident(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(actual) if actual.eq_ignore_ascii_case(expected))
    }

    fn consume_comma(&mut self) -> bool {
        if self.check(TokenKind::Comma) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_eof() {
            self.index += 1;
        }
        &self.tokens[self.index - 1]
    }

    fn error_here(&self, message: impl Into<String>) -> ParseError {
        ParseError { line: self.peek().line, column: self.peek().column, message: message.into() }
    }

    fn error_previous(&self, message: impl Into<String>) -> ParseError {
        let token = &self.tokens[self.index.saturating_sub(1)];
        ParseError { line: token.line, column: token.column, message: message.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}:{}", self.message, self.line, self.column)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Program {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn parses_report_program() {
        let src = r#"
            pool archive: DnaPool { codec: YinYang, redundancy: 3x, profile: Illumina }
            store "genome.fasta" into archive { redundancy: 4x, tag: ["medical", "genomics", "2026"] }
            retrieve from archive where { tag contains "medical", date > 2025-01-01, size < 10MB }
            simulate store "archive.tar" into archive { coverage: 10x, expect recovery > 99.5% }
            pipeline backup { encode "records/" using YinYang, protect with redundancy 3x, store into archive, verify roundtrip }
        "#;
        let program = parse(src);
        assert_eq!(program.declarations.len(), 5);
    }

    #[test]
    fn parses_sequence_literals() {
        let src = r#"
            seq primer_p0: Sequence = "ATCGATCGGCTAGCTA"
            let primer_p1 = seq"ATCGATCG-GCTAGCTA"
            let primer_p2: Sequence = seq"GCTAGCTA-ATCGATCG"
        "#;
        let program = parse(src);
        assert_eq!(program.declarations.len(), 3);
        assert!(matches!(program.declarations[0], Declaration::Sequence(_)));
    }

    #[test]
    fn parses_probabilistic_pool_bindings() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }
            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
        "#;
        let program = parse(src);
        assert_eq!(program.declarations.len(), 3);
        assert!(matches!(program.declarations[1], Declaration::Let(_)));
        assert!(matches!(program.declarations[2], Declaration::Let(_)));
    }

    #[test]
    fn parses_effectful_operations() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }
            let strands: Pool<Twist, 0.03%> = synthesise archive via Twist confirm hardware
            delete "old.bin" from archive confirm physical_key
        "#;
        let program = parse(src);
        assert_eq!(program.declarations.len(), 3);
        assert!(matches!(program.declarations[1], Declaration::Let(_)));
        assert!(matches!(program.declarations[2], Declaration::Operation(Operation::Delete(_))));
    }

    #[test]
    fn parses_package_imports() {
        let src = r#"import { medical_archive, reliable_store as store_recipe } from "nuclescript/presets""#;
        let program = parse(src);
        assert_eq!(program.declarations.len(), 1);
        let Declaration::Import(import) = &program.declarations[0] else {
            panic!("expected import declaration");
        };
        assert_eq!(import.items.len(), 2);
        assert_eq!(import.items[1].alias.as_deref(), Some("store_recipe"));
    }
}
