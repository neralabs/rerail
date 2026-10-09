//! Forte's `Parser\Directives\ArgumentScanner`, the part the tree builder calls
//! (`countArguments`, through `DirectiveHelper::countDirectiveArgs`). Its other methods serve the
//! AST API and are not ported here.

use super::super::scan::{skip_heredoc, skip_quoted_string};

/// `countArguments`: the top-level arguments of an argument string (without its outer
/// parentheses): one more than its commas outside brackets, strings and heredocs.
pub(crate) fn count_arguments(args: &[u8]) -> usize {
    let len = args.len();
    if len == 0 {
        return 0;
    }
    let mut pos = 0;
    skip_whitespace(args, len, &mut pos);
    if pos >= len {
        return 0;
    }
    let mut count = 1;
    // PHP's int; brackets may close more than they open.
    let mut depth: i64 = 0;
    while pos < len {
        let byte = args[pos];
        if byte == b'"' || byte == b'\'' {
            pos += 1;
            skip_quoted_string(args, &mut pos, len, byte);
            continue;
        }
        if is_heredoc_start(args, pos, len) {
            pos += 3;
            skip_heredoc(args, &mut pos, len);
            continue;
        }
        if matches!(byte, b'(' | b'[' | b'{') {
            depth += 1;
            pos += 1;
            continue;
        }
        if matches!(byte, b')' | b']' | b'}') {
            depth -= 1;
            pos += 1;
            continue;
        }
        if byte == b',' && depth == 0 {
            count += 1;
        }
        pos += 1;
    }
    count
}

/// `skipWhitespace`.
fn skip_whitespace(text: &[u8], len: usize, pos: &mut usize) {
    while *pos < len && is_whitespace(text[*pos]) {
        *pos += 1;
    }
}

/// `isWhitespace`: space, tab, `\n`, `\r`.
fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// `isHeredocStart`: `<<<` at `pos`.
fn is_heredoc_start(args: &[u8], pos: usize, len: usize) -> bool {
    pos + 2 < len && args[pos] == b'<' && args[pos + 1] == b'<' && args[pos + 2] == b'<'
}
