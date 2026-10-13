//! Rust `arguments` bindings checked against the rustc oracle.

use super::{Arguments, Passed, SlotKind};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use ast_grep_language::LanguageExt;
use serde_json::Value as Json;

/// Arguments whose value no constant folding gives, with what the case's run printed for them:
/// the call `g(1)` returns its argument; `&S(7)` is a receiver whose field the method prints.
const NOT_CONSTANT: &[(&str, &str, &str)] = &[
    ("f(g(1), 2)", "g(1)", "1"),
    ("S::m(&S(7), 1)", "&S(7)", "7"),
];

/// Every call rustc 1.99.0 compiled in `fixtures/rust/arguments` (`generate.py`): each slot holds
/// the argument rustc bound to that parameter, by the value the run printed for it.
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
                    match crate::code_rule_dsl::value::rust::evaluate(&value) {
                        crate::code_rule_dsl::value::rust::RustValue::Int { value, .. } => {
                            Some(value.to_string())
                        }
                        // An argument with no constant value: the run printed what it evaluated to.
                        _ => NOT_CONSTANT
                            .iter()
                            .find(|(case, argument, _)| *case == call && *argument == text)
                            .map(|(_, _, printed)| (*printed).to_owned())
                            .or(Some(text)),
                    }
                }
            };
            let wanted = bound.get(expected).and_then(Json::as_str);
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
