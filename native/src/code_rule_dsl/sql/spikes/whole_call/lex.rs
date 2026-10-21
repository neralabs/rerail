//! A narrow lexical adapter for trusted compiler output, not a general SQL parser.
use std::collections::HashMap;

pub(super) fn top(text: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let (mut depth, mut quote, mut at) = (0usize, 0u8, 0usize);
    let mut result = Vec::new();
    while at < b.len() {
        let c = b[at];
        if quote != 0 {
            if c == quote {
                if b.get(at + 1) == Some(&quote) {
                    at += 2;
                    continue;
                }
                quote = 0;
            }
        } else {
            match c {
                b'\'' | b'"' | b'`' => quote = c,
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                _ if depth == 0 => result.push(at),
                _ => {}
            }
        }
        at += 1;
    }
    result
}
pub(super) fn arguments(text: &str) -> Vec<&str> {
    let mut start = 0;
    let mut out = Vec::new();
    for at in top(text)
        .into_iter()
        .filter(|&at| text.as_bytes()[at] == b',')
    {
        out.push(text[start..at].trim());
        start = at + 1;
    }
    out.push(text[start..].trim());
    out
}
pub(super) fn replace(text: &str, names: &HashMap<String, String>) -> String {
    let b = text.as_bytes();
    let mut out = String::new();
    let mut at = 0;
    while at < b.len() {
        let start = at;
        if matches!(b[at], b'\'' | b'"' | b'`') {
            let quote = b[at];
            at += 1;
            while at < b.len() {
                if b[at] == quote {
                    at += 1;
                    if b.get(at) == Some(&quote) {
                        at += 1;
                        continue;
                    }
                    break;
                }
                at += 1;
            }
            out.push_str(&text[start..at]);
        } else if b[at].is_ascii_alphabetic() || b[at] == b'_' {
            at += 1;
            while at < b.len() && (b[at].is_ascii_alphanumeric() || b[at] == b'_') {
                at += 1;
            }
            let word = &text[start..at];
            out.push_str(names.get(word).map_or(word, String::as_str));
        } else {
            // Copy a complete UTF-8 character, including literal comments from lowerings.
            at += text[at..].chars().next().unwrap().len_utf8();
            out.push_str(&text[start..at]);
        }
    }
    out
}

pub(super) fn identifiers(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut at = 0;
    let mut out = Vec::new();
    while at < b.len() {
        if matches!(b[at], b'\'' | b'"' | b'`') {
            let quote = b[at];
            at += 1;
            while at < b.len() {
                if b[at] == quote {
                    at += 1;
                    if b.get(at) == Some(&quote) {
                        at += 1;
                        continue;
                    }
                    break;
                }
                at += 1;
            }
        } else if b[at].is_ascii_alphabetic() || b[at] == b'_' {
            let start = at;
            at += 1;
            while at < b.len() && (b[at].is_ascii_alphanumeric() || b[at] == b'_') {
                at += 1;
            }
            out.push(&text[start..at]);
        } else {
            at += text[at..].chars().next().unwrap().len_utf8();
        }
    }
    out
}
