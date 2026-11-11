//! The shape of tree-sitter-python's grammar (`grammar.json`) as the facts and the planner read it: the kinds a
//! field's node can be, whether a kind is a leaf, whether a kind holds one node in a field.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// tree-sitter-python's grammar (`grammar.json`), parsed once.
fn python_grammar() -> Option<&'static serde_json::Value> {
    static GRAMMAR: std::sync::LazyLock<Option<serde_json::Value>> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../../grammars/tree-sitter-python/src/grammar.json")).ok());
    GRAMMAR.as_ref()
}

/// The named kinds a grammar expression's own nodes can be: its symbols, a hidden, inlined or supertype rule read
/// through, named aliases. `inside`: every named node under it as well, not only its own.
fn named_kinds(grammar: &serde_json::Value, e: &serde_json::Value, inside: bool, through: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
    let names = |key: &str| grammar[key].as_array().into_iter().flatten().any(|v| v.as_str() == e["name"].as_str());
    match e["type"].as_str() {
        Some("SYMBOL") => {
            let Some(n) = e["name"].as_str() else { return };
            if n.starts_with('_') || names("inline") || names("supertypes") {
                if through.insert(n.to_owned()) && let Some(rule) = grammar["rules"].get(n) {
                    named_kinds(grammar, rule, inside, through, out);
                }
            } else {
                out.insert(n.to_owned());
                if inside && through.insert(n.to_owned()) && let Some(rule) = grammar["rules"].get(n) {
                    named_kinds(grammar, rule, inside, through, out);
                }
            }
        }
        Some("ALIAS") => {
            if e["named"] == true && let Some(v) = e["value"].as_str() {
                out.insert(v.to_owned());
            }
            if inside {
                named_kinds(grammar, &e["content"], inside, through, out);
            }
        }
        _ => {
            if let Some(c) = e.get("content") {
                named_kinds(grammar, c, inside, through, out);
            }
            for m in e["members"].as_array().into_iter().flatten() {
                named_kinds(grammar, m, inside, through, out);
            }
        }
    }
}

/// The kinds a node in field `field` can be, by tree-sitter-python's grammar, whatever its parent: every rule's
/// `field`'s content's own named nodes. `None` without the grammar, or for a field it does not name.
pub(crate) fn field_kinds(field: &str) -> Option<BTreeSet<String>> {
    static KEPT: std::sync::LazyLock<Mutex<HashMap<String, Option<BTreeSet<String>>>>> = std::sync::LazyLock::new(Default::default);
    if let Some(k) = KEPT.lock().unwrap_or_else(|p| p.into_inner()).get(field) {
        return k.clone();
    }
    let grammar = python_grammar()?;
    fn fields(grammar: &serde_json::Value, e: &serde_json::Value, field: &str, found: &mut bool, out: &mut BTreeSet<String>) {
        if e["type"] == "FIELD" && e["name"] == field {
            *found = true;
            named_kinds(grammar, &e["content"], false, &mut BTreeSet::new(), out);
        }
        if let Some(c) = e.get("content") {
            fields(grammar, c, field, found, out);
        }
        for m in e["members"].as_array().into_iter().flatten() {
            fields(grammar, m, field, found, out);
        }
    }
    let (mut found, mut out) = (false, BTreeSet::new());
    for (_, rule) in grammar["rules"].as_object().into_iter().flatten() {
        fields(grammar, rule, field, &mut found, &mut out);
    }
    let kinds = found.then_some(out);
    KEPT.lock().unwrap_or_else(|p| p.into_inner()).insert(field.to_owned(), kinds.clone());
    kinds
}

/// Whether a node of kind `kind` has no named node under it, by tree-sitter-python's grammar (a token, an external,
/// a rule of anonymous tokens only): its text is a token's. `false` without the grammar.
pub(crate) fn leaf_kind(kind: &str) -> bool {
    let Some(grammar) = python_grammar() else { return false };
    let Some(rule) = grammar["rules"].get(kind) else { return true };
    let mut out = BTreeSet::new();
    named_kinds(grammar, rule, false, &mut BTreeSet::new(), &mut out);
    out.is_empty()
}

/// Whether a node of kind `kind` holds at most one node in field `field`, by tree-sitter-python's grammar: the field
/// once in the kind's rule (its hidden and inlined rules read through), under no repeat. A kind some rule aliases a
/// node to, or a rule that reads itself, is taken to hold several (so its parts stay joins).
pub(crate) fn single_field(kind: &str, field: &str) -> bool {
    use serde_json::Value;
    static GRAMMAR: std::sync::LazyLock<Option<Value>> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../../grammars/tree-sitter-python/src/grammar.json")).ok());
    static KEPT: std::sync::LazyLock<Mutex<HashMap<(String, String), bool>>> = std::sync::LazyLock::new(Default::default);
    let Some(grammar) = GRAMMAR.as_ref() else { return false };
    if let Some(&kept) = KEPT.lock().unwrap_or_else(|p| p.into_inner()).get(&(kind.to_owned(), field.to_owned())) {
        return kept;
    }
    let rules = &grammar["rules"];
    let names = |key: &str| -> BTreeSet<String> { grammar[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect() };
    let (inline, supertypes) = (names("inline"), names("supertypes"));
    fn aliased(e: &Value, out: &mut BTreeSet<String>) {
        if e["type"] == "ALIAS" && e["named"] == true {
            if let Some(v) = e["value"].as_str() {
                out.insert(v.to_owned());
            }
        }
        if let Some(c) = e.get("content") {
            aliased(c, out);
        }
        e["members"].as_array().into_iter().flatten().for_each(|m| aliased(m, out));
    }
    let mut aliases = BTreeSet::new();
    rules.as_object().into_iter().flatten().for_each(|(_, r)| aliased(r, &mut aliases));
    // At most `MANY`: 0, 1 or several of the field.
    const MANY: u8 = 2;
    fn count(e: &Value, field: &str, rules: &Value, inline: &BTreeSet<String>, supertypes: &BTreeSet<String>, seen: &mut Vec<String>) -> u8 {
        let mut c = |e: &Value, seen: &mut Vec<String>| count(e, field, rules, inline, supertypes, seen);
        match e["type"].as_str().unwrap_or("") {
            "FIELD" => (u8::from(e["name"] == field) + c(&e["content"], seen)).min(MANY),
            "SEQ" => e["members"].as_array().into_iter().flatten().map(|m| c(m, seen)).fold(0, |a, b| (a + b).min(MANY)),
            "CHOICE" => e["members"].as_array().into_iter().flatten().map(|m| c(m, seen)).max().unwrap_or(0),
            "REPEAT" | "REPEAT1" => {
                if c(&e["content"], seen) > 0 { MANY } else { 0 }
            }
            "ALIAS" if e["named"] == true => 0,
            "SYMBOL" => {
                let name = e["name"].as_str().unwrap_or("");
                if supertypes.contains(name) || !(name.starts_with('_') || inline.contains(name)) || rules.get(name).is_none() {
                    return 0;
                }
                if seen.iter().any(|s| s == name) {
                    return MANY;
                }
                seen.push(name.to_owned());
                let n = c(&rules[name], seen);
                seen.pop();
                n
            }
            _ => e.get("content").map_or(0, |content| c(content, seen)),
        }
    }
    let single = !kind.starts_with('_') && !aliases.contains(kind) && rules.get(kind).is_some_and(|rule| count(rule, field, rules, &inline, &supertypes, &mut Vec::new()) <= 1);
    KEPT.lock().unwrap_or_else(|p| p.into_inner()).insert((kind.to_owned(), field.to_owned()), single);
    single
}
