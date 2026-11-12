use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
use serde_json::json;

// Each test writes patterns no other test writes: tests on other threads compile documents, and
// every compilation sweeps the table as it ends.

fn regex(pattern: &str) -> Arc<Regex> {
    compile(pattern).expect("the pattern compiles")
}

#[test]
fn every_holder_of_a_pattern_holds_one_regex_until_a_sweep_finds_none() {
    let held = regex("^regexes-held$");
    assert!(Arc::ptr_eq(&held, &regex("^regexes-held$")));
    assert!(!Arc::ptr_eq(&held, &regex("^regexes-other$")));
    assert!(compile("(").is_none() && compile("(").is_none());
    sweep();
    assert!(Arc::ptr_eq(&held, &regex("^regexes-held$")));
    let gone = Arc::downgrade(&held);
    drop(held);
    sweep();
    assert!(gone.upgrade().is_none());
}

/// The regex of the first grammar's matcher, a rule of one `regex` term beside its `kind`.
fn regex_of(rule: &CompiledCodeRuleDsl) -> Arc<Regex> {
    let AstMatcher::All(terms) = &grammar_matchers(rule)[0].matcher else {
        panic!("a kind and a regex term");
    };
    terms
        .iter()
        .find_map(|term| match term {
            AstMatcher::Regex(regex) => Some(Arc::clone(regex)),
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
    drop((first, second));
    // A compilation, as it ends, drops the regexes no compiled rule holds.
    let _next = rule("^regexes-next-rule$");
    assert!(gone.upgrade().is_none());
}
