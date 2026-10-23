//! The leaf executors' compile check for the Phase 2 compiler (`rerail_facts::front::rule::LeafCheck`):
//! pattern queries compile with this crate's `ast.rs`, regex clauses are checked against what
//! `eval.rs::regex_findings` implements. Refusals name the construct.
use crate::ast::{Compiler, Names};
use rerail_facts::front::rule::LeafCheck;
use serde_json::Value as Json;
use std::collections::BTreeMap;

pub struct Leaves {
    names: Names,
}

impl Leaves {
    pub fn new() -> Self {
        Self { names: Names::default() }
    }
}

/// Whether a JSON rule holds `parse: { grammar: annotation }` (directly or through utilities).
fn annotation(v: &Json, utils: &BTreeMap<String, Json>, seen: &mut Vec<String>) -> bool {
    match v {
        Json::Object(o) => o.iter().any(|(k, x)| {
            if k == "parse" && x.get("grammar").and_then(Json::as_str) == Some("annotation") {
                return true;
            }
            if k == "matches" {
                if let Some(id) = x.as_str() {
                    if !seen.iter().any(|s| s == id) {
                        seen.push(id.to_owned());
                        if utils.get(id).is_some_and(|u| annotation(u, utils, seen)) {
                            return true;
                        }
                    }
                }
            }
            annotation(x, utils, seen)
        }),
        Json::Array(a) => a.iter().any(|x| annotation(x, utils, seen)),
        _ => false,
    }
}

impl LeafCheck for Leaves {
    fn pattern(&mut self, ast: &Json, utils: &BTreeMap<String, Json>) -> Result<Vec<String>, String> {
        let mut c = Compiler::new(&mut self.names, &crate::facts::GRAMMAR, utils);
        c.query(ast)?;
        let mut all = utils.clone();
        if let Some(u) = ast.get("utils").and_then(Json::as_object) {
            all.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let mut runtime = Vec::new();
        if ast.get("rule").is_some_and(|r| annotation(r, &all, &mut Vec::new())) {
            runtime.push("parse: {grammar: annotation} (needs the string_type_definition fact)".to_owned());
        }
        Ok(runtime)
    }

    fn known_field(&self, field: &str) -> Option<bool> {
        Some(crate::facts::GRAMMAR.field_code(field).is_some())
    }

    fn regex(&mut self, spec: &Json) -> Result<(), String> {
        if spec.get("content").is_some() {
            return Err("`content` gate".into());
        }
        if let Some(l) = spec.get("lines") {
            for k in ["include", "exclude", "state", "indentation"] {
                if l.get(k).is_some() {
                    return Err(format!("`lines.{k}`"));
                }
            }
            if let Some(t) = l.get("terminators").and_then(Json::as_str).filter(|t| *t != "python") {
                return Err(format!("`lines.terminators: {t}`"));
            }
        }
        Ok(())
    }
}
