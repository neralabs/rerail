//! Python 3.13 diagnostic spelling over the existing parser's structured errors and tokens.
//! No rule names, paths, interpreter calls, or source-oracle results enter this formatter.
use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_parser::{InterpolatedStringErrorType as F, LexicalErrorType as L, ParseError, ParseErrorType as E};

fn line(source: &str, offset: u32) -> u32 {
    source.as_bytes()[..(offset as usize).min(source.len())].iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

fn delimiters(source: &str, error: &ParseError, tokens: &Tokens) -> Option<(u32, String)> {
    let mut opens = Vec::new();
    for token in tokens.iter() {
        let (kind, range) = token.as_tuple();
        let offset = u32::from(range.start());
        if offset > u32::from(error.location.start()) { break; }
        let (open, close) = match kind {
            TokenKind::Lpar => (Some('('), None), TokenKind::Lsqb => (Some('['), None),
            TokenKind::Lbrace => (Some('{'), None), TokenKind::Rpar => (None, Some(')')),
            TokenKind::Rsqb => (None, Some(']')), TokenKind::Rbrace => (None, Some('}')),
            _ => (None, None),
        };
        if let Some(open) = open { opens.push((open, offset)); }
        if let Some(close) = close {
            let Some((open, start)) = opens.pop() else { return Some((offset, format!("unmatched '{close}'"))); };
            if !matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}')) {
                let context = if line(source, start) == line(source, offset) { String::new() }
                              else { format!(" on line {}", line(source, start)) };
                return Some((offset, format!("closing parenthesis '{close}' does not match opening parenthesis '{open}'{context}")));
            }
        }
    }
    if matches!(error.error, E::Lexical(L::Eof)) {
        if let Some((open, offset)) = opens.last() {
            return Some((*offset, format!("'{open}' was never closed")));
        }
    }
    None
}

pub(super) fn describe(source: &str, error: &ParseError, tokens: &Tokens) -> (u32, String) {
    if let Some(message) = delimiters(source, error, tokens) { return message; }
    let offset = u32::from(error.location.start());
    let end = u32::from(error.location.end());
    let fragment = &source[(offset as usize).min(source.len())..(end as usize).min(source.len())];
    let string_error = |triple: bool| {
        let kind = if triple { "triple-quoted string" } else { "string" };
        format!("unterminated {kind} literal (detected at line {})", line(source, end.saturating_sub(1)))
    };
    let fstring = |error: &F| match error {
        F::UnclosedLbrace => "f-string: expecting '}'".to_owned(),
        F::SingleRbrace => "f-string: single '}' is not allowed".to_owned(),
        F::InvalidConversionFlag => format!("f-string: invalid conversion character '{}': expected 's', 'r', or 'a'",
                                             fragment.chars().next().unwrap_or('\0')),
        F::LambdaWithoutParentheses => "f-string: lambda expressions are not allowed without parentheses".to_owned(),
        _ => format!("f-string: {error}"),
    };
    let message = match &error.error {
        E::OtherError(message) if message.starts_with("Expected an indented block after ") => {
            let start = source[..(offset as usize).min(source.len())].lines().enumerate()
                .filter(|(_, text)| text.trim_end().ends_with(':')).last().map_or(line(source, offset), |(i, _)| i as u32 + 1);
            format!("expected{} on line {start}", message.trim_start_matches("Expected"))
        }
        E::OtherError(message) if message.starts_with("Expected ") => "invalid syntax".to_owned(),
        E::ExpectedExpression | E::UnexpectedExpressionToken | E::EmptySlice | E::EmptyGlobalNames |
        E::EmptyNonlocalNames | E::EmptyDeleteTargets | E::EmptyImportNames | E::EmptyTypeParams => "invalid syntax".to_owned(),
        E::ExpectedToken { expected: TokenKind::Colon, .. } => "expected ':'".to_owned(),
        E::ExpectedToken { expected: TokenKind::Comma, .. } => "invalid syntax. Perhaps you forgot a comma?".to_owned(),
        E::UnexpectedIndentation => "unexpected indent".to_owned(),
        E::NonDefaultParamAfterDefaultParam => "parameter without a default follows parameter with a default".to_owned(),
        E::ParamAfterVarKeywordParam => "arguments cannot follow var-keyword argument".to_owned(),
        E::InvalidAssignmentTarget if fragment.parse::<f64>().is_ok() || matches!(fragment, "None" | "True" | "False") =>
            "cannot assign to literal here. Maybe you meant '==' instead of '='?".to_owned(),
        E::Lexical(L::UnclosedStringError) => {
            let quoted = fragment.trim_start_matches(|c: char| c.is_ascii_alphabetic());
            string_error(quoted.starts_with("\"\"\"") || quoted.starts_with("'''"))
        }
        E::FStringError(error) | E::Lexical(L::FStringError(error)) => fstring(error),
        E::Lexical(L::LineContinuationError) => "unexpected character after line continuation character".to_owned(),
        E::Lexical(L::UnrecognizedToken { tok }) if tok.is_ascii() => "invalid syntax".to_owned(),
        _ => error.error.to_string(),
    };
    (offset, message)
}
