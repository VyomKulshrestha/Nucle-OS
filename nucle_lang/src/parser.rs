//! Recursive-descent parser for NucleScript.

use crate::ast::*;
use crate::lexer::{Token, TokenKind};
use std::fmt;

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
    /// The enclosing function's type parameter names, active only while
    /// parsing that one function's params/return type/body (set in
    /// `parse_function_decl` right after `<...>`, cleared when it
    /// returns). Consulted by `parse_type_expr`'s `Pool<...>` branch to
    /// tell a type parameter (`Pool<T>`) apart from a concrete profile
    /// name (`Pool<Illumina>`). Functions can't nest in this grammar, so
    /// a single field is enough -- no save/restore stack needed.
    type_params_in_scope: Vec<String>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0, type_params_in_scope: Vec::new() }
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
        // `///` doc comments immediately preceding a declaration attach to
        // it (see `ast::PoolDecl::doc`'s doc comment) -- only `pool`/
        // `strand`/`seq`/`fn`/`pipeline` have somewhere to put one. Every
        // other declaration kind rejects a leading doc comment outright
        // (`reject_doc_comment` below) rather than silently discarding
        // it, so a `///` in the wrong place is a clear mistake to fix,
        // not documentation that quietly went nowhere.
        let doc = self.consume_doc_comment();
        if self.check_ident("import") {
            self.reject_doc_comment(&doc, "import")?;
            self.parse_import().map(Declaration::Import)
        } else if self.check_ident("pool") {
            self.parse_pool(doc).map(Declaration::Pool)
        } else if self.check_ident("strand") {
            self.parse_strand(doc).map(Declaration::Strand)
        } else if self.check_ident("seq") {
            self.parse_sequence_decl(doc).map(Declaration::Sequence)
        } else if self.check_ident("let") {
            self.parse_let_decl(doc)
        } else if self.check_ident("fn") {
            self.parse_function_decl(doc).map(Declaration::Function)
        } else if self.check_ident("enum") {
            self.parse_enum_decl(doc).map(Declaration::Enum)
        } else if self.check_ident("store") || self.check_ident("simulate") {
            self.reject_doc_comment(&doc, "store")?;
            self.parse_store().map(|op| Declaration::Operation(Operation::Store(op)))
        } else if self.check_ident("retrieve") {
            self.reject_doc_comment(&doc, "retrieve")?;
            self.parse_retrieve().map(|op| Declaration::Operation(Operation::Retrieve(op)))
        } else if self.check_ident("delete") {
            self.reject_doc_comment(&doc, "delete")?;
            self.parse_delete().map(|op| Declaration::Operation(Operation::Delete(op)))
        } else if self.check_ident("assert") {
            self.reject_doc_comment(&doc, "assert")?;
            self.parse_assert().map(|op| Declaration::Operation(Operation::Assert(op)))
        } else if self.check_ident("pipeline") {
            self.parse_pipeline(doc).map(Declaration::Pipeline)
        } else if self.check_ident("if") {
            self.reject_doc_comment(&doc, "if")?;
            self.parse_if().map(Declaration::If)
        } else if self.check_ident("for") {
            self.reject_doc_comment(&doc, "for")?;
            self.parse_for().map(Declaration::For)
        } else if self.check_ident("test") {
            self.reject_doc_comment(&doc, "test")?;
            self.parse_test().map(Declaration::Test)
        } else if doc.is_some() {
            Err(self.error_here("expected a documentable declaration (pool, strand, seq, fn, enum, or pipeline) after a doc comment"))
        } else {
            Err(self.error_here("expected declaration: import, pool, strand, seq, let, fn, enum, store, retrieve, delete, assert, simulate, pipeline, if, for, or test"))
        }
    }

    /// Doc comments only attach to `pool`/`strand`/`seq`/`fn`/`enum`/
    /// `pipeline` (see `ast::PoolDecl::doc`) -- a `///` immediately before
    /// anything else is rejected here rather than silently dropped, so it
    /// reads as a mistake to fix, not documentation that quietly went
    /// nowhere. `let` is handled separately (see `parse_let_decl`) since
    /// it can desugar to a documentable `SequenceDecl` depending on which
    /// form is written.
    fn reject_doc_comment(&self, doc: &Option<String>, keyword: &str) -> Result<(), ParseError> {
        if doc.is_some() {
            Err(self.error_here(format!(
                "doc comments can only precede pool/strand/seq/fn/enum/pipeline declarations, not '{}'",
                keyword
            )))
        } else {
            Ok(())
        }
    }

    /// Accumulates every consecutive `///` line into one `\n`-joined doc
    /// string, or `None` if the next token isn't a doc comment at all.
    fn consume_doc_comment(&mut self) -> Option<String> {
        let mut lines = Vec::new();
        while let TokenKind::DocComment(text) = &self.peek().kind {
            lines.push(text.clone());
            self.advance();
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// `assert` Expr (`,` StringLiteral)?
    fn parse_assert(&mut self) -> Result<AssertOp, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("assert")?;
        let condition = self.parse_expr()?;
        let message = if self.consume_comma() { Some(self.expect_string("assertion message")?) } else { None };
        Ok(AssertOp { condition, message, span: self.span_since(start) })
    }

    /// `test` StringLiteral `{` <declaration>* `}`
    fn parse_test(&mut self) -> Result<TestDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("test")?;
        let name = self.expect_string("test description")?;
        self.expect(TokenKind::LBrace, "'{' to start test body")?;
        let body = self.parse_declaration_block()?;
        self.expect(TokenKind::RBrace, "'}' to end test body")?;
        Ok(TestDecl { name, body, span: self.span_since(start) })
    }

    /// `if` <expr> `{` <declaration>* `}` (`else` `{` <declaration>* `}`)?
    fn parse_if(&mut self) -> Result<IfDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("if")?;
        let condition = self.parse_expr()?;
        self.expect(TokenKind::LBrace, "'{' to start if body")?;
        let then_branch = self.parse_declaration_block()?;
        self.expect(TokenKind::RBrace, "'}' to end if body")?;

        let else_branch = if self.check_ident("else") {
            self.advance();
            self.expect(TokenKind::LBrace, "'{' to start else body")?;
            let declarations = self.parse_declaration_block()?;
            self.expect(TokenKind::RBrace, "'}' to end else body")?;
            Some(declarations)
        } else {
            None
        };

        Ok(IfDecl { condition, then_branch, else_branch, span: self.span_since(start) })
    }

    /// `for` Identifier `in` `[` (Identifier | StringLiteral) (',' ...)* `]` `{` <declaration>* `}`
    fn parse_for(&mut self) -> Result<ForDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("for")?;
        let binding = self.expect_ident_any("loop binding name")?;
        self.expect_ident_text("in")?;
        let items = self.parse_ident_or_string_list()?;
        self.expect(TokenKind::LBrace, "'{' to start for body")?;
        let body = self.parse_declaration_block()?;
        self.expect(TokenKind::RBrace, "'}' to end for body")?;
        Ok(ForDecl { binding, items, body, span: self.span_since(start) })
    }

    /// Shared by `if`/`for`/function bodies: zero or more declarations up
    /// to (but not consuming) the closing `}`, skipping stray commas the
    /// same way `parse_program`/`parse_function_decl` already do.
    fn parse_declaration_block(&mut self) -> Result<Vec<Declaration>, ParseError> {
        let mut declarations = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.consume_comma() {
                continue;
            }
            declarations.push(self.parse_declaration()?);
        }
        Ok(declarations)
    }

    /// `[` (Identifier | StringLiteral) (',' ...)* `]` or a single bare
    /// item -- mirrors `parse_string_list`'s "bracketed list or singleton"
    /// shape but also accepts identifiers (e.g. pool names), collapsing
    /// both into `String` per `ForDecl::items`.
    fn parse_ident_or_string_list(&mut self) -> Result<Vec<String>, ParseError> {
        let parse_item = |p: &mut Self| -> Result<String, ParseError> {
            if p.check(TokenKind::String(String::new())) {
                p.expect_string("for-loop item")
            } else {
                p.expect_ident_any("for-loop item")
            }
        };
        if !self.check(TokenKind::LBracket) {
            return Ok(vec![parse_item(self)?]);
        }
        self.expect(TokenKind::LBracket, "'[' to start item list")?;
        let mut values = Vec::new();
        while !self.check(TokenKind::RBracket) {
            values.push(parse_item(self)?);
            if !self.consume_comma() && !self.check(TokenKind::RBracket) {
                return Err(self.error_here("expected ',' or ']' in item list"));
            }
        }
        self.expect(TokenKind::RBracket, "']' after item list")?;
        Ok(values)
    }

    fn parse_function_decl(&mut self, doc: Option<String>) -> Result<FunctionDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("fn")?;
        let name = self.expect_ident_any("function name")?;

        // `fn name<T, U>(...)` -- optional type-parameter list, usable
        // only as `Pool<T>`'s state slot (see `PoolState::Var`). Scoped
        // for the rest of this function's params/return type/body;
        // cleared unconditionally before returning below, including on
        // every early-return error path, since a stale scope would leak
        // into whatever's parsed next.
        let type_params = if self.check(TokenKind::Lt) {
            self.advance();
            let mut names = Vec::new();
            while !self.check(TokenKind::Gt) {
                names.push(self.expect_ident_any("type parameter name")?);
                if !self.consume_comma() && !self.check(TokenKind::Gt) {
                    return Err(self.error_here("expected ',' or '>' in type parameter list"));
                }
            }
            self.expect(TokenKind::Gt, "'>' after type parameter list")?;
            names
        } else {
            Vec::new()
        };
        // A named `fn` can itself be declared nested inside another
        // function's (or a closure's) body -- merge with whatever the
        // outer scope already had rather than clobbering it, the same
        // fix `parse_closure_expr` needs for the identical reason.
        let outer_type_params = self.type_params_in_scope.clone();
        let mut merged_type_params = outer_type_params.clone();
        merged_type_params.extend(type_params.iter().cloned());
        self.type_params_in_scope = merged_type_params;

        let result = self.parse_function_decl_rest(start, name, type_params, doc);
        self.type_params_in_scope = outer_type_params;
        result
    }

    fn parse_function_decl_rest(
        &mut self,
        start: (usize, usize),
        name: String,
        type_params: Vec<String>,
        doc: Option<String>,
    ) -> Result<FunctionDecl, ParseError> {
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
            type_params,
            params,
            return_type,
            body,
            span: self.span_since(start),
            doc,
        })
    }

    fn parse_import(&mut self) -> Result<ImportDecl, ParseError> {
        let start = self.start_span();
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
        Ok(ImportDecl { source, items, span: self.span_since(start) })
    }

    fn parse_pool(&mut self, doc: Option<String>) -> Result<PoolDecl, ParseError> {
        let start = self.start_span();
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
            span: self.span_since(start),
            doc,
        })
    }

    fn parse_strand(&mut self, doc: Option<String>) -> Result<StrandDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("strand")?;
        let name = self.expect_ident_any("strand name")?;
        self.expect(TokenKind::Colon, "':' after strand name")?;
        self.expect_ident_text("Strand")?;
        self.expect(TokenKind::Eq, "'=' before strand literal")?;
        let sequence = self.expect_string("strand sequence")?;
        Ok(StrandDecl { name, sequence, span: self.span_since(start), doc })
    }

    /// `enum Name { Variant1, Variant2(PayloadType), ... }` (Step 14).
    /// Each variant is a bare name (unit) or a name followed by exactly
    /// one parenthesized payload type -- see `ast::EnumVariant`'s own doc
    /// comment for why never more than one.
    fn parse_enum_decl(&mut self, doc: Option<String>) -> Result<EnumDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("enum")?;
        let name = self.expect_ident_any("enum name")?;
        self.expect(TokenKind::LBrace, "'{' after enum name")?;
        let mut variants = Vec::new();
        while !self.check(TokenKind::RBrace) {
            let variant_start = self.start_span();
            let variant_name = self.expect_ident_any("variant name")?;
            let payload = if self.check(TokenKind::LParen) {
                self.advance();
                let ty = self.parse_type_expr()?;
                self.expect(TokenKind::RParen, "')' after variant payload type")?;
                Some(ty)
            } else {
                None
            };
            variants.push(EnumVariant { name: variant_name, payload, span: self.span_since(variant_start) });
            if !self.consume_comma() && !self.check(TokenKind::RBrace) {
                return Err(self.error_here("expected ',' or '}' in enum variant list"));
            }
        }
        self.expect(TokenKind::RBrace, "'}' to close enum body")?;
        Ok(EnumDecl { name, variants, span: self.span_since(start), doc })
    }

    fn parse_sequence_decl(&mut self, doc: Option<String>) -> Result<SequenceDecl, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("seq")?;
        let name = self.expect_ident_any("sequence name")?;
        self.expect(TokenKind::Colon, "':' after sequence name")?;
        self.expect_ident_text("Sequence")?;
        self.expect(TokenKind::Eq, "'=' before sequence literal")?;
        let sequence = self.expect_string("sequence literal")?;
        Ok(SequenceDecl { name, sequence, span: self.span_since(start), doc })
    }

    fn parse_let_decl(&mut self, doc: Option<String>) -> Result<Declaration, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("let")?;
        let name = self.expect_ident_any("binding name")?;
        if self.check(TokenKind::Colon) {
            self.advance();
            if self.check_ident("Sequence") {
                self.advance();
                self.expect(TokenKind::Eq, "'=' before binding expression")?;
                self.expect_ident_text("seq")?;
                let sequence = self.expect_string("sequence literal")?;
                return Ok(Declaration::Sequence(SequenceDecl { name, sequence, span: self.span_since(start), doc }));
            }
            if doc.is_some() {
                return Err(self.error_here("doc comments can only precede pool/strand/seq/fn/pipeline declarations, not 'let'"));
            }
            let annotation = self.parse_type_expr()?;
            self.expect(TokenKind::Eq, "'=' before binding expression")?;
            let expr = self.parse_expr()?;
            return Ok(Declaration::Let(LetDecl { name, annotation, expr, span: self.span_since(start) }));
        }
        self.expect(TokenKind::Eq, "'=' before binding expression")?;
        self.expect_ident_text("seq")?;
        let sequence = self.expect_string("sequence literal")?;
        Ok(Declaration::Sequence(SequenceDecl { name, sequence, span: self.span_since(start), doc }))
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        if self.check_ident("Pool") {
            self.advance();
            self.expect(TokenKind::Lt, "'<' after Pool")?;
            let state_name = self.expect_ident_any("pool profile or state")?;
            // A name matching the enclosing function's own `<T, U>` list
            // is a type parameter, not a concrete profile -- checked
            // before `PoolState::parse` so `Pool<T>` doesn't fall
            // through to "unknown pool profile or state 'T'". A typo'd
            // name that's neither a declared type parameter nor a real
            // profile still hits that exact existing error unchanged.
            let state = if self.type_params_in_scope.contains(&state_name) {
                PoolState::Var(state_name)
            } else {
                PoolState::parse(&state_name)
                    .ok_or_else(|| self.error_previous(format!("unknown pool profile or state '{}'", state_name)))?
            };
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
        } else if self.check_ident("Result") {
            self.advance();
            self.expect(TokenKind::Lt, "'<' after Result")?;
            let ok_ty = self.parse_type_expr()?;
            self.expect(TokenKind::Comma, "',' between Result's Ok and Err types")?;
            let err_ty = self.parse_type_expr()?;
            self.expect(TokenKind::Gt, "'>' after Result type")?;
            Ok(TypeExpr::Result(Box::new(ok_ty), Box::new(err_ty)))
        } else if self.check_ident("Str") {
            self.advance();
            Ok(TypeExpr::Str)
        } else if self.check_ident("Fn") {
            self.advance();
            self.expect(TokenKind::LParen, "'(' after Fn")?;
            let mut params = Vec::new();
            while !self.check(TokenKind::RParen) {
                params.push(self.parse_type_expr()?);
                if !self.consume_comma() && !self.check(TokenKind::RParen) {
                    return Err(self.error_here("expected ',' or ')' in Fn's parameter type list"));
                }
            }
            self.expect(TokenKind::RParen, "')' after Fn's parameter type list")?;
            if !self.check(TokenKind::Arrow) && !self.check_ident("returns") {
                return Err(self.error_here("expected '->' or 'returns' after Fn's parameter type list"));
            }
            self.advance();
            let return_type = self.parse_type_expr()?;
            Ok(TypeExpr::Fn(params, Box::new(return_type)))
        } else if let TokenKind::Ident(name) = &self.peek().kind {
            // Any identifier not matching one of the built-in type
            // keywords above is presumed to name a user-declared `enum`
            // (Step 14) -- the parser has no `self.enums` table to check
            // against (declarations aren't resolved until typeck), so a
            // genuinely undeclared/misspelled name here is deferred to
            // `E-ENUM-UNKNOWN` at type-check time rather than a parse
            // error, the same "accept optimistically, validate later"
            // precedent `PoolState::Var` already set for `Pool<T>`.
            let name = name.clone();
            self.advance();
            Ok(TypeExpr::Enum(name))
        } else {
            Err(self.error_here("expected type annotation: Pool<...>, Strand, Sequence, File, DnaFile, Recovery, Result<...>, Str, Fn(...), Void, or a declared enum name"))
        }
    }

    /// Top-level expression entry point: precedence-climbing over the new
    /// boolean/comparison operators, bottoming out at `parse_primary_expr`
    /// (the pre-existing keyword-dispatch logic, unchanged) for anything
    /// that isn't one of them. Every existing call site keeps calling
    /// `parse_expr()`, so a program using none of the new operators parses
    /// identically to before -- this is purely additive.
    ///
    /// Precedence, loosest to tightest: `||`, `&&`, unary `!`, then a
    /// single (non-chaining) comparison, then a primary expression. There
    /// is no arithmetic (`+`/`-`/`*`/`/`) -- NucleScript's only numeric
    /// operands today are literal numbers and a probabilistic pool
    /// binding's inferred error rate (see `typeck::resolve_numeric`), and
    /// nothing yet needs to combine those beyond comparing them.
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.check(TokenKind::OrOr) {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expr::BinaryOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not_expr()?;
        while self.check(TokenKind::AndAnd) {
            self.advance();
            let right = self.parse_not_expr()?;
            left = Expr::BinaryOp { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_not_expr(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::Bang) {
            self.advance();
            let inner = self.parse_not_expr()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_comparison_expr()
    }

    fn parse_comparison_expr(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_postfix_expr()?;
        let op = if self.check(TokenKind::EqEq) {
            BinOp::Eq
        } else if self.check(TokenKind::NotEq) {
            BinOp::Ne
        } else if self.check(TokenKind::Le) {
            BinOp::Le
        } else if self.check(TokenKind::Ge) {
            BinOp::Ge
        } else if self.check(TokenKind::Lt) {
            BinOp::Lt
        } else if self.check(TokenKind::Gt) {
            BinOp::Gt
        } else {
            return Ok(left);
        };
        self.advance();
        let right = self.parse_postfix_expr()?;
        Ok(Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) })
    }

    /// A primary expression followed by zero or more postfix `?`
    /// operators (`x?`, `x??` -- the latter isn't meaningful today since
    /// nothing produces a nested `Result`, but the loop costs nothing and
    /// avoids special-casing "exactly one `?`"). `?` binds tighter than
    /// comparison, like Rust (`x? == y` means `(x?) == y`), so this sits
    /// directly around `parse_primary_expr`'s result rather than as its
    /// own precedence layer above comparison -- it's the only postfix
    /// operator in the grammar, which doesn't justify a layer of its own.
    fn parse_postfix_expr(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary_expr()?;
        while self.check(TokenKind::Question) {
            self.advance();
            expr = Expr::Try(Box::new(expr));
        }
        Ok(expr)
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        if self.check(TokenKind::LParen) {
            self.advance();
            let inner = self.parse_expr()?;
            self.expect(TokenKind::RParen, "')' to close parenthesized expression")?;
            return Ok(inner);
        }
        // `store`/`simulate store ... into ...`/`retrieve from ...`/
        // `delete ... from ...` in *expression* position (e.g. the
        // right-hand side of a `let`) -- reuses the exact same
        // `parse_store`/`parse_retrieve`/`parse_delete` the *statement*
        // form (`parse_declaration`) already calls, so there's exactly
        // one grammar for each, not two. Checked before the bare
        // `simulate <pool> under <profile>` (`Expr::SimulatePool`) branch
        // below, with a one-token lookahead, since both start with the
        // same `simulate` keyword: `simulate store` is unambiguously a
        // store, `simulate <anything-else>` is unambiguously SimulatePool.
        if self.check_ident("store") || (self.check_ident("simulate") && self.check_ident_ahead(1, "store")) {
            Ok(Expr::StoreExpr(self.parse_store()?))
        } else if self.check_ident("retrieve") {
            Ok(Expr::RetrieveExpr(self.parse_retrieve()?))
        } else if self.check_ident("delete") {
            Ok(Expr::DeleteExpr(self.parse_delete()?))
        } else if self.check_ident("simulate") {
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
            // Sugar over a call to the `consensus_vote` stdlib function
            // (`stdlib::builtin_functions`) -- desugars straight to
            // `Expr::FunctionCall` so every consumer downstream (typeck,
            // effects, middle) resolves it exactly like a call to any
            // user-defined function, with no separate AST node to keep in
            // sync. `coverage: 10x`'s multiplier suffix is stripped here;
            // only the numeric value survives past this point.
            self.advance();
            self.expect(TokenKind::LParen, "'(' after consensus_vote")?;
            let source = self.expect_ident_any("source pool binding")?;
            self.expect(TokenKind::Comma, "',' after source pool binding")?;
            self.expect_ident_text("coverage")?;
            self.expect(TokenKind::Colon, "':' after coverage")?;
            let coverage = self.expect_multiplier("coverage multiplier")?;
            self.expect(TokenKind::RParen, "')' after consensus_vote")?;
            Ok(Expr::FunctionCall {
                name: "consensus_vote".to_string(),
                args: vec![Expr::Variable(source), Expr::Number(coverage as f64)],
                explicit_type_args: Vec::new(),
            })
        } else if self.check_ident("protect") {
            // Sugar over a call to the `protect` stdlib function -- see
            // the `consensus_vote` case above for why this desugars to
            // `Expr::FunctionCall` rather than its own AST node.
            self.advance();
            let data = self.expect_ident_any("data name")?;
            self.expect_ident_text("for")?;
            let guarantee = self.expect_ident_any("guarantee name")?;
            Ok(Expr::FunctionCall {
                name: "protect".to_string(),
                args: vec![Expr::Variable(data), Expr::Variable(guarantee)],
                explicit_type_args: Vec::new(),
            })
        } else if self.check_ident("match") {
            self.parse_match_expr()
        } else if self.check_ident("fn") {
            self.parse_closure_expr()
        } else if self.check_ident("Ok") && self.tokens.get(self.index + 1).map(|t| &t.kind) == Some(&TokenKind::LParen) {
            self.advance(); // `Ok`
            self.advance(); // `(`
            let inner = self.parse_expr()?;
            self.expect(TokenKind::RParen, "')' after Ok(...)")?;
            Ok(Expr::Ok(Box::new(inner)))
        } else if self.check_ident("Err") && self.tokens.get(self.index + 1).map(|t| &t.kind) == Some(&TokenKind::LParen) {
            self.advance(); // `Err`
            self.advance(); // `(`
            let inner = self.parse_expr()?;
            self.expect(TokenKind::RParen, "')' after Err(...)")?;
            Ok(Expr::Err(Box::new(inner)))
        } else if matches!(self.peek().kind, TokenKind::Ident(_))
            && matches!(self.tokens.get(self.index + 1).map(|t| &t.kind), Some(TokenKind::ColonColon))
            && matches!(self.tokens.get(self.index + 2).map(|t| &t.kind), Some(TokenKind::Ident(_)))
        {
            // `EnumName::Variant(payload)` / `EnumName::Variant` -- reuses
            // the `::` token added for turbofish (Step 13). Unambiguous by
            // one token of lookahead: turbofish is always `::` followed by
            // `<`, a variant reference is always `::` followed by an
            // identifier (see `Expr::EnumConstruct`'s doc comment).
            let enum_name = self.expect_ident_any("enum name")?;
            self.expect(TokenKind::ColonColon, "'::' after enum name")?;
            let variant = self.expect_ident_any("variant name")?;
            let payload = if self.check(TokenKind::LParen) {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(TokenKind::RParen, "')' after enum variant payload")?;
                Some(Box::new(inner))
            } else {
                None
            };
            Ok(Expr::EnumConstruct { enum_name, variant, payload })
        } else if let TokenKind::Ident(name) = &self.peek().kind {
            let name = name.clone();
            let next_is_call = matches!(
                self.tokens.get(self.index + 1).map(|t| &t.kind),
                Some(TokenKind::LParen) | Some(TokenKind::ColonColon)
            );
            if next_is_call {
                self.advance(); // consume ident
                // `name::<Illumina, Nanopore>(...)` -- explicit type
                // arguments, only needed when a generic function's type
                // parameter can't be inferred from any argument. Reuses
                // `Profile::parse` directly (a type argument is always a
                // concrete profile -- the only thing a `PoolState::Var`
                // can ever be unified against).
                let mut explicit_type_args = Vec::new();
                if self.check(TokenKind::ColonColon) {
                    self.advance();
                    self.expect(TokenKind::Lt, "'<' after '::'")?;
                    loop {
                        let profile_name = self.expect_ident_any("explicit type argument")?;
                        let profile = Profile::parse(&profile_name)
                            .ok_or_else(|| self.error_previous(format!("unknown profile '{}'", profile_name)))?;
                        explicit_type_args.push(profile);
                        if !self.consume_comma() {
                            break;
                        }
                    }
                    self.expect(TokenKind::Gt, "'>' after explicit type arguments")?;
                }
                self.expect(TokenKind::LParen, "'(' after function name")?;
                let mut args = Vec::new();
                while !self.check(TokenKind::RParen) {
                    args.push(self.parse_expr()?);
                    if !self.consume_comma() && !self.check(TokenKind::RParen) {
                        return Err(self.error_here("expected ',' or ')' in argument list"));
                    }
                }
                self.expect(TokenKind::RParen, "')' after function arguments")?;
                Ok(Expr::FunctionCall { name, args, explicit_type_args })
            } else {
                self.advance(); // consume ident
                Ok(Expr::Variable(name))
            }
        } else if self.check(TokenKind::String(String::new())) {
            let val = self.expect_string("string literal")?;
            Ok(Expr::StringLiteral(val))
        } else if self.check(TokenKind::Number(String::new())) {
            let raw = self.expect_number("number literal")?;
            let value = raw.parse::<f64>().map_err(|_| self.error_previous(format!("invalid number literal '{}'", raw)))?;
            Ok(Expr::Number(value))
        } else {
            Err(self.error_here("expected expression"))
        }
    }

    /// `match <scrutinee> { <arm>, ... }` -- the general pattern-matching
    /// engine (Step 14). Any number of arms, any order (no longer fixed
    /// `Ok` then `Err`) -- `typeck::TypeChecker::check_match` validates
    /// exhaustiveness/duplicate-variant/wildcard-position against the
    /// scrutinee's own declared variant list (built-in Result, or a user
    /// `enum`). See `Expr::Match`'s doc comment in ast.rs.
    fn parse_match_expr(&mut self) -> Result<Expr, ParseError> {
        self.advance(); // `match`
        let scrutinee = self.parse_expr()?;
        self.expect(TokenKind::LBrace, "'{' to open match arms")?;
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) {
            arms.push(self.parse_match_arm()?);
            if !self.consume_comma() && !self.check(TokenKind::RBrace) {
                return Err(self.error_here("expected ',' or '}' between match arms"));
            }
        }
        self.expect(TokenKind::RBrace, "'}' to close match arms")?;
        Ok(Expr::Match { scrutinee: Box::new(scrutinee), arms })
    }

    /// One `match` arm: `_ => <expr>` (wildcard, no capture), `Variant =>
    /// <expr>` (unit variant, nothing to bind), or `Variant(<binding>) =>
    /// <expr>` (payload variant). `_` is an ordinary identifier at the
    /// lexer level (`is_ident_start` accepts `_`), so it's recognized here
    /// by value rather than needing its own token kind.
    fn parse_match_arm(&mut self) -> Result<MatchArm, ParseError> {
        let start = self.start_span();
        let name = self.expect_ident_any("match arm variant name or '_'")?;
        let (variant, binding) = if name == "_" {
            (None, None)
        } else if self.check(TokenKind::LParen) {
            self.advance();
            let binding = self.expect_ident_any("pattern binding name")?;
            self.expect(TokenKind::RParen, "')' to close pattern")?;
            (Some(name), Some(binding))
        } else {
            (Some(name), None)
        };
        self.expect(TokenKind::FatArrow, "'=>'")?;
        let body = self.parse_expr()?;
        Ok(MatchArm { variant, binding, body: Box::new(body), span: self.span_since(start) })
    }

    /// `fn(params) -> ReturnType { body }` in *expression* position -- an
    /// anonymous closure literal, never a name. Deliberately not sharing
    /// `parse_function_decl_rest`'s param-list/return-type/body-parsing
    /// with a refactor: that function is tightly coupled to building a
    /// named `FunctionDecl` (with `type_params`/`doc` fields a closure
    /// doesn't have), so a small amount of duplication here is the
    /// additive-over-invasive-refactor tradeoff this project already
    /// prefers elsewhere.
    fn parse_closure_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.start_span();
        self.advance(); // `fn`

        // `fn<T, U>(...)` -- optional type-parameter list, same grammar
        // `parse_function_decl` already has. Closures can nest inside a
        // generic function's (or another generic closure's) own body, so
        // unlike `parse_function_decl` (safe only because a top-level
        // `fn` never nests), this must *merge* with the outer scope
        // rather than clobber it: a non-generic closure nested inside a
        // generic function still needs to recognize that function's own
        // `Pool<T>` if it references it, and the outer scope must come
        // back exactly as it was once this closure's own signature/body
        // is done parsing.
        let type_params = if self.check(TokenKind::Lt) {
            self.advance();
            let mut names = Vec::new();
            while !self.check(TokenKind::Gt) {
                names.push(self.expect_ident_any("type parameter name")?);
                if !self.consume_comma() && !self.check(TokenKind::Gt) {
                    return Err(self.error_here("expected ',' or '>' in type parameter list"));
                }
            }
            self.expect(TokenKind::Gt, "'>' after type parameter list")?;
            names
        } else {
            Vec::new()
        };
        let outer_type_params = self.type_params_in_scope.clone();
        let mut merged_type_params = outer_type_params.clone();
        merged_type_params.extend(type_params.iter().cloned());
        self.type_params_in_scope = merged_type_params;

        let result = self.parse_closure_expr_rest(start, type_params);
        self.type_params_in_scope = outer_type_params;
        result
    }

    fn parse_closure_expr_rest(&mut self, start: (usize, usize), type_params: Vec<String>) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LParen, "'(' after 'fn'")?;
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) {
            let param_name = self.expect_ident_any("parameter name")?;
            self.expect(TokenKind::Colon, "':' after parameter name")?;
            let ty = self.parse_type_expr()?;
            params.push(FnParam { name: param_name, ty });
            if !self.consume_comma() && !self.check(TokenKind::RParen) {
                return Err(self.error_here("expected ',' or ')' in closure parameter list"));
            }
        }
        self.expect(TokenKind::RParen, "')' after closure parameter list")?;
        if !self.check(TokenKind::Arrow) && !self.check_ident("returns") {
            return Err(self.error_here("expected '->' or 'returns' followed by a return type after a closure's parameters"));
        }
        self.advance();
        let return_type = self.parse_type_expr()?;
        self.expect(TokenKind::LBrace, "'{' to start closure body")?;
        let mut body = Vec::new();
        while !self.check(TokenKind::RBrace) {
            if self.consume_comma() {
                continue;
            }
            body.push(self.parse_declaration()?);
        }
        self.expect(TokenKind::RBrace, "'}' to end closure body")?;
        Ok(Expr::Closure { type_params, params, return_type, body, span: self.span_since(start) })
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
        let start = self.start_span();
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
        Ok(StoreOp { simulate, file, pool, options, span: self.span_since(start) })
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
        let start = self.start_span();
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
        Ok(RetrieveOp { pool, query, span: self.span_since(start) })
    }

    fn parse_delete(&mut self) -> Result<DeleteOp, ParseError> {
        let start = self.start_span();
        self.expect_ident_text("delete")?;
        let file = if self.check(TokenKind::String(String::new())) {
            self.expect_string("file path")?
        } else {
            self.expect_ident_any("file variable")?
        };
        self.expect_ident_text("from")?;
        let pool = self.expect_ident_any("pool name")?;
        let confirmed = self.consume_confirmation("physical_key")?;
        Ok(DeleteOp { file, pool, confirmed, span: self.span_since(start) })
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

    fn parse_pipeline(&mut self, doc: Option<String>) -> Result<PipelineDecl, ParseError> {
        let start = self.start_span();
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
        Ok(PipelineDecl { name, steps, span: self.span_since(start), doc })
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

    /// Like `check_ident`, but for the token `offset` positions ahead of
    /// the current one -- used to disambiguate `simulate <pool> under
    /// ...` (`Expr::SimulatePool`) from `simulate store <file> into ...`
    /// (a `StoreOp` with `simulate: true`) before committing to either
    /// parse, since both start with the same keyword.
    fn check_ident_ahead(&self, offset: usize, expected: &str) -> bool {
        matches!(self.tokens.get(self.index + offset).map(|t| &t.kind), Some(TokenKind::Ident(actual)) if actual.eq_ignore_ascii_case(expected))
    }

    fn consume_comma(&mut self) -> bool {
        if self.check(TokenKind::Comma) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Line/column of the next token, to be paired with `span_since` once
    /// the declaration/operation that starts there has finished parsing.
    fn start_span(&self) -> (usize, usize) {
        let token = self.peek();
        (token.line, token.column)
    }

    /// Build a `Span` from a `start_span()` point to the last token this
    /// parser actually consumed -- good enough to underline "this
    /// declaration" in an editor without needing per-character end
    /// tracking in the lexer.
    fn span_since(&self, start: (usize, usize)) -> Span {
        let end = &self.tokens[self.index.saturating_sub(1)];
        Span { line: start.0, column: start.1, end_line: end.line, end_column: end.column }
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

    #[test]
    fn attaches_a_doc_comment_to_a_pool_declaration() {
        let src = "/// The main archive.\npool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }";
        let program = parse(src);
        let Declaration::Pool(pool) = &program.declarations[0] else { panic!("expected pool declaration") };
        assert_eq!(pool.doc.as_deref(), Some("The main archive."));
    }

    #[test]
    fn joins_consecutive_doc_comment_lines() {
        let src = "/// Line one.\n/// Line two.\nfn helper(x: Void) returns Void {\n}";
        let program = parse(src);
        let Declaration::Function(func) = &program.declarations[0] else { panic!("expected function declaration") };
        assert_eq!(func.doc.as_deref(), Some("Line one.\nLine two."));
    }

    #[test]
    fn rejects_a_doc_comment_before_a_declaration_that_cannot_carry_one() {
        for src in [
            "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n/// bad\nstore \"x\" into a",
            "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n/// bad\nlet noisy: Pool<Illumina, 0.35%> = simulate a under Illumina",
            "/// bad\nif 1.0 > 0.5 {\n}",
            "/// bad\ntest \"x\" {\n}",
        ] {
            let tokens = Lexer::new(src).tokenize().unwrap();
            let result = Parser::new(tokens).parse_program();
            assert!(result.is_err(), "expected a parse error for: {src}");
        }
    }
}
