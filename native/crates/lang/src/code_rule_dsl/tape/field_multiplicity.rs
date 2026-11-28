//! Which of a grammar's fields a node may hold more than once, from its `grammar.json`: tree-sitter's
//! `node-types.json` `multiple`, any node's. The build script mounts this as a module of its own and writes each
//! vendored tree-sitter grammar's list (`language::field_repeats`): a field no node holds twice names one child of a
//! parent (`Source::names_one_row`), so a probe of it by its parent is a lookup.
//!
//! A field is counted once for each visible node its content makes (a field over a hidden rule or a sequence tags
//! every node in it); a hidden or inline rule's fields are its parent's; a sequence adds, a choice takes the most, a
//! repeat makes any field it holds many, and so does recursion through hidden rules. Wrong only on the safe side: a
//! field counted many where no node holds it twice costs a lookup, one counted once where a node may hold it twice
//! would drop rows. A rule type it does not know makes no list (`None`): no field of the grammar is claimed single.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A count saturated at two: none, one, many.
type Count = u8;

fn add(a: Count, b: Count) -> Count {
    (a + b).min(2)
}

/// What a rule makes in the node it is part of: how many visible nodes, each field's count, and whether it reaches a
/// hidden rule it is inside (recursion through hidden rules: its fields may repeat).
#[derive(Default, Clone)]
struct Made {
    nodes: Count,
    fields: BTreeMap<String, Count>,
    recursive: bool,
}

impl Made {
    fn many(mut self) -> Self {
        if self.nodes > 0 {
            self.nodes = 2;
        }
        self.fields.values_mut().for_each(|c| *c = 2);
        self
    }
}

struct Grammar<'a> {
    rules: &'a serde_json::Map<String, Value>,
    /// Rules whose nodes are not in the tree: `_`-named ones and the `inline` list.
    hidden: BTreeSet<&'a str>,
}

impl<'a> Grammar<'a> {
    fn hidden(&self, name: &str) -> bool {
        name.starts_with('_') || self.hidden.contains(name)
    }

    /// What `rule` makes, inside the hidden rules `inside`.
    fn made(&self, rule: &'a Value, inside: &mut Vec<&'a str>) -> Option<Made> {
        let ty = rule.get("type")?.as_str()?;
        let content = || rule.get("content");
        Some(match ty {
            "BLANK" => Made::default(),
            // A token: one node (an anonymous one, or a pattern's).
            "STRING" | "PATTERN" | "TOKEN" | "IMMEDIATE_TOKEN" => Made { nodes: 1, ..Made::default() },
            // A node of its own: its fields are its own.
            "ALIAS" => Made { nodes: 1, ..Made::default() },
            "PREC" | "PREC_LEFT" | "PREC_RIGHT" | "PREC_DYNAMIC" | "RESERVED" => self.made(content()?, inside)?,
            "REPEAT" | "REPEAT1" => self.made(content()?, inside)?.many(),
            "SEQ" | "CHOICE" => {
                let mut out = Made::default();
                for m in rule.get("members")?.as_array()? {
                    let m = self.made(m, inside)?;
                    out.recursive |= m.recursive;
                    if ty == "SEQ" {
                        out.nodes = add(out.nodes, m.nodes);
                        for (f, c) in m.fields {
                            let e = out.fields.entry(f).or_default();
                            *e = add(*e, c);
                        }
                    } else {
                        out.nodes = out.nodes.max(m.nodes);
                        for (f, c) in m.fields {
                            let e = out.fields.entry(f).or_default();
                            *e = (*e).max(c);
                        }
                    }
                }
                out
            }
            // The field tags each node its content makes; fields inside the content are the parent's too.
            "FIELD" => {
                let name = rule.get("name")?.as_str()?;
                let mut m = self.made(content()?, inside)?;
                let e = m.fields.entry(name.to_owned()).or_default();
                *e = add(*e, m.nodes.max(1));
                m
            }
            "SYMBOL" => {
                let name = rule.get("name")?.as_str()?;
                match self.rules.get(name) {
                    // An external token, or a visible node: one node, its fields its own.
                    None => Made { nodes: 1, ..Made::default() },
                    Some(_) if !self.hidden(name) => Made { nodes: 1, ..Made::default() },
                    Some(_) if inside.contains(&name) => Made { nodes: 2, fields: BTreeMap::new(), recursive: true },
                    Some(body) => {
                        inside.push(name);
                        let m = self.made(body, inside);
                        inside.pop();
                        let m = m?;
                        if m.recursive { m.many() } else { m }
                    }
                }
            }
            _ => return None,
        })
    }
}

/// The fields of `grammar` (a `grammar.json`'s text) some node may hold more than once, sorted; `None` where its
/// rules are not all understood.
pub fn repeated(grammar: &str) -> Option<Vec<String>> {
    let g: Value = serde_json::from_str(grammar).ok()?;
    let rules = g.get("rules")?.as_object()?;
    let mut hidden = BTreeSet::new();
    for i in g.get("inline").and_then(Value::as_array).into_iter().flatten() {
        hidden.insert(i.as_str()?);
    }
    let grammar = Grammar { rules, hidden };
    let mut many = BTreeSet::new();
    // Every node's: each visible rule's, and each alias's (a node of its own, its content's fields).
    let mut nodes: Vec<&Value> = rules.iter().filter(|(n, _)| !grammar.hidden(n)).map(|(_, r)| r).collect();
    let mut aliases: Vec<&Value> = Vec::new();
    fn collect<'a>(r: &'a Value, out: &mut Vec<&'a Value>) {
        if r.get("type").and_then(Value::as_str) == Some("ALIAS")
            && let Some(c) = r.get("content")
        {
            out.push(c);
        }
        if let Some(c) = r.get("content") {
            collect(c, out);
        }
        for m in r.get("members").and_then(Value::as_array).into_iter().flatten() {
            collect(m, out);
        }
    }
    for r in rules.values() {
        collect(r, &mut aliases);
    }
    // An alias's content: a hidden rule's body is the aliased node's (a visible rule's node is counted as its own).
    for c in aliases {
        match c.get("type").and_then(Value::as_str) {
            Some("SYMBOL") => {
                let name = c.get("name").and_then(Value::as_str)?;
                if grammar.hidden(name)
                    && let Some(body) = rules.get(name)
                {
                    nodes.push(body);
                }
            }
            _ => nodes.push(c),
        }
    }
    for body in nodes {
        let m = grammar.made(body, &mut Vec::new())?;
        many.extend(m.fields.into_iter().filter(|&(_, c)| c > 1).map(|(f, _)| f));
    }
    Some(many.into_iter().collect())
}

#[cfg(test)]
mod tests {
    /// Python's: a field a node holds twice (an `if`'s `alternative`s, a subscript's `subscript`s, an import's `name`s,
    /// a call's `argument`s in `print`) and one none does (a function's `parameters`, an assignment's `left`, an
    /// attribute's `object`). The vendored grammar's, as the build reads it.
    #[test]
    fn pythons_repeated_fields_are_those_a_node_holds_twice() {
        let json = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../grammars/tree-sitter-python/src/grammar.json");
        let fields = super::repeated(&std::fs::read_to_string(&json).unwrap()).expect("Python's rules are understood");
        for f in ["alternative", "subscript", "name", "argument"] {
            assert!(fields.iter().any(|x| x == f), "{f} repeats: {fields:?}");
        }
        for f in ["parameters", "left", "object", "function", "attribute"] {
            assert!(!fields.iter().any(|x| x == f), "{f} does not repeat: {fields:?}");
        }
        assert_eq!(crate::code_rule_dsl::language::field_repeats(crate::code_rule_dsl::language::GrammarDialect::Python, "parameters"), Some(false));
        assert_eq!(crate::code_rule_dsl::language::field_repeats(crate::code_rule_dsl::language::GrammarDialect::Python, "alternative"), Some(true));
    }

    /// A rule type it does not know makes no list: no field is claimed single.
    #[test]
    fn a_rule_it_does_not_know_makes_no_list() {
        let g = r#"{"rules": {"a": {"type": "FIELD", "name": "x", "content": {"type": "NEW_KIND"}}}}"#;
        assert_eq!(super::repeated(g), None);
        let g = r#"{"rules": {"a": {"type": "REPEAT", "content": {"type": "FIELD", "name": "x", "content": {"type": "SYMBOL", "name": "b"}}}, "b": {"type": "STRING", "value": "b"}}}"#;
        assert_eq!(super::repeated(g), Some(vec!["x".to_owned()]));
        // Recursion through a hidden rule repeats its fields; a field over a hidden sequence tags each of its nodes.
        let g = r#"{"rules": {"a": {"type": "SYMBOL", "name": "_l"}, "_l": {"type": "SEQ", "members": [{"type": "FIELD", "name": "y", "content": {"type": "STRING", "value": "y"}}, {"type": "CHOICE", "members": [{"type": "SYMBOL", "name": "_l"}, {"type": "BLANK"}]}]}, "c": {"type": "FIELD", "name": "z", "content": {"type": "SYMBOL", "name": "_two"}}, "_two": {"type": "SEQ", "members": [{"type": "STRING", "value": "p"}, {"type": "STRING", "value": "q"}]}, "d": {"type": "FIELD", "name": "w", "content": {"type": "STRING", "value": "w"}}}}"#;
        assert_eq!(super::repeated(g), Some(vec!["y".to_owned(), "z".to_owned()]));
    }
}
