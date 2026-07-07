//! Canonical formatter for NucleScript (`nucle fmt`).
//!
//! One opinionated style, zero configuration -- the same philosophy as
//! `gofmt`: there is exactly one way `nucle fmt` writes a given program,
//! so two developers never bikeshed over indentation or brace placement,
//! and a diff never contains unrelated whitespace churn.
//!
//! # Design
//!
//! This does **not** re-derive source text from the AST. The AST (by
//! design, see `ast.rs`) drops comments and normalizes literal spellings,
//! so printing from it would silently delete every `//` comment in the
//! file and could reorder/rewrite things a token-faithful formatter
//! should leave alone. Instead, formatting works directly on the real
//! token stream (`lexer::Lexer`, which already carries each token's
//! original line/column) plus a small dedicated scan for comments (which
//! the lexer discards during tokenization). The AST is only consulted for
//! one thing: each top-level declaration's start line (`Declaration::
//! span()`), used to decide where the "exactly one blank line between
//! top-level declarations" rule applies.
//!
//! Concretely: `nucle fmt`
//! - keeps every *line-break* the input already has (like `gofmt`, it
//!   does not invent line-wrapping heuristics for a first cut),
//! - recomputes each line's *indentation* from bracket-nesting depth,
//! - recomputes *inter-token spacing* on a line from a small set of
//!   spacing rules,
//! - collapses runs of 2+ blank lines to exactly 1 everywhere, and
//! - forces exactly 1 blank line between top-level declarations
//!   (inserting one if the input had none), while leaving a declaration's
//!   own leading comment block attached to it with no blank line between
//!   them.
//!
//! Because every rule above is a pure function of (tokens, comments,
//! which line-breaks exist), running the formatter on its own output is
//! provably a no-op: the second pass sees the exact same tokens and
//! comments, and the line-breaks it "preserves" are exactly the ones the
//! first pass already decided on.

use crate::ast::Program;
use crate::lexer::{Lexer, LexError, Token, TokenKind};
use crate::parser::{ParseError, Parser};
use std::collections::HashSet;
use std::fmt;

const INDENT_UNIT: &str = "    ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    pub message: String,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FormatError {}

impl From<LexError> for FormatError {
    fn from(value: LexError) -> Self {
        Self { message: format!("lex error: {}", value) }
    }
}

impl From<ParseError> for FormatError {
    fn from(value: ParseError) -> Self {
        Self { message: format!("parse error: {}", value) }
    }
}

/// Re-renders `source` in NucleScript's one canonical style. Errors if
/// `source` doesn't lex/parse -- formatting is only defined for
/// syntactically valid NucleScript, the same precondition `rustfmt`/
/// `gofmt` impose.
pub fn format_source(source: &str) -> Result<String, FormatError> {
    let tokens = Lexer::new(source).tokenize()?;
    let program = Parser::new(tokens.clone()).parse_program()?;
    let comments = extract_comments(source);
    Ok(render(&tokens, &comments, &program))
}

/// Whether `source` is already in canonical form -- `nucle fmt --check`'s
/// underlying test. Re-lexing/re-parsing the *formatted* output to
/// compare token-for-token would be more robust to incidental trailing-
/// whitespace differences, but a straight string comparison is simpler
/// and, since `format_source` never emits trailing whitespace, equivalent
/// in practice.
pub fn is_formatted(source: &str) -> Result<bool, FormatError> {
    Ok(format_source(source)? == source)
}

#[derive(Debug, Clone)]
struct CommentTrivia {
    line: usize,
    text: String,
    /// True if nothing but whitespace precedes `//` on this source line
    /// (a standalone comment line); false if it trails real code on the
    /// same line.
    own_line: bool,
}

/// Scans raw source for `//...` comments, the one piece of source text
/// `Lexer::tokenize` discards. Mirrors `Lexer`'s own string-literal
/// handling (escape-aware, `//` inside a string is not a comment) so a
/// URL or path literal like `"http://example.com"` is never misread as a
/// comment start.
fn extract_comments(source: &str) -> Vec<CommentTrivia> {
    let chars: Vec<char> = source.chars().collect();
    let mut comments = Vec::new();
    let mut i = 0;
    let mut line = 1usize;
    let mut in_string = false;
    let mut line_has_content = false;

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            line_has_content = false;
            i += 1;
            continue;
        }
        if in_string {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            line_has_content = true;
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            line_has_content = true;
            i += 1;
            continue;
        }
        // `///` is a *doc* comment -- it's already a real `TokenKind::
        // DocComment` in the lexer's token stream (see `lexer.rs`), which
        // `build_entries` places into `code_lines`. If this scanner also
        // recorded it as a standalone `CommentTrivia`, the same line
        // would get two competing entries (one `Code`, one `Comment`),
        // breaking `build_entries`'s "one entry per line" invariant. Skip
        // the line's text without recording anything, but still advance
        // `i`/`line` past it exactly like a plain comment would.
        if c == '/' && chars.get(i + 1) == Some(&'/') && chars.get(i + 2) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let own_line = !line_has_content;
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            comments.push(CommentTrivia { line, text: text.trim_end().to_string(), own_line });
            continue;
        }
        if !c.is_whitespace() {
            line_has_content = true;
        }
        i += 1;
    }
    comments
}

/// One physical output line's worth of source: either real code tokens
/// (with an optional same-line trailing comment) or a standalone comment.
enum LineEntry<'a> {
    Code { line: usize, tokens: Vec<&'a Token>, trailing_comment: Option<String> },
    Comment { line: usize, text: String },
}

impl LineEntry<'_> {
    fn line(&self) -> usize {
        match self {
            LineEntry::Code { line, .. } => *line,
            LineEntry::Comment { line, .. } => *line,
        }
    }
}

fn render(tokens: &[Token], comments: &[CommentTrivia], program: &Program) -> String {
    let entries = build_entries(tokens, comments);
    let unit_start_lines = declaration_unit_start_lines(&entries, program);

    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut generic_depth: i32 = 0;
    let mut prev_line: Option<usize> = None;

    for entry in &entries {
        let line = entry.line();
        if let Some(prev) = prev_line {
            let gap = line.saturating_sub(prev);
            let force_blank = unit_start_lines.contains(&line);
            if force_blank || gap >= 2 {
                out.push('\n');
            }
        }
        if prev_line.is_some() {
            out.push('\n');
        }

        match entry {
            LineEntry::Comment { text, .. } => {
                out.push_str(&INDENT_UNIT.repeat(depth.max(0) as usize));
                out.push_str(text);
            }
            LineEntry::Code { tokens, trailing_comment, .. } => {
                let leading_closers = tokens
                    .iter()
                    .take_while(|t| matches!(t.kind, TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket))
                    .count();
                let line_indent = (depth - leading_closers as i32).max(0);
                out.push_str(&INDENT_UNIT.repeat(line_indent as usize));
                out.push_str(&render_tokens(tokens, &mut depth, &mut generic_depth));
                if let Some(comment) = trailing_comment {
                    out.push(' ');
                    out.push_str(comment);
                }
            }
        }
        prev_line = Some(line);
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Groups tokens by source line and interleaves standalone comment lines
/// in source order, producing the ordered sequence `render` walks.
fn build_entries<'a>(tokens: &'a [Token], comments: &[CommentTrivia]) -> Vec<LineEntry<'a>> {
    let mut entries = Vec::new();
    let mut token_lines: Vec<(usize, Vec<&Token>)> = Vec::new();
    for token in tokens {
        if matches!(token.kind, TokenKind::Eof) {
            continue;
        }
        match token_lines.last_mut() {
            Some((line, group)) if *line == token.line => group.push(token),
            _ => token_lines.push((token.line, vec![token])),
        }
    }

    let mut trailing_by_line: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let mut standalone: Vec<(usize, String)> = Vec::new();
    let code_lines: HashSet<usize> = token_lines.iter().map(|(l, _)| *l).collect();
    for comment in comments {
        if comment.own_line || !code_lines.contains(&comment.line) {
            standalone.push((comment.line, comment.text.clone()));
        } else {
            trailing_by_line.insert(comment.line, comment.text.clone());
        }
    }

    let mut code_iter = token_lines.into_iter().peekable();
    let mut comment_iter = standalone.into_iter().peekable();
    loop {
        match (code_iter.peek(), comment_iter.peek()) {
            (Some((code_line, _)), Some((comment_line, _))) => {
                if comment_line < code_line {
                    let (line, text) = comment_iter.next().unwrap();
                    entries.push(LineEntry::Comment { line, text });
                } else {
                    let (line, group) = code_iter.next().unwrap();
                    let trailing_comment = trailing_by_line.get(&line).cloned();
                    entries.push(LineEntry::Code { line, tokens: group, trailing_comment });
                }
            }
            (Some(_), None) => {
                let (line, group) = code_iter.next().unwrap();
                let trailing_comment = trailing_by_line.get(&line).cloned();
                entries.push(LineEntry::Code { line, tokens: group, trailing_comment });
            }
            (None, Some(_)) => {
                let (line, text) = comment_iter.next().unwrap();
                entries.push(LineEntry::Comment { line, text });
            }
            (None, None) => break,
        }
    }
    entries
}

/// The line each top-level declaration's "unit" (its own leading block of
/// standalone comments, if any, plus the declaration itself) starts on --
/// the boundary `render` forces exactly one blank line before. Walking a
/// declaration's leading comments backward (rather than just using its
/// own span line) keeps a doc comment glued to the declaration it
/// documents instead of splitting them with an inserted blank line.
fn declaration_unit_start_lines(entries: &[LineEntry<'_>], program: &Program) -> HashSet<usize> {
    let mut unit_starts = HashSet::new();
    for (index, decl) in program.declarations.iter().enumerate() {
        if index == 0 {
            continue;
        }
        let decl_line = decl.span().line;
        let Some(entry_index) = entries.iter().position(|e| matches!(e, LineEntry::Code { line, .. } if *line == decl_line))
        else {
            continue;
        };
        let mut unit_line = decl_line;
        let mut walk = entry_index;
        while walk > 0 {
            let Some(line) = leading_trivia_line(&entries[walk - 1]) else { break };
            if line + 1 != unit_line {
                break;
            }
            unit_line = line;
            walk -= 1;
        }
        unit_starts.insert(unit_line);
    }
    unit_starts
}

/// The line number of `entry` if it's a piece of leading-documentation
/// trivia -- a standalone `//` comment, or a `///` doc comment (which,
/// unlike a plain comment, is a real token and so shows up as a one-token
/// `Code` entry, not a `Comment` one; see `TokenKind::DocComment`). Used
/// by `declaration_unit_start_lines` to glue either form to the
/// declaration it documents, so the forced blank line between top-level
/// declarations lands *before* the documentation, not between it and the
/// thing it documents.
fn leading_trivia_line(entry: &LineEntry<'_>) -> Option<usize> {
    match entry {
        LineEntry::Comment { line, .. } => Some(*line),
        LineEntry::Code { line, tokens, .. } if matches!(tokens.as_slice(), [t] if matches!(t.kind, TokenKind::DocComment(_))) => {
            Some(*line)
        }
        _ => None,
    }
}

/// Renders one source line's worth of tokens with canonical inter-token
/// spacing, updating `depth` (bracket-nesting, drives the *next* line's
/// indentation) and `generic_depth` (tracks `Pool<...>`, the one
/// angle-bracket construct that must NOT be spaced like a comparison) as
/// it goes.
fn render_tokens(tokens: &[&Token], depth: &mut i32, generic_depth: &mut i32) -> String {
    let mut out = String::new();
    let mut prev: Option<&Token> = None;

    for token in tokens {
        if let Some(prev_token) = prev {
            if needs_space(prev_token, token, *generic_depth) {
                out.push(' ');
            }
        }
        out.push_str(&token_text(token));

        match &token.kind {
            TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => *depth += 1,
            TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => *depth -= 1,
            TokenKind::Lt => {
                if is_pool_generic_open(prev) {
                    *generic_depth += 1;
                }
            }
            TokenKind::Gt if *generic_depth > 0 => *generic_depth -= 1,
            _ => {}
        }
        prev = Some(token);
    }
    out
}

fn is_pool_generic_open(prev: Option<&Token>) -> bool {
    matches!(prev.map(|t| &t.kind), Some(TokenKind::Ident(name)) if name == "Pool")
}

/// Whether a space belongs between two adjacent tokens on the same
/// output line. Default is "yes" -- every exception below is a
/// deliberate, narrow override, not the common case.
fn needs_space(prev: &Token, cur: &Token, generic_depth: i32) -> bool {
    // `Pool<Illumina, 0.35%>` -- type-parameter angle brackets, not a
    // comparison, are never spaced (commas inside them still are, via
    // the default rule below).
    if matches!(cur.kind, TokenKind::Lt) && is_pool_generic_open(Some(prev)) {
        return false;
    }
    if matches!(prev.kind, TokenKind::Lt) && generic_depth > 0 {
        return false;
    }
    if matches!(cur.kind, TokenKind::Gt) && generic_depth > 0 {
        return false;
    }

    // No space right after an opening bracket/paren, or right after `!`.
    if matches!(prev.kind, TokenKind::LParen | TokenKind::LBracket | TokenKind::Bang) {
        return false;
    }
    // `seq"ATCG..."` -- every real example writes the `seq` DNA-literal
    // cast glued to its string, like a literal prefix (`b"..."`,
    // `r"..."` in other languages), not `seq "..."` as if `seq` were an
    // ordinary keyword taking a spaced argument. Preserved as the one
    // established idiom worth keeping rather than flattening under the
    // default "always space between tokens" rule.
    if matches!(&prev.kind, TokenKind::Ident(name) if name == "seq") && matches!(cur.kind, TokenKind::String(_)) {
        return false;
    }
    // No space right before a closing paren/bracket, a comma, or a colon.
    // `}` is deliberately NOT here: unlike call-parens and list-brackets,
    // this grammar's `{ ... }` blocks are always written with a space
    // before the close (`{ codec: Ternary, ... }`, `} else {`), whether
    // on one line or as a lone closing line (where there's no preceding
    // token on that line to space against anyway).
    if matches!(cur.kind, TokenKind::RParen | TokenKind::RBracket | TokenKind::Comma | TokenKind::Colon) {
        return false;
    }
    // Function-call / builtin-call style: `name(` with no space, unless
    // `name` is a control keyword that takes a parenthesized/grouped
    // expression afterward (`if`/`assert`; `for`'s `(` never directly
    // follows the keyword itself).
    if matches!(cur.kind, TokenKind::LParen) {
        if let TokenKind::Ident(name) = &prev.kind {
            if !matches!(name.as_str(), "if" | "assert") {
                return false;
            }
        }
    }
    true
}

fn token_text(token: &Token) -> String {
    match &token.kind {
        TokenKind::Ident(s) => s.clone(),
        TokenKind::String(s) => format!("\"{}\"", escape_string(s)),
        TokenKind::Number(s) => s.clone(),
        TokenKind::LBrace => "{".to_string(),
        TokenKind::RBrace => "}".to_string(),
        TokenKind::LBracket => "[".to_string(),
        TokenKind::RBracket => "]".to_string(),
        TokenKind::LParen => "(".to_string(),
        TokenKind::RParen => ")".to_string(),
        TokenKind::Colon => ":".to_string(),
        TokenKind::Comma => ",".to_string(),
        TokenKind::Eq => "=".to_string(),
        TokenKind::Gt => ">".to_string(),
        TokenKind::Lt => "<".to_string(),
        TokenKind::Arrow => "->".to_string(),
        TokenKind::EqEq => "==".to_string(),
        TokenKind::NotEq => "!=".to_string(),
        TokenKind::Le => "<=".to_string(),
        TokenKind::Ge => ">=".to_string(),
        TokenKind::AndAnd => "&&".to_string(),
        TokenKind::OrOr => "||".to_string(),
        TokenKind::Bang => "!".to_string(),
        // A `///` doc comment always occupies its own line (it consumes
        // the rest of the line it starts on, same as a plain `//`
        // comment), so it's always the sole token on whatever "code" line
        // `build_entries` groups it into -- rendering it back out
        // verbatim here is enough, no special line-placement logic needed.
        TokenKind::DocComment(text) if text.is_empty() => "///".to_string(),
        TokenKind::DocComment(text) => format!("/// {}", text),
        TokenKind::Eof => String::new(),
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_minimal_pool_and_store() {
        let src = "pool archive:DnaPool{codec:Ternary,redundancy:3x,profile:Illumina}\nstore \"a.txt\" into archive";
        let formatted = format_source(src).unwrap();
        assert_eq!(
            formatted,
            "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n\nstore \"a.txt\" into archive\n"
        );
    }

    #[test]
    fn is_idempotent_on_its_own_output() {
        let src = r#"
            pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }


            let noisy: Pool<Illumina, 0.35%> = simulate archive under Illumina
            if noisy > 0.1 && noisy < 5.0 {
                let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 10x)
            } else {
                let recovered: Pool<Recovered> = consensus_vote(noisy, coverage: 2x)
            }
        "#;
        let once = format_source(src).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn preserves_comments() {
        let src = "// a doc comment\npool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina } // trailing";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("// a doc comment"));
        assert!(formatted.contains("// trailing"));
    }

    #[test]
    fn forces_exactly_one_blank_line_between_top_level_declarations() {
        let src = "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\npool b: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("}\n\npool b"));
    }

    #[test]
    fn keeps_leading_doc_comment_glued_to_its_declaration() {
        let src = "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n// describes b\npool b: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("}\n\n// describes b\npool b"));
    }

    #[test]
    fn rejects_invalid_syntax() {
        assert!(format_source("pool archive DnaPool {").is_err());
    }

    #[test]
    fn does_not_space_pool_generic_brackets() {
        let src = "let noisy:Pool<Illumina,0.35%> = simulate archive under Illumina";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("Pool<Illumina, 0.35%>"));
    }

    #[test]
    fn spaces_comparison_and_boolean_operators() {
        let src = "if noisy>0.1&&!(noisy<5.0){\npool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n}";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("noisy > 0.1 && !(noisy < 5.0)"));
    }

    #[test]
    fn spaces_assert_before_a_parenthesized_condition_like_if() {
        let src = "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n\ntest \"x\" {\n    assert(1.0<2.0)\n}";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("assert (1.0 < 2.0)"), "got: {formatted}");
    }

    #[test]
    fn formats_test_and_assert_declarations() {
        let src = "pool a:DnaPool{codec:Ternary,redundancy:1x,profile:Illumina}\ntest \"name\"{assert 1.0<2.0,\"msg\"}";
        let formatted = format_source(src).unwrap();
        let twice = format_source(&formatted).unwrap();
        assert_eq!(formatted, twice, "formatting should be idempotent");
        assert!(formatted.contains("test \"name\" {"));
        assert!(formatted.contains("assert 1.0 < 2.0, \"msg\""));
    }

    #[test]
    fn keeps_a_doc_comment_glued_to_its_declaration_not_split_by_the_blank_line_rule() {
        let src = "pool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }\n/// Documents b.\npool b: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }";
        let formatted = format_source(src).unwrap();
        assert!(formatted.contains("}\n\n/// Documents b.\npool b"), "got: {formatted}");
    }

    #[test]
    fn preserves_multi_line_doc_comments_and_is_idempotent() {
        let src = "/// Line one.\n/// Line two.\nfn helper(x: Void) returns Void {\n}";
        let once = format_source(src).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
        assert!(once.contains("/// Line one.\n/// Line two.\nfn helper"), "got: {once}");
    }

    #[test]
    fn a_plain_comment_is_still_distinct_from_a_doc_comment() {
        let src = "// plain comment, not a doc comment\npool a: DnaPool { codec: Ternary, redundancy: 1x, profile: Illumina }";
        let formatted = format_source(src).unwrap();
        assert!(formatted.starts_with("// plain comment, not a doc comment\npool a"));
    }
}
