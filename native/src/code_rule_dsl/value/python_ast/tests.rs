use super::*;
use crate::code_rule_dsl::{language::GrammarDialect, oracle};
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn dumps_and_signed_fingerprints_match_cpython_3_13() {
    let oracle: Value = serde_json::from_str(oracle!("python/ast-dump/cpython.expected.json")).unwrap();
    assert!(oracle["python"].as_str().unwrap().starts_with("3.13."));
    for case in oracle["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let tree = GrammarDialect::Python.ast_grep(source);
        let actual = dump(&tree.root()).unwrap_or_else(|e| panic!("case {}: {}", case["id"], e.0));
        assert_eq!(actual, case["dump"].as_str().unwrap(), "case {}", case["id"]);
        let mut functions = Vec::new();
        collect(&tree.root(), &mut functions);
        for expected in case["functions"].as_array().unwrap() {
            let name = expected["name"].as_str().unwrap();
            let line = expected["line"].as_u64().unwrap() as usize;
            let node = functions.iter().find(|n| n.start_pos().line() + 1 == line).unwrap_or_else(|| panic!("{name}:{line}"));
            let actual = function(node).unwrap_or_else(|e| panic!("case {} {name}: {}", case["id"], e.0));
            assert_eq!(actual.body, expected["body"].as_str().unwrap(), "case {} {name}: body", case["id"]);
            assert_eq!(actual.signature, expected["signature"].as_str().unwrap(), "case {} {name}: signature", case["id"]);
            assert_eq!(format!("{:x}", Sha256::digest(actual.body.as_bytes())), expected["body_sha256"].as_str().unwrap());
            assert_eq!(format!("{:x}", Sha256::digest(actual.signature.as_bytes())), expected["signature_sha256"].as_str().unwrap());
        }
    }
}
fn collect<'a, D: Doc>(node: &Node<'a, D>, functions: &mut Vec<Node<'a, D>>) {
    if node.kind() == "function_definition" { functions.push(node.clone()); }
    for child in node.children() { collect(&child, functions); }
}
