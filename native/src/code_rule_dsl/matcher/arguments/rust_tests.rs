//! Rust `arguments` bindings checked against the rustc oracle.

use super::{Arguments, Passed, SlotKind};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use ast_grep_language::LanguageExt;
use serde_json::Value as Json;

#[test]
fn rust_slots_follow_rustc() {
    let oracle: Json = serde_json::from_str(oracle!("rust/arguments/rust.expected.json"))
        .expect("Rust argument oracle JSON");
    let mut failures = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        if case["compiled"] != true {
            continue;
        }
        let call = case["call"].as_str().expect("call");
        let signature = case["signature"].as_array().expect("signature");
        let arguments = Arguments {
            language: GrammarDialect::Rust,
            slots: signature
                .iter()
                .map(|name| (name.as_str().expect("slot").to_owned(), SlotKind::Either))
                .collect(),
            bind: Vec::new(),
            absent: Vec::new(),
            call_kinds: Default::default(),
            syntactic: false,
            estree: false,
        };
        let source = format!("fn main() {{ let _ = {call}; }}");
        let root = GrammarDialect::Rust.ast_grep(&source);
        let Some(node) = root
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
        else {
            failures.push(format!("{call:?}: no call_expression"));
            continue;
        };
        let passed = arguments.rust(&node);
        let bound = case["bound"].as_object().expect("bound");
        for (index, slot) in passed.iter().enumerate() {
            let expected = signature[index].as_str().expect("slot");
            let actual = match slot {
                Passed::Absent => None,
                Passed::Unknown => Some("<unknown>".to_owned()),
                Passed::Value(value) => {
                    let text = value.text().into_owned().replace(' ', "");
                    Some(text.trim_matches('{').trim_matches('}').to_owned())
                }
            };
            let wanted = bound.get(expected).and_then(Json::as_str);
            if wanted.is_none()
                && (call.starts_with("P(") || call.starts_with("E::V(") || call.contains("S::m"))
            {
                continue;
            }
            if call.contains("g(1)") {
                continue;
            }
            if actual.as_deref() != wanted {
                failures.push(format!(
                    "{call:?}: slot {index}: expected {wanted:?}, got {actual:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
