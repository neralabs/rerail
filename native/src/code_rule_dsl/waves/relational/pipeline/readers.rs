//! Conservative SQL relation-use counts after the authorizer establishes dependencies.
pub(super) fn occurrences(query: &str, name: &str) -> usize {
    static IDENTIFIER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("SQL identifiers")
    });
    let bytes = query.as_bytes();
    let mut masked = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"--") || bytes[i..].starts_with(b"/*") {
            let line = bytes[i..].starts_with(b"--");
            let start = i;
            i += 2;
            while i < bytes.len()
                && if line {
                    bytes[i] != b'\n'
                } else {
                    !bytes[i..].starts_with(b"*/")
                }
            {
                i += 1;
            }
            if !line {
                i = (i + 2).min(bytes.len());
            }
            masked[start..i].fill(b' ');
        } else if matches!(bytes[i], b'\'' | b'"' | b'`' | b'[') {
            let start = i;
            let quote = bytes[i];
            let end = if quote == b'[' { b']' } else { quote };
            i += 1;
            while i < bytes.len() {
                if bytes[i] == end {
                    i += 1;
                    if i < bytes.len() && bytes[i] == end && end != b']' {
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if quote == b'\'' {
                // SQLite accepts single-quoted table identifiers. Keep possible
                // relation positions conservatively, including parenthesized joins.
                let prefix = &masked[..start];
                let last = prefix.iter().rposition(|b| !b.is_ascii_whitespace());
                let relation = last.is_some_and(|last| {
                    if matches!(prefix[last], b',' | b'(' | b'.') {
                        return true;
                    }
                    let begin = prefix[..=last]
                        .iter()
                        .rposition(|b| !b.is_ascii_alphanumeric() && *b != b'_')
                        .map_or(0, |n| n + 1);
                    prefix[begin..=last].eq_ignore_ascii_case(b"FROM")
                        || prefix[begin..=last].eq_ignore_ascii_case(b"JOIN")
                });
                if !relation {
                    masked[start..i].fill(b' ');
                }
            }
        } else {
            i += 1;
        }
    }
    let masked = std::str::from_utf8(&masked).expect("mask preserves UTF-8");
    IDENTIFIER
        .find_iter(masked)
        .filter(|token| token.as_str().eq_ignore_ascii_case(name))
        .filter(|token| {
            let tail = masked[token.end()..].trim_start();
            let tail = tail
                .strip_prefix(['"', '`', ']'])
                .unwrap_or(tail)
                .trim_start();
            !tail.starts_with('.')
        })
        .count()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_qualifiers_and_literal_mentions_do_not_invent_second_readers() {
        assert_eq!(
            occurrences("SELECT facts.a,facts.b FROM facts WHERE facts.c=1", "facts"),
            1
        );
        assert_eq!(
            occurrences(
                "SELECT 'facts' label,f.a FROM facts f /* facts */ -- facts\nWHERE f.a='it''s facts'",
                "facts"
            ),
            1
        );
        assert_eq!(
            occurrences("SELECT \"facts\".a,[facts].b FROM \"facts\"", "facts"),
            1
        );
    }
    #[test]
    fn real_repeated_relations_keep_their_index_boundary() {
        assert_eq!(
            occurrences("SELECT a.x FROM facts a JOIN facts b ON a.x=b.x", "facts"),
            2
        );
        assert_eq!(
            occurrences(
                "SELECT a.x FROM 'facts' a JOIN 'facts' b ON a.x=b.x",
                "facts"
            ),
            2
        );
        assert_eq!(
            occurrences(
                "SELECT * FROM facts f WHERE EXISTS(SELECT 1 FROM facts g WHERE g.x=f.x)",
                "facts"
            ),
            2
        );
    }
}
