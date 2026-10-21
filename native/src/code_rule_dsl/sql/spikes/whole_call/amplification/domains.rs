//! Finite abstract interpretation of demanded node kinds. Unknown means all kinds, not an
//! empty set. Joins only widen; recursive unit references participate in the same fixed point.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
type Kinds = Option<BTreeSet<u16>>;
fn kinds(v: &Value) -> Kinds {
    v.as_array()
        .map(|a| a.iter().map(|v| v.as_u64().unwrap() as u16).collect())
}
fn meet(a: &Kinds, b: &Kinds) -> Kinds {
    match (a, b) {
        (None, b) | (b, None) => b.clone(),
        (Some(a), Some(b)) => Some(a.intersection(b).copied().collect()),
    }
}
fn join(to: &mut Option<Kinds>, value: &Kinds) -> bool {
    if value.as_ref().is_some_and(BTreeSet::is_empty) {
        return false;
    }
    match to {
        None => {
            *to = Some(value.clone());
            true
        }
        Some(None) => false,
        Some(current @ Some(_)) => match value {
            None => {
                *current = None;
                true
            }
            Some(value) => {
                let set = current.as_mut().unwrap();
                let old = set.len();
                set.extend(value);
                old != set.len()
            }
        },
    }
}
pub(in super::super) fn analyze(
    code: &[Value],
    roots: &[(usize, Kinds)],
) -> BTreeMap<(String, usize), Kinds> {
    let mut demanded = vec![None; code.len()];
    let mut queue = VecDeque::new();
    for (entry, kinds) in roots {
        if join(&mut demanded[*entry], kinds) {
            queue.push_back(*entry);
        }
    }
    while let Some(id) = queue.pop_front() {
        let domain = demanded[id].clone().unwrap();
        let p = &code[id];
        let mut edges = Vec::new();
        let children: Vec<_> = p["kids"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        match p["tag"].as_str().unwrap() {
            "all" | "seq" => {
                let mut current = domain;
                for child in children {
                    edges.push((child, current.clone()));
                    if code[child]["tag"] == "kind" {
                        current = meet(&current, &kinds(&code[child]["kinds"]));
                    }
                    if code[child]["tag"] == "never" {
                        current = Some(BTreeSet::new());
                    }
                }
            }
            "walk" | "count" => {
                for child in children {
                    edges.push((child, kinds(&p["need"])));
                }
            }
            "recur" => edges.push((p["target"].as_u64().unwrap() as usize, domain)),
            _ => {
                for child in children {
                    edges.push((child, domain.clone()));
                }
            }
        }
        for (child, domain) in edges {
            if join(&mut demanded[child], &domain) {
                queue.push_back(child);
            }
        }
    }
    let mut terminals = BTreeMap::<(String, usize), Option<Kinds>>::new();
    for (id, p) in code.iter().enumerate() {
        if let Some(term) = p["term"].as_u64() {
            terminals
                .entry((p["tag"].as_str().unwrap().to_owned(), term as usize))
                .or_insert_with(|| Some(Some(BTreeSet::new())));
        }
        if let (Some(term), Some(domain)) = (p["term"].as_u64(), &demanded[id]) {
            let key = (p["tag"].as_str().unwrap().to_owned(), term as usize);
            join(terminals.entry(key).or_default(), domain);
        }
    }
    terminals
        .into_iter()
        .map(|(k, v)| (k, v.unwrap_or_else(|| Some(BTreeSet::new()))))
        .collect()
}
pub(in super::super) fn filter(
    domains: &BTreeMap<(String, usize), Kinds>,
    tag: &str,
    term: usize,
) -> String {
    match domains.get(&(tag.to_owned(), term)) {
        None | Some(None) => String::new(),
        Some(Some(kinds)) if kinds.is_empty() => " WHERE 0".into(),
        Some(Some(kinds)) => format!(
            " WHERE kind IN({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
