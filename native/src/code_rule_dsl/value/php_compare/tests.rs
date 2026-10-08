//! Agreement with the PHP compare oracle (`rerail-fixtures/php/compare/generate_php.php`): keys,
//! members, names and references as nikic/PHP-Parser 5.9.0 parses the code and PHP 8.5 evaluates
//! and looks up what it names.

use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::scope::ScopeTable;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

const ORACLE: &str = oracle!("php/compare/php.expected.json");

fn oracle() -> Json {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["php"].as_str().map(|v| &v[..3]), Some("8.5"));
    assert_eq!(oracle["phpParser"].as_str(), Some("5.9.0"));
    oracle
}

/// The byte range of the `nth` occurrence of `at` in `source`, where an occurrence that begins or
/// ends with a word character stands on a word boundary there (as the generator finds it).
fn occurrence(source: &str, at: &str, nth: usize) -> std::ops::Range<usize> {
    let word =
        |byte: Option<&u8>| byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
    let bytes = source.as_bytes();
    let start = source
        .match_indices(at)
        .map(|(start, _)| start)
        .filter(|&start| {
            let end = start + at.len();
            let cut_before = word(at.as_bytes().first()) && start > 0 && word(bytes.get(start - 1));
            let cut_after = word(at.as_bytes().last()) && word(bytes.get(end));
            !cut_before && !cut_after
        })
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of {at:?} in {source:?}"));
    start..start + at.len()
}

/// The innermost named node spanning exactly `range`.
fn node_at<'r, D: Doc>(root: Node<'r, D>, range: &std::ops::Range<usize>) -> Node<'r, D> {
    root.dfs()
        .filter(|node| node.is_named() && node.range() == *range)
        .last()
        .unwrap_or_else(|| panic!("no node spans {range:?} in {:?}", root.text()))
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

/// The oracle's name as UTF-16, `None` for no name or one that is not UTF-8.
fn expected_name(name: &Json) -> Option<Vec<u16>> {
    let bytes = hex(name.as_str()?);
    Some(String::from_utf8(bytes).ok()?.encode_utf16().collect())
}

fn located(case: &Json) -> (&str, &str, usize) {
    (
        case["src"].as_str().expect("src"),
        case["at"].as_str().expect("at"),
        case["nth"].as_u64().unwrap_or(0) as usize,
    )
}

/// Every key in the oracle: where PHP names it, Rerail's `key` names it the same, unless the case
/// says why Rerail does not know it; where PHP has no name, neither has Rerail.
#[test]
fn keys_are_the_names_php_converts() {
    let mut failures = Vec::new();
    let oracle = oracle();
    for case in oracle["keys"].as_array().expect("keys") {
        let (source, at, nth) = located(case);
        let root = GrammarDialect::Php.ast_grep(source);
        let node = node_at(root.root(), &occurrence(source, at, nth));
        let mut read = |node: &Node<'_, _>| {
            let mut spend = || true;
            let mut context = Context::new(&mut spend);
            php::decode(node, &mut context)
        };
        let actual = key_name(&node, &mut read).flatten();
        let expected = expected_name(&case["name"]);
        let explained = case.get("unknown").is_some();
        if actual != expected && !(actual.is_none() && explained) {
            failures.push(format!(
                "{source:?} at {at:?}: PHP names {}, Rerail {:?}",
                case["name"],
                actual.map(|name| String::from_utf16_lossy(&name))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every member in the oracle, its computed name read through the program's bindings: a name
/// Rerail knows is the one PHP converts the value to; where PHP has a name and Rerail none, the
/// case says why.
#[test]
fn members_follow_bindings_to_the_names_php_converts() {
    let mut failures = Vec::new();
    let mut known = 0;
    let oracle = oracle();
    for case in oracle["members"].as_array().expect("members") {
        let (source, at, nth) = located(case);
        let root = GrammarDialect::Php.ast_grep(source);
        let table = ScopeTable::build(root.root(), GrammarDialect::Php, usize::MAX, &|| false)
            .expect("table builds");
        let node = node_at(root.root(), &occurrence(source, at, nth));
        let mut read = |node: &Node<'_, _>| {
            let mut spend = || true;
            let mut binding = |range: std::ops::Range<usize>, name: &str| {
                let reference = table.reference(&range)?;
                let (binding, value) = table.binding(reference, name)?;
                Some(binding.followed(value?))
            };
            let mut context = Context::new(&mut spend).following(&mut binding);
            php::decode(node, &mut context)
        };
        let actual = key_name(&node, &mut read).flatten();
        let expected = expected_name(&case["name"]);
        match (&actual, &expected) {
            (Some(actual), Some(expected)) if actual == expected => known += 1,
            (None, Some(_)) if case.get("unknown").is_some() => {}
            (None, None) => {}
            _ => failures.push(format!(
                "{source:?} at {at:?}: PHP names {}, Rerail {:?}",
                case["name"],
                actual.map(|name| String::from_utf16_lossy(&name))
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(known >= 12, "Rerail knows only {known} members");
}

/// Every pair of names in the oracle: Rerail's `name` reads them equal exactly where PHP finds the
/// same symbol by them; every string literal's name is its value, and a node that is no name or
/// literal string has none.
#[test]
fn names_are_what_php_looks_symbols_up_by() {
    let mut failures = Vec::new();
    let oracle = oracle();
    for case in oracle["names"]["pairs"].as_array().expect("pairs") {
        let source = case["src"].as_str().expect("src");
        let root = GrammarDialect::Php.ast_grep(source);
        let name = |side: &str| {
            let at = case[side][0].as_str().expect("text");
            let nth = case[side][1].as_u64().expect("nth") as usize;
            node_name(&node_at(root.root(), &occurrence(source, at, nth)))
        };
        let (left, right) = (name("left"), name("right"));
        let same = case["same"].as_bool().expect("same");
        if left.is_none() || right.is_none() || (left == right) != same {
            failures.push(format!(
                "{source:?}: {} and {}: PHP finds {}, Rerail names {:?} and {:?}",
                case["left"],
                case["right"],
                if same { "one symbol" } else { "two" },
                left.map(|name| String::from_utf16_lossy(&name)),
                right.map(|name| String::from_utf16_lossy(&name)),
            ));
        }
    }
    for case in oracle["names"]["strings"].as_array().expect("strings") {
        let (source, at, nth) = located(case);
        let root = GrammarDialect::Php.ast_grep(source);
        let actual = node_name(&node_at(root.root(), &occurrence(source, at, nth)));
        let expected = expected_name(&case["name"]);
        if actual != expected {
            failures.push(format!(
                "{source:?} at {at:?}: PHP's value {}, Rerail names {:?}",
                case["name"],
                actual.map(|name| String::from_utf16_lossy(&name))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The expression of a program's one expression statement.
fn expression<'r, D: Doc>(root: Node<'r, D>) -> Node<'r, D> {
    root.children()
        .find(|child| child.kind() == "expression_statement")
        .and_then(|statement| {
            statement
                .children()
                .find(|child| child.is_named() && child.kind() != "comment")
        })
        .expect("an expression")
}

/// Every pair of expressions in the oracle is the same reference, with and without static keys,
/// exactly where the oracle's PHP-Parser trees are.
#[test]
fn references_are_the_same_where_php_parser_and_php_agree() {
    let mut failures = Vec::new();
    let oracle = oracle();
    for case in oracle["references"].as_array().expect("references") {
        let (left, right) = (
            case["left"].as_str().expect("left"),
            case["right"].as_str().expect("right"),
        );
        let left_text = format!("<?php {left};");
        let right_text = format!("<?php {right};");
        let left_root = GrammarDialect::Php.ast_grep(&left_text);
        let right_root = GrammarDialect::Php.ast_grep(&right_text);
        let (l, r) = (expression(left_root.root()), expression(right_root.root()));
        for (static_keys, field) in [(true, "same"), (false, "sameNoStaticKeys")] {
            let expected = case[field].as_bool().expect(field);
            let actual = same_reference(&l, &r, static_keys);
            if actual != Some(expected) {
                failures.push(format!(
                    "{left:?} and {right:?} (static keys {static_keys}): oracle {expected}, Rerail {actual:?}"
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
