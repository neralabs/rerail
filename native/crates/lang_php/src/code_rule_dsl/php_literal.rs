//! PHP's string literals and comments as its lexer reads them: where one starting at a byte
//! ends, past `\` escapes, `{$...}`/`${...}` interpolations and the literals nested in them, and
//! heredoc and nowdoc bodies. Blade's view finds a template's PHP code with it (`blade::view`), and
//! a PHP file's view finds where PHP mode ends (`php_view`).

/// How deeply string interpolations may nest (`"{$a["{$b}"]}"`) before the scan takes the rest of
/// the text as the innermost literal: a bound on the recursion template text drives.
const NESTING: usize = 32;

/// Where the comment or string literal at `bytes[at]` ends, `None` where none starts there. One
/// PHP would find unterminated runs to the end of `bytes`.
pub(super) fn literal_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let end = match &bytes[at..] {
        [quote @ (b'\'' | b'"' | b'`'), ..] => quoted_end(bytes, at + 1, *quote, nesting),
        [b'/', b'*', ..] => bytes[at + 2..]
            .windows(2)
            .position(|pair| pair == b"*/")
            .map_or(bytes.len(), |end| at + 2 + end + 2),
        [b'/', b'/', ..] => line_end(bytes, at + 2),
        // `#[` opens an attribute.
        [b'#', b'[', ..] => return None,
        [b'#', ..] => line_end(bytes, at + 1),
        [b'<', b'<', b'<', ..] => return doc_end(bytes, at, nesting),
        _ => return None,
    };
    Some(end)
}

/// Where a `//` or `#` comment whose text starts at `from` ends: at its line's end.
fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|byte| matches!(byte, b'\n' | b'\r'))
        .map_or(bytes.len(), |end| from + end)
}

/// Where a `close`-quoted string whose text starts at `from` ends: past `\` escapes and, in one
/// that interpolates (`"`, `` ` ``), past the code of each `{$...}` and `${...}`.
pub(super) fn quoted_end(bytes: &[u8], from: usize, close: u8, nesting: usize) -> usize {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at] == close {
            return at + 1;
        } else if close != b'\''
            && let Some(end) = interpolation_end(bytes, at, nesting)
        {
            at = end;
        } else {
            at += 1;
        }
    }
    bytes.len()
}

/// Where the interpolation (`{$...}`, `${...}`) at `bytes[at]` ends, `None` where none starts
/// there: its code runs to the `}` balancing its `{`.
fn interpolation_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let from = match bytes.get(at..at + 2)? {
        b"{$" => at + 1,
        b"${" => at + 2,
        _ => return None,
    };
    if nesting == NESTING {
        return Some(bytes.len());
    }
    let mut depth = 1usize;
    let mut index = from;
    while index < bytes.len() {
        if let Some(end) = literal_end(bytes, index, nesting + 1) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Some(bytes.len())
}

/// Whether `byte` may be in a heredoc's label.
pub(super) fn label_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Where the heredoc or nowdoc whose `<<<` is at `bytes[at]` ends, `None` where `<<<` opens none:
/// after its label, closing the first line whose text, past spaces and tabs, is the label and no
/// further label byte (PHP 7.3's closing marker, which code may follow). A heredoc's body
/// interpolates; a nowdoc's (`<<<'LABEL'`) does not.
pub(super) fn doc_end(bytes: &[u8], at: usize, nesting: usize) -> Option<usize> {
    let blank = |from: usize| {
        from + bytes[from..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
    };
    let mut index = blank(at + 3);
    let quote = match bytes.get(index) {
        Some(&quote @ (b'\'' | b'"')) => {
            index += 1;
            Some(quote)
        }
        _ => None,
    };
    let start = index;
    index += bytes[index..]
        .iter()
        .take_while(|byte| label_byte(**byte))
        .count();
    let label = &bytes[start..index];
    if label.first().is_none_or(u8::is_ascii_digit) {
        return None;
    }
    if let Some(quote) = quote {
        if bytes.get(index) != Some(&quote) {
            return None;
        }
        index += 1;
    }
    index += match bytes.get(index..) {
        Some([b'\r', b'\n', ..]) => 2,
        Some([b'\n' | b'\r', ..]) => 1,
        _ => return None,
    };
    let interpolates = quote != Some(b'\'');
    let mut line = true;
    while index < bytes.len() {
        if line {
            line = false;
            let indent = blank(index);
            let after = indent + label.len();
            if bytes.get(indent..after) == Some(label)
                && !bytes.get(after).copied().is_some_and(label_byte)
            {
                return Some(after);
            }
        }
        if bytes[index] == b'\n' {
            line = true;
            index += 1;
        } else if !interpolates {
            index += 1;
        } else if bytes[index] == b'\\' && bytes.get(index + 1) != Some(&b'\n') {
            // `\` escapes in a heredoc, but not the line end.
            index += 2;
        } else if let Some(end) = interpolation_end(bytes, index, nesting) {
            index = end;
        } else {
            index += 1;
        }
    }
    Some(bytes.len())
}
