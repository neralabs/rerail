//! Distinct-key evaluation (`emit::keyed`): a walking sub-rule runs once per distinct key of its
//! input, `(file, node, the slots it names)`, however many rows share the key, and the rows take
//! their key's verdict back. Rows still agree with the native matcher, and nested walks write rows
//! linear in the walks' own pairs rather than their product.

use super::{same, snapshot};
use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
use crate::code_rule_dsl::test_support::compiled;
use serde_json::json;

/// A TypeScript source whose nesting is `depth` blocks deep, with a few statements each.
fn nested(depth: usize) -> String {
    let mut source = String::new();
    for level in 0..depth {
        source.push_str(&format!("if (a{level} > {level}) {{ f(b{level}, 1); g({level});\n"));
    }
    source.push_str(&"}\n".repeat(depth));
    source
}

/// `inside end { has end }` from every identifier: per root its ancestors, per ancestor its
/// descendants. Per row, the descendants would be walked once per (root, ancestor) pair; per
/// distinct key, once per ancestor. The rows written stay within a small multiple of the
/// (node, ancestor) pairs, the cost of the walks themselves.
#[test]
fn nested_walks_run_once_per_distinct_node() {
    let rule = compiled(json!({ "rule": {
        "kind": "identifier",
        "inside": { "stopBy": "end", "kind": "if_statement", "has": { "stopBy": "end", "kind": "number", "regex": "^1$" } },
    } }));
    let rows = |depth: usize| {
        STATEMENT_PROFILE.set(Some(Default::default()));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &nested(depth))]));
        let profile = STATEMENT_PROFILE.take().unwrap_or_default();
        assert!(result["violations"].as_array().is_some_and(|found| !found.is_empty()));
        profile.values().map(|(_, rows, _)| rows).sum::<usize>()
    };
    let (small, large) = (rows(10), rows(40));
    eprintln!("rows written: depth 10 {small}, depth 40 {large}");
    // Four times the depth: the (node, ancestor) pairs grow 16-fold; one walk per row from each
    // root would grow them 64-fold.
    assert!(large < small * 24, "{small} rows at depth 10, {large} at depth 40");
}

/// A walk keyed by a capture some rows bind and others leave unbound: the key compares slots
/// with `IS`, so an unbound row reads the verdict of the unbound key, which binds afresh.
#[test]
fn a_key_holds_unbound_slots_as_their_own_value() {
    let rule = compiled(json!({ "rule": {
        "any": [{ "pattern": "f($A, 1)" }, { "kind": "call_expression" }],
        "inside": { "stopBy": "end", "has": { "stopBy": "end", "pattern": "$A" } },
    } }));
    let result = same(&rule, &snapshot(&[("typescript", "a.ts", &nested(6))]));
    assert!(result["violations"].as_array().is_some_and(|found| found.len() > 6));
}

/// Three-valued: a keyed search returns its key's verdict, unknown ones included.
#[test]
fn a_keyed_search_returns_unknown_verdicts() {
    let rule = compiled(json!({ "rule": {
        "kind": "identifier",
        "inside": { "stopBy": "end", "has": { "stopBy": "end", "testUnknown": "^g$" } },
    } }));
    same(&rule, &snapshot(&[("typescript", "a.ts", &nested(5))]));
}
