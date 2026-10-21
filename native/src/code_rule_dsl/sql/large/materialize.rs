//! Hoist a statement's shared producers before its data-row consumers. Correlation to a SQL
//! iteration must not turn a one-time CTE into one materialization per outer data row.
use super::*;

/// Rename table bindings, preserving their SQL aliases and column names. CTE names such
/// as `k` can also be column names; a lexical replacement of every identifier is invalid.
pub(super) fn bindings(sql: &str, names: &HashMap<String, String>) -> String {
    let words = lex::identifiers(sql);
    let mut changes = Vec::new();
    for (i, &word) in words.iter().enumerate() {
        if i == 0 || !matches!(words[i - 1], "FROM" | "JOIN") {
            continue;
        }
        let Some(name) = names.get(word) else {
            continue;
        };
        let start = word.as_ptr() as usize - sql.as_ptr() as usize;
        let end = start + word.len();
        let tail = sql[end..].trim_start();
        let next = tail
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        let alias = !next.is_empty()
            && !matches!(
                next,
                "WHERE"
                    | "ON"
                    | "USING"
                    | "LEFT"
                    | "RIGHT"
                    | "INNER"
                    | "CROSS"
                    | "JOIN"
                    | "UNION"
                    | "EXCEPT"
                    | "INTERSECT"
                    | "GROUP"
                    | "ORDER"
                    | "LIMIT"
                    | "HAVING"
                    | "WINDOW"
            );
        changes.push((
            start,
            end,
            if alias {
                name.clone()
            } else {
                format!("{name} AS {word}")
            },
        ));
    }
    let mut out = sql.to_owned();
    for (start, end, text) in changes.into_iter().rev() {
        out.replace_range(start..end, &text);
    }
    out
}

pub(super) struct Producer {
    pub name: String,
    pub columns: Vec<String>,
    pub body: String,
}
pub(super) fn split(
    sql: &str,
    schemas: &BTreeMap<String, Schema>,
) -> Option<(Vec<Producer>, String)> {
    let mut rest = sql.strip_prefix("WITH ")?;
    if rest.starts_with("RECURSIVE ") {
        return None;
    }
    let mut producers = Vec::new();
    let mut known: BTreeMap<String, Vec<String>> = BTreeMap::new();
    loop {
        rest = rest.trim_start();
        let len = rest
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        if len == 0 {
            return None;
        }
        let name = rest[..len].to_owned();
        rest = rest[len..].trim_start();
        let explicit = if rest.starts_with('(') {
            let close = closing(rest)?;
            let columns = lex::arguments(&rest[1..close])
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            rest = rest[close + 1..].trim_start();
            Some(columns)
        } else {
            None
        };
        rest = rest.strip_prefix("AS ")?.trim_start();
        if let Some(tail) = rest.strip_prefix("MATERIALIZED ") {
            rest = tail.trim_start();
        }
        if let Some(tail) = rest.strip_prefix("NOT MATERIALIZED ") {
            rest = tail.trim_start();
        }
        let close = closing(rest)?;
        let body = rest[1..close].to_owned();
        let columns = explicit.or_else(|| columns(&body, &known, schemas))?;
        known.insert(name.clone(), columns.clone());
        producers.push(Producer {
            name,
            columns,
            body,
        });
        rest = rest[close + 1..].trim_start();
        if let Some(tail) = rest.strip_prefix(',') {
            rest = tail;
        } else {
            return Some((producers, rest.to_owned()));
        }
    }
}
fn closing(sql: &str) -> Option<usize> {
    if !sql.starts_with('(') {
        return None;
    }
    let b = sql.as_bytes();
    let mut depth = 0;
    let mut quote = 0;
    let mut at = 0;
    while at < b.len() {
        if quote != 0 {
            if b[at] == quote {
                if b.get(at + 1) == Some(&quote) {
                    at += 2;
                    continue;
                }
                quote = 0;
            }
        } else {
            match b[at] {
                b'\'' | b'"' => quote = b[at],
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at);
                    }
                }
                _ => {}
            }
        }
        at += 1;
    }
    None
}

pub(super) struct Membership<'a> {
    pub start: usize,
    pub end: usize,
    pub lhs: &'a str,
    pub negated: bool,
    pub query: &'a str,
}
/// Hoist a closed membership set before scanning its consumer. Keep terminal demand IN
/// constraints intact: the virtual-table API consumes those complete batches itself.
pub(super) fn membership(sql: &str) -> Option<Membership<'_>> {
    let words = lex::identifiers(sql);
    for (i, &word) in words.iter().enumerate() {
        if word != "IN" || i == 0 {
            continue;
        }
        let negated = words[i - 1] == "NOT";
        if negated && i < 2 {
            continue;
        }
        let col = words[i - 1 - usize::from(negated)];
        if matches!(col, "ek" | "nk" | "member") {
            continue;
        }
        let word_at = word.as_ptr() as usize - sql.as_ptr() as usize;
        let lhs_end = if negated {
            words[i - 1].as_ptr() as usize - sql.as_ptr() as usize
        } else {
            word_at
        };
        let before = sql[..lhs_end].trim_end();
        let end = before.len();
        let start = if before.ends_with(')') {
            let mut depth = 1;
            let mut at = end - 1;
            while at > 0 {
                at -= 1;
                match sql.as_bytes()[at] {
                    b')' => depth += 1,
                    b'(' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 || at > 0 && sql.as_bytes()[at - 1].is_ascii_alphanumeric() {
                continue;
            }
            at
        } else {
            let mut at = col.as_ptr() as usize - sql.as_ptr() as usize;
            if at > 0 && sql.as_bytes()[at - 1] == b'.' {
                let column = i - 1 - usize::from(negated);
                if column == 0 {
                    continue;
                }
                at = words[column - 1].as_ptr() as usize - sql.as_ptr() as usize;
            }
            at
        };
        let tail = sql[word_at + 2..].trim_start();
        if !tail.starts_with("(SELECT ") {
            continue;
        }
        let Some(close) = closing(tail) else {
            continue;
        };
        let query = &tail[1..close];
        let select = query.strip_prefix("SELECT ").unwrap();
        let at = lex::top(select)
            .into_iter()
            .find(|&at| select[at..].starts_with(" FROM "))
            .unwrap_or(select.len());
        if lex::arguments(&select[..at]).len() != 1 {
            continue;
        }
        let identifiers = lex::identifiers(query);
        let mut aliases = BTreeSet::new();
        for (j, &w) in identifiers.iter().enumerate() {
            if !matches!(w, "FROM" | "JOIN") {
                continue;
            }
            let Some(&table) = identifiers.get(j + 1) else {
                continue;
            };
            aliases.insert(table);
            let Some(&next) = identifiers.get(j + 2) else {
                continue;
            };
            if next == "AS" {
                if let Some(&alias) = identifiers.get(j + 3) {
                    aliases.insert(alias);
                }
            } else if !matches!(
                next,
                "WHERE"
                    | "ON"
                    | "USING"
                    | "JOIN"
                    | "LEFT"
                    | "RIGHT"
                    | "CROSS"
                    | "INNER"
                    | "GROUP"
                    | "ORDER"
                    | "LIMIT"
                    | "UNION"
                    | "EXCEPT"
                    | "INTERSECT"
            ) {
                aliases.insert(next);
            }
        }
        if identifiers.iter().any(|&w| {
            let pos = w.as_ptr() as usize - query.as_ptr() as usize;
            query.as_bytes().get(pos + w.len()) == Some(&b'.') && !aliases.contains(w)
        }) {
            continue;
        }
        let end = tail.as_ptr() as usize - sql.as_ptr() as usize + close + 1;
        return Some(Membership {
            start,
            end,
            lhs: &sql[start..before.len()],
            negated,
            query,
        });
    }
    None
}
fn columns(
    body: &str,
    known: &BTreeMap<String, Vec<String>>,
    schemas: &BTreeMap<String, Schema>,
) -> Option<Vec<String>> {
    let select = body.strip_prefix("SELECT ")?;
    let select = select.strip_prefix("DISTINCT ").unwrap_or(select);
    let from = lex::top(select)
        .into_iter()
        .find(|&i| select[i..].starts_with(" FROM "))?;
    let fields = lex::arguments(&select[..from]);
    let mut result = Vec::new();
    for field in fields {
        if field == "*" || field.ends_with(".*") {
            let source = select[from + 6..].split_whitespace().next()?;
            result.extend(
                known
                    .get(source)
                    .or_else(|| schemas.get(source).map(|s| &s.columns))?
                    .iter()
                    .cloned(),
            );
            continue;
        }
        if let Some(at) = lex::top(field)
            .into_iter()
            .rfind(|&i| field[i..].starts_with(" AS "))
        {
            result.push(field[at + 4..].trim().into());
            continue;
        }
        let name = field.rsplit('.').next()?.trim();
        if !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'"'))
        {
            return None;
        }
        result.push(name.into());
    }
    Some(result)
}
