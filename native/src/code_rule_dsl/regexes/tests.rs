use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::test_support::{
    CAPABILITIES, compile_document_with, compiled, grammar_matchers, violation,
};
use serde_json::{Value, json};

// Each test writes patterns no other test writes: tests on other threads compile documents and
// drop rules, and each compilation and each rule that drops sweeps the table.

fn regex(pattern: &str) -> Arc<regex::Regex> {
    Regex::new(pattern).expect("the pattern compiles").0
}

#[test]
fn every_holder_of_a_pattern_holds_one_regex_until_a_sweep_finds_none() {
    let held = regex("^regexes-held$");
    assert!(Arc::ptr_eq(&held, &regex("^regexes-held$")));
    assert!(!Arc::ptr_eq(&held, &regex("^regexes-other$")));
    assert!(Regex::new("(").is_err() && Regex::new("(").is_err());
    sweep();
    assert!(Arc::ptr_eq(&held, &regex("^regexes-held$")));
    let gone = Arc::downgrade(&held);
    drop(held);
    sweep();
    assert!(gone.upgrade().is_none());
}

/// The regex of the first grammar's matcher, a rule of one `regex` term beside its `kind`.
fn regex_of(rule: &CompiledCodeRuleDsl) -> Arc<regex::Regex> {
    let AstMatcher::All(terms) = &grammar_matchers(rule)[0].matcher else {
        panic!("a kind and a regex term");
    };
    terms
        .iter()
        .find_map(|term| match term {
            AstMatcher::Regex(regex) => Some(Arc::clone(&regex.0)),
            _ => None,
        })
        .expect("a regex term")
}

#[test]
fn two_rules_that_write_one_pattern_hold_one_regex_until_neither_does() {
    let rule =
        |pattern: &str| compiled(json!({ "rule": { "kind": "identifier", "regex": pattern } }));
    let (first, second) = (rule("^regexes-two-rules$"), rule("^regexes-two-rules$"));
    assert!(Arc::ptr_eq(&regex_of(&first), &regex_of(&second)));
    let gone = Arc::downgrade(&regex_of(&first));
    drop(first);
    assert!(gone.upgrade().is_some());
    // The last rule that holds it frees it as it drops: no compilation comes after.
    drop(second);
    assert!(gone.upgrade().is_none());
}

#[test]
fn a_compilation_drops_the_regexes_only_its_validation_took() {
    let shared = json!({ "id": "shared", "define": { "language": "typescript", "utils": {
        "unreached": { "kind": "identifier", "regex": "^regexes-unreached$" },
    } } });
    let rule = json!({ "language": "typescript",
        "rule": { "kind": "identifier", "regex": "^regexes-reached$" } });
    let query = json!({ "id": "query", "forbid": { "ast": rule }, "violation": violation() });
    let document = json!({ "format": "rerail.policy/v1", "id": "regexes", "revision": 1,
        "mode": "steer", "clauses": [shared, query] });
    let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
    let _compiled = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    // Validation and the define check compiled the utility no query reaches; no clause holds it.
    let table = TABLE.read();
    assert!(table.contains_key("^regexes-reached$"));
    assert!(!table.contains_key("^regexes-unreached$"));
}
