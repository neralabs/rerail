//! `ast.parse`'s verdict on a module's text, as CPython 3.13 gives it (core.rr's `syntax_error(d, l, msg)`): whether it
//! raises `SyntaxError` (or its subclasses `IndentationError`, `TabError`), and the error's `lineno` and `msg`.
//!
//! Ruff's parser (`ruff_python_parser`, at `cpython::VERSION`) reads the grammar; its errors and the version-gated
//! syntax it reports (`unsupported_syntax_errors`: a t-string, `except A, B:`) are where CPython's parser fails. Only
//! what `ast.parse` rejects counts: compile-time errors (`return` outside a function, a repeated keyword argument, a
//! duplicate parameter, `nonlocal` at module level) are not syntax errors here, as they are not in `ast.parse`.
//! CPython's choice among several errors is followed where it differs from the earliest: a generic `invalid syntax`
//! gives way to a tokenizer error later in the file (an unterminated string, a stray bracket, a bad dedent) and to a
//! bracket left open on an earlier line (`'(' was never closed`), as `_PyPegen_tokenize_full_source_to_check_for_errors`
//! does. Messages are CPython's for the errors CPython names (indentation, unterminated strings, brackets, `expected
//! ':'`, invalid targets); any other is `invalid syntax`.

use ruff_python_ast::statement_visitor::{StatementVisitor, walk_stmt};
use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{Expr, Mod, Stmt};
use ruff_python_parser::{
    InterpolatedStringErrorType, LexicalErrorType, Mode, ParseErrorType, ParseOptions, UnsupportedSyntaxErrorKind, parse_unchecked,
};
use ruff_text_size::{Ranged, TextRange, TextSize};

/// CPython's `SyntaxError`: its `lineno` (1 where CPython gives none, for a null byte) and `msg`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) line: usize,
    pub(crate) msg: String,
}

/// `ast.parse(source)`'s error, `None` where it parses.
pub(crate) fn syntax_error(source: &str) -> Option<SyntaxError> {
    if source.contains('\0') {
        return Some(SyntaxError { line: 1, msg: "source code string cannot contain null bytes".into() });
    }
    let lines = Lines::new(source);
    // CPython reads a str: a byte order mark is a character, which its tokenizer refuses (Ruff skips it).
    if source.starts_with('\u{feff}') {
        return Some(SyntaxError { line: 1, msg: "invalid non-printable character U+FEFF".into() });
    }
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(super::VERSION));
    let tokens = parsed.tokens();
    let cx = Cx { source, lines: &lines, tokens, module: parsed.syntax() };
    let mut found: Vec<Found> = Vec::new();
    for e in parsed.errors() {
        found.push(Found { at: e.location.start(), end: e.location.end(), what: What::Parse(&e.error) });
    }
    for e in parsed.unsupported_syntax_errors() {
        found.push(Found { at: e.range.start(), end: e.range.end(), what: What::Unsupported(e.kind) });
    }
    let brackets = Brackets::scan(tokens);
    for &(at, ref msg) in &brackets.errors {
        found.push(Found { at, end: at, what: What::Tokenizer(msg.clone()) });
    }
    for (at, msg) in try_without_handlers(parsed.syntax(), tokens) {
        found.push(Found { at, end: at, what: What::Named(msg) });
    }
    found.sort_by_key(|f| (f.at, f.rank()));
    let first = found.first()?;
    let mut chosen = first.message(&cx);
    let unclosed = brackets.open.last().map(|&(open, kind)| Chosen {
        line: lines.line(open),
        msg: format!("'{}' was never closed", bracket_char(kind)),
        generic: false,
        at_eof: false,
    });
    if chosen.generic {
        // CPython tokenizes the rest of the file after a generic error: a tokenizer error there replaces it, and so does
        // a bracket still open at the end that opened on an earlier line.
        if let Some(later) = found.iter().skip(1).filter(|f| f.at > first.at).find(|f| f.is_tokenizer()) {
            chosen = later.message(&cx);
        } else if let Some(unclosed) = unclosed
            && (chosen.at_eof || unclosed.line < chosen.line)
        {
            chosen = unclosed;
        }
    } else if chosen.at_eof
        && let Some(unclosed) = unclosed
    {
        // The parser ran into the end of the file inside a bracket.
        chosen = unclosed;
    }
    Some(SyntaxError { line: chosen.line, msg: chosen.msg })
}

/// Line starts, at CPython's terminators (`\r\n`, `\r`, `\n`).
struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    starts.push(i + 2);
                    i += 2;
                    continue;
                }
                b'\r' | b'\n' => starts.push(i + 1),
                _ => {}
            }
            i += 1;
        }
        Self { starts, len: bytes.len() }
    }

    /// The 1-based line of byte offset `at`.
    fn line(&self, at: TextSize) -> usize {
        self.starts.partition_point(|&s| s <= at.to_usize())
    }

    /// The line the tokenizer is on at the end of the text: its last character's.
    fn count(&self) -> usize {
        self.line(TextSize::new(self.len.saturating_sub(1) as u32))
    }
}

/// The brackets as CPython's tokenizer pairs them: the errors it raises at a closing one (none open, or another kind
/// open) and those still open at the end, innermost last.
struct Brackets {
    errors: Vec<(TextSize, String)>,
    open: Vec<(TextSize, TokenKind)>,
}

impl Brackets {
    fn scan(tokens: &Tokens) -> Self {
        let mut errors: Vec<(TextSize, String)> = Vec::new();
        let mut open: Vec<(TextSize, TokenKind)> = Vec::new();
        for t in tokens.iter() {
            match t.kind() {
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => open.push((t.start(), t.kind())),
                k @ (TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace) => match open.last() {
                    None => errors.push((t.start(), format!("unmatched '{}'", bracket_char(k)))),
                    Some(&(_, o)) if closing(o) != k => {
                        errors.push((t.start(), format!("closing parenthesis '{}' does not match opening parenthesis '{}'", bracket_char(k), bracket_char(o))));
                        // CPython stops at the first; the rest of the scan is never read once this is chosen.
                        open.pop();
                    }
                    Some(_) => {
                        open.pop();
                    }
                },
                _ => {}
            }
        }
        Self { errors, open }
    }
}

fn closing(open: TokenKind) -> TokenKind {
    match open {
        TokenKind::Lpar => TokenKind::Rpar,
        TokenKind::Lsqb => TokenKind::Rsqb,
        _ => TokenKind::Rbrace,
    }
}

fn bracket_char(kind: TokenKind) -> char {
    match kind {
        TokenKind::Lpar => '(',
        TokenKind::Rpar => ')',
        TokenKind::Lsqb => '[',
        TokenKind::Rsqb => ']',
        TokenKind::Lbrace => '{',
        _ => '}',
    }
}

struct Found<'a> {
    at: TextSize,
    end: TextSize,
    what: What<'a>,
}

enum What<'a> {
    Parse(&'a ParseErrorType),
    Unsupported(UnsupportedSyntaxErrorKind),
    /// A bracket error of CPython's tokenizer, its message.
    Tokenizer(String),
    /// What CPython's parser rejects and Ruff's accepts, its message.
    Named(String),
}

/// A message chosen: its line, its text, and whether it is CPython's generic `invalid syntax`.
struct Chosen {
    line: usize,
    msg: String,
    generic: bool,
    at_eof: bool,
}

impl Found<'_> {
    /// At one offset, a tokenizer error comes first: CPython's tokenizer raises it before the parser sees the token.
    fn rank(&self) -> u8 {
        u8::from(!self.is_tokenizer())
    }

    fn is_tokenizer(&self) -> bool {
        match &self.what {
            What::Tokenizer(_) => true,
            What::Parse(ParseErrorType::Lexical(l)) => matches!(
                l,
                LexicalErrorType::UnclosedStringError
                    | LexicalErrorType::IndentationError
                    | LexicalErrorType::UnrecognizedToken { .. }
                    | LexicalErrorType::LineContinuationError
                    | LexicalErrorType::FStringError(
                        InterpolatedStringErrorType::UnterminatedString | InterpolatedStringErrorType::UnterminatedTripleQuotedString
                    )
            ),
            _ => false,
        }
    }

    fn message(&self, cx: &Cx) -> Chosen {
        let (source, lines, tokens) = (cx.source, cx.lines, cx.tokens);
        let line = lines.line(self.at);
        let at_eof = tokens
            .iter()
            .rev()
            .find(|t| !t.kind().is_trivia() && !matches!(t.kind(), TokenKind::EndOfFile | TokenKind::Newline | TokenKind::Dedent))
            .is_none_or(|t| t.end() <= self.at);
        let generic = |line| Chosen { line, msg: "invalid syntax".into(), generic: true, at_eof };
        let named = |line, msg: String| Chosen { line, msg, generic: false, at_eof };
        let target = || expr_at(cx.module, self.at, self.end);
        let name = || target().map_or("expression", expr_name);
        match &self.what {
            What::Tokenizer(msg) | What::Named(msg) => named(line, msg.clone()),
            What::Unsupported(UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes) => {
                named(line, "multiple exception types must be parenthesized".into())
            }
            What::Unsupported(_) => generic(line),
            What::Parse(e) => match e {
                ParseErrorType::Lexical(l) => match l {
                    LexicalErrorType::UnclosedStringError => {
                        let text = &source[self.at.to_usize()..];
                        let quote_at = text.find(['\'', '"']).unwrap_or(0);
                        let q = &text[quote_at..];
                        if q.starts_with("'''") || q.starts_with("\"\"\"") {
                            named(line, format!("unterminated triple-quoted string literal (detected at line {})", lines.count()))
                        } else {
                            named(line, format!("unterminated string literal (detected at line {})", lines.line(self.end)))
                        }
                    }
                    LexicalErrorType::IndentationError => named(line, "unindent does not match any outer indentation level".into()),
                    LexicalErrorType::UnrecognizedToken { tok } => named(line, invalid_character(*tok)),
                    LexicalErrorType::LineContinuationError => named(line, "unexpected character after line continuation character".into()),
                    LexicalErrorType::InvalidByteLiteral => named(line, "bytes can only contain ASCII literal characters".into()),
                    LexicalErrorType::FStringError(f) => fstring(f, self, cx, line, &named),
                    _ => generic(line),
                },
                ParseErrorType::FStringError(f) => fstring(f, self, cx, line, &named),
                ParseErrorType::UnexpectedIndentation => named(line, "unexpected indent".into()),
                ParseErrorType::ExpectedToken { expected: TokenKind::Colon, .. } => named(line, "expected ':'".into()),
                ParseErrorType::ExpectedToken { expected: TokenKind::Comma | TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace, found }
                    if depth_at(tokens, self.at) > 0 && starts_expression(*found) =>
                {
                    // `invalid_expression`: two expressions side by side in brackets, reported at the first. Its
                    // parse reads on to the end of the file where a bracket is never closed, which is reported then.
                    let before = tokens.iter().take_while(|t| t.end() <= self.at).filter(|t| !t.kind().is_trivia()).last();
                    let line = before.map_or(line, |t| lines.line(t.start()));
                    Chosen { line, msg: "invalid syntax. Perhaps you forgot a comma?".into(), generic: false, at_eof: true }
                }
                // A line break inside brackets is no token for CPython: it fails at the next one.
                ParseErrorType::ExpectedToken { found: TokenKind::Newline, .. } if depth_at(tokens, self.at) > 0 => {
                    let next = tokens.iter().find(|t| t.start() > self.at && !t.kind().is_trivia() && !t.kind().is_any_newline());
                    generic(next.map_or(line, |t| lines.line(t.start())))
                }
                ParseErrorType::EmptyTypeParams => named(line, "Type parameter list cannot be empty".into()),
                ParseErrorType::EmptyImportNames => named(line, "Expected one or more names after 'import'".into()),
                ParseErrorType::InvalidMatchPatternTarget => named(line, "cannot use '_' as a target".into()),
                ParseErrorType::OtherError(msg) => match msg.strip_prefix("Expected an indented block after ") {
                    Some(clause) => {
                        let header = header_line(tokens, self.at, lines);
                        // At the end of the file CPython's position is its last line, comments included.
                        let line = if at_eof { last_content_line(source, lines).max(line) } else { line };
                        named(line, format!("expected an indented block after {} on line {header}", clause_name(clause)))
                    }
                    None => match other_message(msg) {
                        Some(m) => named(line, m.into()),
                        None => generic(line),
                    },
                },
                ParseErrorType::InvalidDeleteTarget => named(line, format!("cannot delete {}", name())),
                ParseErrorType::InvalidAssignmentTarget => {
                    // `invalid_named_expression`: an operand target followed by `=` and a value is "here", with a hint.
                    let operand = target().is_some_and(|e| {
                        !matches!(
                            e,
                            Expr::List(_)
                                | Expr::Tuple(_)
                                | Expr::Generator(_)
                                | Expr::BooleanLiteral(_)
                                | Expr::NoneLiteral(_)
                                | Expr::Compare(_)
                                | Expr::BoolOp(_)
                                | Expr::If(_)
                                | Expr::Lambda(_)
                                | Expr::Named(_)
                                | Expr::Yield(_)
                                | Expr::YieldFrom(_)
                                | Expr::Starred(_)
                        ) && !matches!(e, Expr::UnaryOp(u) if u.op == ruff_python_ast::UnaryOp::Not)
                    });
                    let next = tokens.iter().find(|t| t.start() >= self.end && !t.kind().is_trivia()).map(|t| t.kind());
                    if operand && next == Some(TokenKind::Equal) {
                        named(line, format!("cannot assign to {} here. Maybe you meant '==' instead of '='?", name()))
                    } else {
                        named(line, format!("cannot assign to {}", name()))
                    }
                }
                ParseErrorType::InvalidAugmentedAssignmentTarget => {
                    named(line, format!("'{}' is an illegal expression for augmented assignment", name()))
                }
                ParseErrorType::InvalidAnnotatedAssignmentTarget => match target() {
                    Some(Expr::Tuple(_)) => named(line, "only single target (not tuple) can be annotated".into()),
                    Some(Expr::List(_)) => named(line, "only single target (not list) can be annotated".into()),
                    _ => named(line, "illegal target for annotation".into()),
                },
                ParseErrorType::InvalidNamedAssignmentTarget => named(line, format!("cannot use assignment expressions with {}", name())),
                ParseErrorType::UnparenthesizedGeneratorExpression => named(line, "Generator expression must be parenthesized".into()),
                ParseErrorType::PositionalAfterKeywordArgument => named(line, "positional argument follows keyword argument".into()),
                ParseErrorType::PositionalAfterKeywordUnpacking => {
                    named(line, "positional argument follows keyword argument unpacking".into())
                }
                ParseErrorType::InvalidArgumentUnpackingOrder => {
                    named(line, "iterable argument unpacking follows keyword argument unpacking".into())
                }
                ParseErrorType::NonDefaultParamAfterDefaultParam => {
                    named(line, "parameter without a default follows parameter with a default".into())
                }
                ParseErrorType::ParamAfterVarKeywordParam => named(line, "arguments cannot follow var-keyword argument".into()),
                ParseErrorType::ExpectedKeywordParam => named(line, "named arguments must follow bare *".into()),
                _ => generic(line),
            },
        }
    }
}

/// What a message is read from: the text, its lines, Ruff's tokens and tree.
struct Cx<'a> {
    source: &'a str,
    lines: &'a Lines,
    tokens: &'a Tokens,
    module: &'a Mod,
}

/// The outermost expression spanning exactly `start..end`.
fn expr_at(module: &Mod, start: TextSize, end: TextSize) -> Option<&Expr> {
    struct At<'a> {
        range: TextRange,
        found: Option<&'a Expr>,
    }
    impl<'a> Visitor<'a> for At<'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if self.found.is_some() || !expr.range().contains_range(self.range) {
                return;
            }
            if expr.range() == self.range {
                self.found = Some(expr);
                return;
            }
            walk_expr(self, expr);
        }
    }
    let mut at = At { range: TextRange::new(start, end), found: None };
    if let Mod::Module(m) = module {
        at.visit_body(&m.body);
    }
    at.found
}

/// CPython's name of an expression in its messages (`_PyPegen_get_expr_name`).
fn expr_name(e: &Expr) -> &'static str {
    match e {
        Expr::Attribute(_) => "attribute",
        Expr::Subscript(_) => "subscript",
        Expr::Starred(_) => "starred",
        Expr::Name(_) => "name",
        Expr::List(_) => "list",
        Expr::Tuple(_) => "tuple",
        Expr::Lambda(_) => "lambda",
        Expr::Call(_) => "function call",
        Expr::BoolOp(_) | Expr::BinOp(_) | Expr::UnaryOp(_) | Expr::Slice(_) | Expr::IpyEscapeCommand(_) => "expression",
        Expr::Generator(_) => "generator expression",
        Expr::Yield(_) | Expr::YieldFrom(_) => "yield expression",
        Expr::Await(_) => "await expression",
        Expr::ListComp(_) => "list comprehension",
        Expr::SetComp(_) => "set comprehension",
        Expr::DictComp(_) => "dict comprehension",
        Expr::Dict(_) => "dict literal",
        Expr::Set(_) => "set display",
        Expr::FString(_) | Expr::TString(_) => "f-string expression",
        Expr::NoneLiteral(_) => "None",
        Expr::BooleanLiteral(b) if b.value => "True",
        Expr::BooleanLiteral(_) => "False",
        Expr::EllipsisLiteral(_) => "ellipsis",
        Expr::StringLiteral(_) | Expr::BytesLiteral(_) | Expr::NumberLiteral(_) => "literal",
        Expr::Compare(_) => "comparison",
        Expr::If(_) => "conditional expression",
        Expr::Named(_) => "named expression",
    }
}

/// Ruff's errors on `source` and the brackets left open, for a difference report.
#[cfg(test)]
pub(crate) fn explain(source: &str) -> String {
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(super::VERSION));
    let lines = Lines::new(source);
    let mut out: Vec<String> = parsed.errors().iter().map(|e| format!("L{} {:?}", lines.line(e.location.start()), e.error)).collect();
    out.extend(parsed.unsupported_syntax_errors().iter().map(|e| format!("L{} unsupported {:?}", lines.line(e.range.start()), e.kind)));
    let brackets = Brackets::scan(parsed.tokens());
    out.extend(brackets.open.iter().map(|(at, k)| format!("open {k:?} at L{}", lines.line(*at))));
    out.join("; ")
}

/// CPython's message for a character its tokenizer cannot start a token with.
fn invalid_character(c: char) -> String {
    if is_printable(c) { format!("invalid character '{c}' (U+{:04X})", c as u32) } else { format!("invalid non-printable character U+{:04X}", c as u32) }
}

/// `str.isprintable` of one character: not a control, format, surrogate, private-use, unassigned or separator one
/// (but the space).
fn is_printable(c: char) -> bool {
    static NOT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[\p{C}\p{Z}]$").unwrap());
    c == ' ' || !NOT.is_match(c.encode_utf8(&mut [0; 4]))
}

/// CPython's name of the clause whose block is missing, from Ruff's.
fn clause_name(ruff: &str) -> String {
    match ruff {
        "`class` definition" => "class definition".into(),
        "function definition" => "function definition".into(),
        "`case` block" => "'case' statement".into(),
        _ => {
            let keyword = ruff.split('`').nth(1).unwrap_or(ruff);
            format!("'{keyword}' statement")
        }
    }
}

/// The line of the clause header a missing block follows: the first token of the logical line holding the last `:`
/// before `at`.
fn header_line(tokens: &Tokens, at: TextSize, lines: &Lines) -> usize {
    let before: Vec<_> = tokens.iter().take_while(|t| t.start() < at).filter(|t| !t.kind().is_trivia()).collect();
    let Some(colon) = before.iter().rposition(|t| t.kind() == TokenKind::Colon) else { return lines.line(at) };
    let start = before[..colon].iter().rposition(|t| matches!(t.kind(), TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent)).map_or(0, |i| i + 1);
    lines.line(before[start].start())
}

/// CPython's message for an f-string error of Ruff's.
fn fstring(e: &InterpolatedStringErrorType, at: &Found, cx: &Cx, line: usize, named: &dyn Fn(usize, String) -> Chosen) -> Chosen {
    let text = &cx.source[at.at.to_usize()..];
    let msg = match e {
        InterpolatedStringErrorType::UnclosedLbrace => "f-string: expecting '}'".into(),
        InterpolatedStringErrorType::SingleRbrace => "f-string: single '}' is not allowed".into(),
        InterpolatedStringErrorType::LambdaWithoutParentheses => "f-string: lambda expressions are not allowed without parentheses".into(),
        InterpolatedStringErrorType::ConversionFlagNotImmediatelyAfterExclamation => {
            "f-string: conversion type must come right after the exclamation mark".into()
        }
        InterpolatedStringErrorType::NewlineInFormatSpec => {
            "f-string: newlines are not allowed in format specifiers for single quoted f-strings".into()
        }
        InterpolatedStringErrorType::InvalidConversionFlag => {
            let name: String = text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                format!("f-string: invalid conversion character '{name}': expected 's', 'r', or 'a'")
            } else {
                "f-string: invalid conversion character".into()
            }
        }
        InterpolatedStringErrorType::UnterminatedString => format!("unterminated f-string literal (detected at line {})", cx.lines.line(at.end)),
        InterpolatedStringErrorType::UnterminatedTripleQuotedString => {
            format!("unterminated triple-quoted f-string literal (detected at line {})", cx.lines.count())
        }
    };
    named(line, msg)
}

/// CPython's message for one of Ruff's `OtherError`s, where CPython names it.
fn other_message(ruff: &str) -> Option<&'static str> {
    Some(match ruff {
        "Multiple exception types must be parenthesized when using `as`" => "multiple exception types must be parenthesized",
        "Trailing comma not allowed" => "trailing comma not allowed without surrounding parentheses",
        "Bytes literal cannot be mixed with non-bytes literals" => "cannot mix bytes and nonbytes literals",
        "Only one '/' separator allowed" => "/ may appear only once",
        "Only one '*' separator allowed"
        | "Only one '*' parameter allowed"
        | "Keyword-only parameter separator not allowed after '*' parameter" => "* argument may appear only once",
        "'/' parameter must appear before '*' parameter" => "/ must be ahead of *",
        _ => return None,
    })
}

/// The bracket depth at `at`, as CPython's tokenizer counts it.
fn depth_at(tokens: &Tokens, at: TextSize) -> usize {
    tokens.iter().take_while(|t| t.start() < at).fold(0usize, |d, t| match t.kind() {
        TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => d + 1,
        TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => d.saturating_sub(1),
        _ => d,
    })
}

/// Whether a token can start an expression (so two expressions stand side by side).
fn starts_expression(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Name
            | TokenKind::Int
            | TokenKind::Float
            | TokenKind::Complex
            | TokenKind::String
            | TokenKind::FStringStart
            | TokenKind::Lpar
            | TokenKind::Lsqb
            | TokenKind::Lbrace
            | TokenKind::None
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Ellipsis
            | TokenKind::Tilde
            | TokenKind::Not
            | TokenKind::Lambda
            | TokenKind::Await
    )
}

/// The last line holding anything but whitespace, comments included.
fn last_content_line(source: &str, lines: &Lines) -> usize {
    source.char_indices().rev().find(|(_, c)| !c.is_whitespace()).map_or(1, |(i, _)| lines.line(TextSize::new(i as u32)))
}

/// `try:` with an `else` and no `except` (a `finally` or not): Ruff accepts it, CPython's grammar does not, at the
/// `else`.
fn try_without_handlers(module: &Mod, tokens: &Tokens) -> Vec<(TextSize, String)> {
    struct Tries<'t> {
        tokens: &'t Tokens,
        out: Vec<(TextSize, String)>,
    }
    impl<'a> StatementVisitor<'a> for Tries<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Try(t) = stmt
                && t.handlers.is_empty()
                && !t.orelse.is_empty()
                && let Some(last) = t.body.last()
            {
                let at = self.tokens.iter().find(|k| k.start() >= last.end() && k.kind() == TokenKind::Else).map_or(last.end(), |k| k.start());
                self.out.push((at, "expected 'except' or 'finally' block".into()));
            }
            walk_stmt(self, stmt);
        }
    }
    let mut tries = Tries { tokens, out: Vec::new() };
    if let Mod::Module(m) = module {
        tries.visit_body(&m.body);
    }
    tries.out
}
