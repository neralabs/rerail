//! Conservative lexical admission. SQLite validates the emitted SQL; unknown shapes stay on the owner.
use std::collections::BTreeSet;
#[derive(Clone)]
enum Token {
    Word(String),
    Open,
    Close,
    Comma,
    Other,
}
pub fn read(sql: &str) -> Option<BTreeSet<String>> {
    let b = sql.as_bytes();
    let mut at = 0;
    let mut tokens = vec![];
    while at < b.len() {
        match b[at] {
            b'\'' => {
                at += 1;
                while at < b.len() {
                    if b[at] == b'\'' {
                        at += 1;
                        if b.get(at) != Some(&b'\'') {
                            break;
                        }
                    }
                    at += 1;
                }
                tokens.push(Token::Other);
            }
            b'-' if b.get(at + 1) == Some(&b'-') => {
                while at < b.len() && b[at] != b'\n' {
                    at += 1;
                }
            }
            b'/' if b.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < b.len() && &b[at..at + 2] != b"*/" {
                    at += 1;
                }
                at = (at + 2).min(b.len());
            }
            b'"' | b'`' | b'[' => {
                let end = if b[at] == b'[' { b']' } else { b[at] };
                at += 1;
                let mut name = vec![];
                while at < b.len() {
                    if b[at] == end {
                        at += 1;
                        if b.get(at) != Some(&end) {
                            break;
                        }
                    }
                    name.push(b[at]);
                    at += 1;
                }
                tokens.push(Token::Word(String::from_utf8(name).ok()?));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = at;
                at += 1;
                while at < b.len() && (b[at].is_ascii_alphanumeric() || b[at] == b'_') {
                    at += 1;
                }
                tokens.push(Token::Word(sql[start..at].into()));
            }
            b'(' => {
                at += 1;
                tokens.push(Token::Open);
            }
            b')' => {
                at += 1;
                tokens.push(Token::Close);
            }
            b',' => {
                at += 1;
                tokens.push(Token::Comma);
            }
            c if c.is_ascii_whitespace() => at += 1,
            _ => {
                at += 1;
                tokens.push(Token::Other);
            }
        }
    }
    let mut reads = BTreeSet::new();
    let mut from = vec![false];
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Open => {
                from.push(false);
                continue;
            }
            Token::Close => {
                if from.len() > 1 {
                    from.pop();
                }
                continue;
            }
            Token::Comma if *from.last().unwrap() => {
                if let Some(Token::Word(name)) = tokens.get(i + 1) {
                    reads.insert(name.clone());
                }
                continue;
            }
            _ => {}
        }
        let Token::Word(word) = token else {
            continue;
        };
        if matches!(
            word.to_ascii_lowercase().as_str(),
            "select"
                | "where"
                | "group"
                | "order"
                | "having"
                | "union"
                | "except"
                | "intersect"
                | "limit"
        ) {
            *from.last_mut().unwrap() = false;
        }
        if word.eq_ignore_ascii_case("with") {
            return None;
        }
        if word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("join") {
            *from.last_mut().unwrap() = true;
            if let Some(Token::Word(name)) = tokens.get(i + 1) {
                if matches!(tokens.get(i + 2), Some(Token::Open)) {
                    return None;
                }
                reads.insert(name.clone());
            }
        }
        if matches!(tokens.get(i + 1), Some(Token::Open))
            && !matches!(
                word.to_ascii_lowercase().as_str(),
                "in" | "exists"
                    | "select"
                    | "values"
                    | "from"
                    | "on"
                    | "over"
                    | "filter"
                    | "where"
                    | "having"
                    | "as"
                    | "and"
                    | "or"
                    | "not"
                    | "when"
                    | "then"
                    | "else"
            )
            && !super::super::super::work_graph::worker::pure_builtin(word)
        {
            return None;
        }
    }
    Some(reads)
}
