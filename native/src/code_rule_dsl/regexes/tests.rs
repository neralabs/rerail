use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::test_support::{
    CAPABILITIES, compile_document_with, compiled, grammar_matchers, violation,
};
use serde_json::{Value, json};

// Each test writes patterns no other test writes, and lets go of its handles on its own thread,
// which compiles nothing then unless the test says so.

fn shared(regex: &Regex) -> &Arc<regex::Regex> {
    &regex.0
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("the pattern compiles")
}

#[test]
fn every_holder_of_a_pattern_holds_one_regex_until_none_does() {
    let held = regex("^regexes-held$");
    assert!(Arc::ptr_eq(shared(&held), shared(&regex("^regexes-held$"))));
    assert!(!Arc::ptr_eq(
        shared(&held),
        shared(&regex("^regexes-other$"))
    ));
    assert!(!TABLE.read().contains_key("^regexes-other$"));
    assert!(Regex::new("(").is_err() && Regex::new("(").is_err());
    let copy = held.clone();
    let gone = Arc::downgrade(shared(&held));
    drop(held);
    assert!(gone.upgrade().is_some());
    drop(copy);
    assert!(gone.upgrade().is_none());
    assert!(!TABLE.read().contains_key("^regexes-held$"));
}

#[test]
fn a_compiling_thread_keeps_the_regexes_it_lets_go_of_until_the_compilation_ends() {
    let compilation = compiling();
    let gone = Arc::downgrade(shared(&regex("^regexes-validated$")));
    let taken = regex("^regexes-validated$");
    assert!(
        gone.upgrade()
            .is_some_and(|kept| Arc::ptr_eq(&kept, shared(&taken)))
    );
    drop(taken);
    assert!(gone.upgrade().is_some());
    drop(compilation);
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
            AstMatcher::Regex(regex) => Some(Arc::clone(shared(regex))),
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
