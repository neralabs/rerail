//! Forte's `Support\ScanPrimitives` and the PHP string functions the lexer and parser lean on.
//! Positions are byte offsets passed by reference as PHP's `int &$pos`. `strpos` searches the
//! whole source from an offset, past `len` when a caller passes a shorter one, as PHP's does.
//!
//! PHP's `skipTemplateLiteral` and `skipTemplateExpression` call each other once per nested
//! `${`…`` ` ``; here they share one loop over an explicit stack, so nesting costs heap, not the
//! thread's stack, and the bytes they skip are the same.

/// `strpos($haystack, $needle, $offset)`: `None` for no match, or for an offset past the end
/// (where PHP throws a `ValueError`, which no caller lets happen).
pub(crate) fn strpos(haystack: &[u8], needle: &[u8], offset: usize) -> Option<usize> {
    if offset > haystack.len() {
        return None;
    }
    if needle.len() == 1 {
        return memchr(needle[0], &haystack[offset..]).map(|at| at + offset);
    }
    haystack[offset..]
        .windows(needle.len().max(1))
        .position(|window| window == needle)
        .map(|at| at + offset)
}

/// The first occurrence of `byte`.
pub(crate) fn memchr(byte: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|b| *b == byte)
}

/// `ctype_alnum` of one byte (the C locale: ASCII letters and digits).
pub(crate) fn is_alnum(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// `ctype_alpha` of one byte.
pub(crate) fn is_alpha(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

/// `ctype_space` of one byte: space, `\t`, `\n`, `\v`, `\f`, `\r`.
pub(crate) fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The bytes PHP's `trim` strips by default: " \t\n\r\0\x0B".
pub(crate) fn is_trim(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0 | 0x0b)
}

/// `substr($source, $start, $length)` for in-range non-negative arguments, clamped as PHP 8
/// clamps them.
pub(crate) fn substr(source: &[u8], start: usize, length: usize) -> &[u8] {
    let start = start.min(source.len());
    let end = start.saturating_add(length).min(source.len());
    &source[start..end]
}

/// `findLineEnding`: the first `\n` or `\r` at or after `pos`, before `len` when given. PHP's two
/// `strpos` calls search to the end of the source; one pass that stops at `len` finds the same.
pub(crate) fn find_line_ending(source: &[u8], pos: usize, len: Option<usize>) -> Option<usize> {
    let end = len.map_or(source.len(), |len| len.min(source.len()));
    if pos >= end {
        return None;
    }
    source[pos..end]
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
        .map(|at| at + pos)
}

/// `skipLineEnding`: past one `\n`, `\r` or `\r\n`.
pub(crate) fn skip_line_ending(source: &[u8], pos: &mut usize, len: usize) {
    if *pos >= len {
        return;
    }
    match source[*pos] {
        b'\n' => *pos += 1,
        b'\r' => {
            *pos += 1;
            if *pos < len && source[*pos] == b'\n' {
                *pos += 1;
            }
        }
        _ => {}
    }
}

/// `skipQuotedString`: past the next `quote` not escaped by an odd run of backslashes.
pub(crate) fn skip_quoted_string(source: &[u8], pos: &mut usize, len: usize, quote: u8) {
    while *pos < len {
        let Some(quote_pos) = strpos(source, &[quote], *pos) else {
            *pos = len;
            return;
        };
        *pos = quote_pos;
        let mut backslashes = 0;
        let mut check = quote_pos;
        while check > 0 && source[check - 1] == b'\\' {
            backslashes += 1;
            check -= 1;
        }
        *pos += 1;
        if backslashes % 2 == 0 {
            return;
        }
    }
    *pos = len;
}

/// `skipBlockComment`: past the next `*/`.
pub(crate) fn skip_block_comment(source: &[u8], pos: &mut usize, len: usize) {
    while *pos < len {
        let Some(star) = strpos(source, b"*", *pos) else {
            *pos = len;
            return;
        };
        *pos = star + 1;
        if *pos < len && source[*pos] == b'/' {
            *pos += 1;
            return;
        }
    }
    *pos = len;
}

/// `skipLineComment`: past the line's end.
pub(crate) fn skip_line_comment(source: &[u8], pos: &mut usize, len: usize) {
    match find_line_ending(source, *pos, Some(len)) {
        None => *pos = len,
        Some(end) => {
            *pos = end;
            skip_line_ending(source, pos, len);
        }
    }
}

/// `skipLineCommentStoppingAt`: to the line's end (past it), or to `stop` (before it, `true`).
pub(crate) fn skip_line_comment_stopping_at(
    source: &[u8],
    pos: &mut usize,
    len: usize,
    stop: &[u8],
) -> bool {
    while *pos < len {
        let byte = source[*pos];
        if byte == b'\n' || byte == b'\r' {
            *pos += 1;
            return false;
        }
        if *pos + stop.len() <= len && &source[*pos..*pos + stop.len()] == stop {
            return true;
        }
        *pos += 1;
    }
    false
}

/// `skipLineCommentDetecting`: past the line's end; the offsets of each of `sequences` in the
/// comment, sequence by sequence.
pub(crate) fn skip_line_comment_detecting(
    source: &[u8],
    pos: &mut usize,
    len: usize,
    sequences: &[&[u8]],
) -> Vec<usize> {
    let mut detected = Vec::new();
    let line_end = find_line_ending(source, *pos, Some(len));
    let end = line_end.unwrap_or(len);
    for sequence in sequences {
        let mut search = *pos;
        while search + sequence.len() <= end {
            // Only a match starting before `end` counts, so the search stops there.
            let window = &source[..(end + sequence.len()).saturating_sub(1).min(source.len())];
            match strpos(window, sequence, search) {
                Some(found) if found < end => {
                    detected.push(found);
                    search = found + sequence.len();
                }
                _ => break,
            }
        }
    }
    match line_end {
        None => *pos = len,
        Some(line_end) => {
            *pos = line_end;
            skip_line_ending(source, pos, len);
        }
    }
    detected
}

/// `skipHeredoc`: from after `<<<`, past the heredoc or nowdoc and its closing delimiter.
pub(crate) fn skip_heredoc(source: &[u8], pos: &mut usize, len: usize) {
    if *pos >= len {
        return;
    }
    let nowdoc = source[*pos] == b'\'';
    if nowdoc {
        *pos += 1;
    }
    let delim_start = *pos;
    while *pos < len && (is_alnum(source[*pos]) || source[*pos] == b'_') {
        *pos += 1;
    }
    if *pos == delim_start {
        *pos = len;
        return;
    }
    let delimiter = &source[delim_start..*pos];
    if nowdoc && *pos < len && source[*pos] == b'\'' {
        *pos += 1;
    }
    let Some(line_end) = find_line_ending(source, *pos, Some(len)) else {
        *pos = len;
        return;
    };
    *pos = line_end;
    skip_line_ending(source, pos, len);
    while *pos < len {
        if *pos + delimiter.len() <= len && &source[*pos..*pos + delimiter.len()] == delimiter {
            let after = *pos + delimiter.len();
            if after >= len || matches!(source[after], b'\n' | b'\r' | b';') {
                *pos = after;
                if *pos < len && source[*pos] == b';' {
                    *pos += 1;
                }
                skip_line_ending(source, pos, len);
                return;
            }
        }
        let Some(line_end) = find_line_ending(source, *pos, Some(len)) else {
            *pos = len;
            return;
        };
        *pos = line_end;
        skip_line_ending(source, pos, len);
    }
    *pos = len;
}

/// A level of `skip_template`'s stack.
enum Template {
    /// Inside `` `…` `` (`skipTemplateLiteral`).
    Literal,
    /// Inside `${…}` at a brace depth (`skipTemplateExpression`).
    Expression(usize),
}

/// `skipTemplateLiteral` and `skipTemplateExpression`, from the given level.
fn skip_template(source: &[u8], pos: &mut usize, len: usize, start: Template) {
    let mut stack = vec![start];
    while let Some(top) = stack.last_mut() {
        match top {
            Template::Literal => {
                if *pos >= len {
                    stack.pop();
                    continue;
                }
                match source[*pos] {
                    b'`' => {
                        *pos += 1;
                        stack.pop();
                    }
                    // A dangling escape at EOF clamps to len.
                    b'\\' => *pos = len.min(*pos + 2),
                    b'$' if *pos + 1 < len && source[*pos + 1] == b'{' => {
                        *pos += 2;
                        stack.push(Template::Expression(1));
                    }
                    _ => *pos += 1,
                }
            }
            Template::Expression(depth) => {
                if *depth == 0 || *pos >= len {
                    stack.pop();
                    continue;
                }
                let byte = source[*pos];
                match byte {
                    b'{' => {
                        *depth += 1;
                        *pos += 1;
                    }
                    b'}' => {
                        *depth -= 1;
                        *pos += 1;
                    }
                    b'\'' | b'"' => {
                        *pos += 1;
                        skip_quoted_string(source, pos, len, byte);
                    }
                    b'`' => {
                        *pos += 1;
                        stack.push(Template::Literal);
                    }
                    b'/' if *pos + 1 < len => match source[*pos + 1] {
                        b'/' => {
                            *pos += 2;
                            skip_line_comment(source, pos, len);
                        }
                        b'*' => {
                            *pos += 2;
                            skip_block_comment(source, pos, len);
                        }
                        _ => *pos += 1,
                    },
                    _ => *pos += 1,
                }
            }
        }
    }
}

/// `skipTemplateLiteral`: from after the opening backtick, past the closing one.
pub(crate) fn skip_template_literal(source: &[u8], pos: &mut usize, len: usize) {
    skip_template(source, pos, len, Template::Literal);
}

/// `skipBacktickString`.
pub(crate) fn skip_backtick_string(source: &[u8], pos: &mut usize, len: usize) {
    skip_quoted_string(source, pos, len, b'`');
}
