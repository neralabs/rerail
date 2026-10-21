//! Constant propagation for trusted frame SELECTs. A capture column can be absent physically
//! until its first nonconstant binding. Ambiguous aliases, stars, output and native ABI columns
//! remain intact. This experiment deliberately refuses SQL outside the recognized contracts.
use super::{graph::Graph, lex};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
static BIND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:FROM|JOIN) ([A-Za-z_][A-Za-z0-9_]*) (?:AS )?([A-Za-z_][A-Za-z0-9_]*)\b")
        .unwrap()
});
static SUB_ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\)\s+(?:AS\s+)?([a-z][A-Za-z0-9_]*)\b").unwrap());
static REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\.(s[0-9]+[abhnr]?)\b").unwrap());
static KEY_READ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bSELECT (?:file\s*\*\s*4294967296\s*\+\s*node|file,\s*node|k|file|node|count\(\*\)) FROM (q[0-9]+)(?:\)| ORDER BY file, node\s*(?:\)|$)|\s*$)").unwrap()
});
static NULLABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bLEFT(?: OUTER)? JOIN ([A-Za-z_][A-Za-z0-9_]*) (?:AS )?([a-z][A-Za-z0-9_]*)\b")
        .unwrap()
});
fn nullable(body: &str) -> BTreeSet<String> {
    NULLABLE
        .captures_iter(body)
        .map(|c| c[2].to_owned())
        .collect()
}
fn aliases(body: &str) -> BTreeMap<String, Option<usize>> {
    let mut aliases = BTreeMap::new();
    for c in BIND.captures_iter(body) {
        let alias = c[2].to_owned();
        if alias.to_ascii_uppercase() == alias {
            continue;
        }
        let id = c[1].strip_prefix('q').and_then(|s| s.parse().ok());
        aliases
            .entry(alias)
            .and_modify(|old| {
                if *old != id {
                    *old = None;
                }
            })
            .or_insert(id);
    }
    for c in SUB_ALIAS.captures_iter(body) {
        aliases.insert(c[1].to_owned(), None);
    }
    aliases
}
fn quoted(body: &str) -> Vec<bool> {
    let bytes = body.as_bytes();
    let mut mask = vec![false; bytes.len()];
    let mut quote = 0;
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if quote != 0 {
            mask[at] = true;
            if c == quote {
                if bytes.get(at + 1) == Some(&quote) {
                    at += 1;
                    mask[at] = true;
                } else {
                    quote = 0;
                }
            }
        } else if matches!(c, b'\'' | b'"' | b'`') {
            quote = c;
            mask[at] = true;
        }
        at += 1;
    }
    mask
}
fn named_projections(body: &str, mask: &[bool]) -> BTreeSet<usize> {
    static SELECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bSELECT\s+").unwrap());
    let mut names = BTreeSet::new();
    for found in SELECT.find_iter(body) {
        if mask[found.start()] {
            continue;
        }
        let start = found.end();
        let suffix = &body[start..];
        let end = lex::top(suffix)
            .into_iter()
            .find(|&i| {
                suffix[i..].starts_with("FROM ")
                    || suffix[i..].starts_with("WHERE ")
                    || suffix[i..].starts_with("UNION ")
            })
            .unwrap_or(suffix.len());
        for expression in lex::arguments(&suffix[..end]) {
            let expression = expression
                .strip_prefix("DISTINCT ")
                .unwrap_or(expression)
                .trim();
            if let Some(reference) = REF.find(expression)
                && reference.start() == 0
                && reference.end() == expression.len()
            {
                names.insert(expression.as_ptr() as usize - body.as_ptr() as usize);
            }
        }
    }
    names
}
fn substitute(body: &str, constants: &[BTreeMap<String, String>]) -> (String, usize) {
    let aliases = aliases(body);
    let nullable = nullable(body);
    let mask = quoted(body);
    let names = named_projections(body, &mask);
    let mut count = 0;
    let result = REF.replace_all(body, |c: &regex::Captures<'_>| {
        if !mask[c.get(0).unwrap().start()]
            && let Some(value) = aliases
                .get(&c[1])
                .copied()
                .flatten()
                .and_then(|id| constants.get(id))
                .and_then(|cols| cols.get(&c[2]))
        {
            if value != "NULL" && nullable.contains(&c[1]) {
                return c[0].to_owned();
            }
            count += 1;
            return if names.contains(&c.get(0).unwrap().start()) {
                format!("{value} AS {}", &c[2])
            } else {
                value.clone()
            };
        }
        c[0].to_owned()
    });
    (result.into_owned(), count)
}
fn select_range(body: &str) -> Option<(usize, usize)> {
    let top = lex::top(body);
    let start = top
        .iter()
        .copied()
        .find(|&i| body[i..].starts_with("SELECT "))?
        + 7;
    let end = top
        .iter()
        .copied()
        .find(|&i| {
            i >= start
                && (body[i..].starts_with("FROM ")
                    || body[i..].starts_with("WHERE ")
                    || body[i..].starts_with("UNION "))
        })
        .unwrap_or(body.len());
    Some((start, end))
}
fn matching(body: &str, open: usize) -> Option<usize> {
    let mask = quoted(body);
    let mut depth = 0;
    for (at, c) in body.as_bytes().iter().copied().enumerate().skip(open) {
        if mask[at] {
            continue;
        }
        if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(at);
            }
        }
    }
    None
}
fn literal(expr: &str) -> Option<String> {
    let mut s = expr.trim();
    loop {
        if let Some(inner) = s.strip_prefix('+') {
            s = inner.trim();
        } else if s.starts_with('(') && matching(s, 0) == Some(s.len() - 1) {
            s = s[1..s.len() - 1].trim();
        } else {
            break;
        }
    }
    if let Some((before, _)) = s.split_once(" AS ")
        && !s.starts_with("CAST(")
    {
        return literal(before);
    }
    if s.starts_with("CAST(") && s.ends_with(')') {
        return s[5..s.len() - 1]
            .rsplit_once(" AS ")
            .and_then(|(inner, _)| literal(inner));
    }
    matches!(s, "NULL" | "0").then(|| s.to_owned())
}
fn projection(body: &str) -> Option<Vec<Option<String>>> {
    // A top-level union needs agreement across every arm. Refuse it instead of inferring
    // constants from the first arm. Nested unions inside payloads are handled the same way.
    if lex::top(body)
        .into_iter()
        .any(|i| body[i..].starts_with("UNION "))
    {
        return None;
    }
    let (start, end) = select_range(body)?;
    let mut expressions: Vec<_> = lex::arguments(&body[start..end])
        .into_iter()
        .map(str::to_owned)
        .collect();
    if body.starts_with("WITH payload(") {
        let open = body.find('(')?;
        let columns_end = matching(body, open)?;
        let cols = lex::arguments(&body[open + 1..columns_end]);
        let payload_open = body[columns_end + 1..].find('(')? + columns_end + 1;
        let payload_end = matching(body, payload_open)?;
        let values = projection(&body[payload_open + 1..payload_end])?;
        if cols.len() != values.len() {
            return None;
        }
        for expression in &mut expressions {
            for (column, value) in cols.iter().zip(&values) {
                if let Some(value) = value {
                    // The outer projection is the emitter's CAST/affinity wrapper over p.
                    let pattern = format!("p.{column}");
                    if expression.trim() == pattern
                        || expression.contains(&format!("({pattern})"))
                        || expression.contains(&format!("({pattern} AS "))
                    {
                        *expression = expression.replace(&pattern, value);
                    }
                }
            }
        }
    }
    Some(expressions.iter().map(|s| literal(s)).collect())
}
pub(in super::super) fn apply(graph: &mut Graph, outputs: &[(usize, usize)]) -> Value {
    if graph
        .relations
        .iter()
        .any(|r| r.body.contains("RIGHT JOIN ") || r.body.contains("FULL JOIN "))
    {
        return json!({"refused":"unsupported null-extension shape"});
    }
    let mut constants = Vec::<BTreeMap<String, String>>::new();
    let mut substitutions = 0;
    for relation in &mut graph.relations {
        let (body, count) = substitute(&relation.body, &constants);
        substitutions += count;
        relation.body = body;
        let values = projection(&relation.body).unwrap_or_default();
        let cols = if values.len() == relation.columns.len() {
            relation
                .columns
                .iter()
                .zip(values)
                .filter_map(|(c, v)| v.map(|v| (c.name.clone(), v)))
                .collect()
        } else {
            BTreeMap::new()
        };
        constants.push(cols);
    }
    let protected: BTreeSet<_> = outputs
        .iter()
        .map(|o| o.1)
        .chain(graph.handles.iter().map(|h| h.1))
        .collect();
    let mut removable = vec![true; graph.relations.len()];
    for relation in &graph.relations {
        let a = aliases(&relation.body);
        let nullable = nullable(&relation.body);
        for (alias, dep) in &a {
            if let Some(dep) = dep
                && nullable.contains(alias)
                && constants[*dep].values().any(|v| v != "NULL")
            {
                removable[*dep] = false;
            }
        }
        for dep in graph.dependencies(&relation.body) {
            let name = Graph::name(dep);
            let references = lex::identifiers(&relation.body)
                .iter()
                .filter(|&&w| w == name)
                .count();
            let bindings = BIND
                .captures_iter(&relation.body)
                .filter(|c| c[1] == name && a.get(&c[2]) == Some(&Some(dep)))
                .count();
            let key_reads = KEY_READ
                .captures_iter(&relation.body)
                .filter(|c| c[1] == name)
                .count();
            if references != bindings + key_reads
                || relation.body.contains('*') && relation.body.contains(".*")
            {
                removable[dep] = false;
            }
        }
    }
    let mut dropped = 0;
    let mut relations = 0;
    for (id, relation) in graph.relations.iter_mut().enumerate() {
        if protected.contains(&id) || !removable[id] {
            continue;
        }
        let Some((start, end)) = select_range(&relation.body) else {
            continue;
        };
        if lex::top(&relation.body)
            .into_iter()
            .any(|i| relation.body[i..].starts_with("UNION "))
        {
            continue;
        }
        let expressions = lex::arguments(&relation.body[start..end]);
        if expressions.len() != relation.columns.len() {
            continue;
        }
        let keep: Vec<_> = relation
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.name.starts_with('s') || !constants[id].contains_key(&c.name))
            .map(|(i, _)| i)
            .collect();
        if keep.len() == relation.columns.len() {
            continue;
        }
        let projected = keep
            .iter()
            .map(|&i| expressions[i])
            .collect::<Vec<_>>()
            .join(",");
        dropped += relation.columns.len() - keep.len();
        relations += 1;
        relation
            .body
            .replace_range(start..end, &format!("{projected} "));
        relation.columns = keep
            .into_iter()
            .map(|i| relation.columns[i].clone())
            .collect();
    }
    json!({"literalReferences":substitutions,"captureColumnsRemoved":dropped,"relationsNarrowed":relations})
}
