//! Agreement with the PHP compare oracle's structures (`fixtures/php/value/compare/generate_php.php`):
//! nikic/PHP-Parser 5.9.0's trees, attributes stripped, and PHP's array key identity.

use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

const ORACLE: &str = oracle!("php/value/compare/php.expected.json");

/// The structure of the one expression or statement `snippet` is.
fn read(snippet: &str, statement: bool, equality: Equality) -> Option<Vec<Tok>> {
    let text = if statement {
        format!("<?php {snippet}")
    } else {
        format!("<?php {snippet};")
    };
    let root = GrammarDialect::Php.ast_grep(&text);
    let statements: Vec<_> = root
        .root()
        .children()
        .filter(|child| child.is_named() && !matches!(child.kind().as_ref(), "php_tag" | "comment"))
        .collect();
    let [node] = statements.as_slice() else {
        panic!("{snippet:?} is not one statement");
    };
    // An expression PHP's grammar reads as a statement (`exit;`) compares as the expression
    // statement holds it: without the statement's own tokens.
    if !statement && node.kind() == "exit_statement" {
        let mut spend = || true;
        let mut context = Context::new(&mut spend);
        let tokens = structure(node, equality, &mut context)?;
        return Some(tokens[1..tokens.len() - 1].to_vec());
    }
    let node = if statement {
        node.clone()
    } else {
        node.children()
            .find(|child| child.is_named() && child.kind() != "comment")
            .unwrap_or_else(|| panic!("{snippet:?} holds no expression"))
    };
    let mut spend = || true;
    let mut context = Context::new(&mut spend);
    structure(&node, equality, &mut context)
}

/// Every pair of snippets in the oracle: `structure` reads them equal exactly where PHP-Parser's
/// trees are, and `structure-hash` exactly where PHP's array keys or, for anything else, the trees
/// are.
#[test]
fn structures_are_php_parsers_trees() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["phpParser"].as_str(), Some("5.9.0"));
    let mut failures = Vec::new();
    let mut compared = 0;
    for case in oracle["structures"].as_array().expect("structures") {
        let (left, right) = (
            case["left"].as_str().expect("left"),
            case["right"].as_str().expect("right"),
        );
        let statement = case["statement"].as_bool().unwrap_or(false);
        for (equality, field) in [(Equality::Plain, "equal"), (Equality::Hash, "hash")] {
            let expected = case[field].as_bool().expect(field);
            let (l, r) = (
                read(left, statement, equality),
                read(right, statement, equality),
            );
            let actual = match (&l, &r) {
                (Some(l), Some(r)) => Some(l == r),
                _ => None,
            };
            compared += 1;
            if actual != Some(expected) {
                failures.push(format!(
                    "{left:?} and {right:?} ({field}): PHP-Parser {expected}, Rerail {actual:?}\n  {l:?}\n  {r:?}"
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
    assert!(compared > 300, "{compared} comparisons");
}
