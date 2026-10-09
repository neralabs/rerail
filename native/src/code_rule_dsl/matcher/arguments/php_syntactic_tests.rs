//! `arguments` with `mode: "syntactic"` for PHP, against nikic/PHP-Parser 5.9.0
//! (`fixtures/php/matcher/arguments/generate_php_syntactic.php`).

use super::{Arguments, Passed, SlotKind};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

/// Every case in the oracle: each slot holds the argument Ruff's `find_argument` finds in
/// PHP-Parser's `args`, or none; a first-class callable is no call.
#[test]
fn php_slots_are_found_as_find_argument_finds_them() {
    let oracle: Json =
        serde_json::from_str(oracle!("php/matcher/arguments/php-syntactic.expected.json"))
            .expect("oracle JSON");
    assert_eq!(oracle["phpParser"].as_str(), Some("5.9.0"));
    let mut failures = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        let call = case["call"].as_str().expect("call");
        let arguments = Arguments {
            language: GrammarDialect::Php,
            slots: case["signature"]
                .as_array()
                .expect("signature")
                .iter()
                .map(|name| (name.as_str().expect("slot").to_owned(), SlotKind::Either))
                .collect(),
            bind: Vec::new(),
            absent: Vec::new(),
            call_kinds: Default::default(),
            syntactic: true,
            estree: false,
        };
        let source = format!("<?php {call};");
        let root = GrammarDialect::Php.ast_grep(&source);
        let node = root
            .root()
            .dfs()
            .find(|node| {
                matches!(
                    node.kind().as_ref(),
                    "function_call_expression"
                        | "member_call_expression"
                        | "nullsafe_member_call_expression"
                        | "scoped_call_expression"
                        | "object_creation_expression"
                )
            })
            .expect("a call");
        let passed = arguments.php_syntactic(&node);
        if case["callable"].as_bool() == Some(true) {
            if passed.is_some() {
                failures.push(format!(
                    "{call:?} is a first-class callable, Rerail reads a call"
                ));
            }
            continue;
        }
        let Some(passed) = passed else {
            failures.push(format!("{call:?} is a call, Rerail reads none"));
            continue;
        };
        for (index, (slot, expected)) in passed
            .iter()
            .zip(case["slots"].as_array().expect("slots"))
            .enumerate()
        {
            let actual = match slot {
                Passed::Absent => None,
                Passed::Value(node) => Some(node.text().into_owned()),
                Passed::Unknown => Some("<unknown>".to_owned()),
            };
            if actual.as_deref() != expected.as_str() {
                failures.push(format!(
                    "{call:?}: slot {index}: PHP-Parser finds {expected}, Rerail {actual:?}"
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
