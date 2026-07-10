//! Lexer for NucleScript.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    String(String),
    Number(String),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Colon,
    Comma,
    Eq,
    Gt,
    Lt,
    Arrow,
    /// `=>` -- separates a `match` arm's pattern from its body.
    FatArrow,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `!`
    Bang,
    /// `?` -- postfix Result-unwrap/propagate operator.
    Question,
    /// `/// text` -- a doc comment, distinct from a plain `//` comment
    /// (which is discarded during tokenizing and never becomes a token at
    /// all). The `String` is the line's text with the leading `///` and
    /// exactly one following space (if present) stripped. See
    /// `parser::Parser::parse_declaration`'s doc-comment accumulation for
    /// how consecutive `DocComment` tokens become one declaration's
    /// `doc: Option<String>`.
    DocComment(String),
    Eof,
}

pub struct Lexer<'a> {
    chars: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
    _source: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            line: 1,
            column: 1,
            _source: source,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            let line = self.line;
            let column = self.column;
            let Some(ch) = self.peek() else {
                tokens.push(Token { kind: TokenKind::Eof, line, column });
                break;
            };

            let kind = match ch {
                '{' => { self.bump(); TokenKind::LBrace }
                '}' => { self.bump(); TokenKind::RBrace }
                '[' => { self.bump(); TokenKind::LBracket }
                ']' => { self.bump(); TokenKind::RBracket }
                '(' => { self.bump(); TokenKind::LParen }
                ')' => { self.bump(); TokenKind::RParen }
                ':' => { self.bump(); TokenKind::Colon }
                ',' => { self.bump(); TokenKind::Comma }
                '=' if self.peek_next() == Some('=') => { self.bump(); self.bump(); TokenKind::EqEq }
                '=' if self.peek_next() == Some('>') => { self.bump(); self.bump(); TokenKind::FatArrow }
                '=' => { self.bump(); TokenKind::Eq }
                '!' if self.peek_next() == Some('=') => { self.bump(); self.bump(); TokenKind::NotEq }
                '!' => { self.bump(); TokenKind::Bang }
                '?' => { self.bump(); TokenKind::Question }
                '>' if self.peek_next() == Some('=') => { self.bump(); self.bump(); TokenKind::Ge }
                '>' => { self.bump(); TokenKind::Gt }
                '<' if self.peek_next() == Some('=') => { self.bump(); self.bump(); TokenKind::Le }
                '<' => { self.bump(); TokenKind::Lt }
                '&' if self.peek_next() == Some('&') => { self.bump(); self.bump(); TokenKind::AndAnd }
                '|' if self.peek_next() == Some('|') => { self.bump(); self.bump(); TokenKind::OrOr }
                '-' if self.peek_next() == Some('>') => {
                    self.bump();
                    self.bump();
                    TokenKind::Arrow
                }
                '"' => TokenKind::String(self.lex_string(line, column)?),
                '/' if self.peek_next() == Some('/') && self.peek_at(2) == Some('/') => {
                    TokenKind::DocComment(self.lex_doc_comment())
                }
                c if c.is_ascii_digit() => TokenKind::Number(self.lex_number_like()),
                c if is_ident_start(c) => TokenKind::Ident(self.lex_ident()),
                other => {
                    return Err(LexError {
                        line,
                        column,
                        message: format!("unexpected character '{}'", other),
                    });
                }
            };
            tokens.push(Token { kind, line, column });
        }
        Ok(tokens)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(c) if c.is_whitespace()) {
                self.bump();
            }

            // A plain `//` comment (exactly two slashes, not `///`) is
            // discarded here and never becomes a token -- `///` is left
            // alone so `tokenize()`'s main dispatch can turn it into a
            // real `DocComment` token instead.
            if self.peek() == Some('/') && self.peek_next() == Some('/') && self.peek_at(2) != Some('/') {
                while let Some(c) = self.peek() {
                    self.bump();
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    /// Consumes a `///` doc comment line, returning its text with the
    /// `///` prefix and at most one following space stripped (so `///
    /// Foo` becomes `"Foo"`, matching how Rustdoc's `///` conventionally
    /// reads).
    fn lex_doc_comment(&mut self) -> String {
        self.bump();
        self.bump();
        self.bump();
        if self.peek() == Some(' ') {
            self.bump();
        }
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            text.push(c);
            self.bump();
        }
        text
    }

    fn lex_string(&mut self, line: usize, column: usize) -> Result<String, LexError> {
        self.bump(); // opening quote
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            match ch {
                '"' => {
                    self.bump();
                    return Ok(value);
                }
                '\\' => {
                    self.bump();
                    let escaped = self.bump().ok_or_else(|| LexError {
                        line,
                        column,
                        message: "unterminated escape sequence".into(),
                    })?;
                    let decoded = match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        other => other,
                    };
                    value.push(decoded);
                }
                other => {
                    self.bump();
                    value.push(other);
                }
            }
        }
        Err(LexError { line, column, message: "unterminated string literal".into() })
    }

    fn lex_number_like(&mut self) -> String {
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '%' | '_') {
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        value
    }

    fn lex_ident(&mut self) -> String {
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            if is_ident_continue(ch) {
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        value
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.index + 1).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}:{}", self.message, self.line, self.column)
    }
}

impl std::error::Error for LexError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_redundancy_and_percent_literals() {
        let tokens = Lexer::new("redundancy: 3x, expect recovery > 99.5%").tokenize().unwrap();
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Number("3x".into())));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Number("99.5%".into())));
    }

    #[test]
    fn doc_comments_become_real_tokens_distinct_from_plain_comments() {
        let tokens = Lexer::new("/// a doc comment\n// a plain comment\npool").tokenize().unwrap();
        assert!(tokens.iter().any(|t| t.kind == TokenKind::DocComment("a doc comment".into())));
        // The plain `//` comment is discarded entirely -- only `pool`
        // (and the trailing Eof) should remain besides the doc comment.
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1].kind, TokenKind::Ident("pool".into()));
    }

    #[test]
    fn doc_comment_with_no_leading_space_is_captured_verbatim() {
        let tokens = Lexer::new("///no leading space").tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::DocComment("no leading space".into()));
    }
}
