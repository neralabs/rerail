use super::*;
use crate::code_rule_dsl::{language::GrammarDialect, oracle};
use ast_grep_core::{Doc, Node, tree_sitter::LanguageExt};
use serde_json::Value;

#[test]
fn dumps_and_signed_fingerprints_match_cpython_3_13() {
    let oracle: Value =
        serde_json::from_str(oracle!("python/ast-dump/cpython.expected.json")).unwrap();
    assert!(oracle["python"].as_str().unwrap().starts_with("3.13."));
    let mut documents = Vec::new();
    let mut expected = Vec::new();
    for case in oracle["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let tree = GrammarDialect::Python.ast_grep(source);
        let mut requests = vec![Request {
            start: 0,
            end: source.len(),
            kind: "module".into(),
            fields: vec!["dump".into()],
        }];
        let mut rows = vec![Row::from([(
            "dump".into(),
            case["dump"].as_str().unwrap().into(),
        )])];
        let mut functions = Vec::new();
        collect(&tree.root(), &mut functions);
        for entry in case["functions"].as_array().unwrap() {
            let line = entry["line"].as_u64().unwrap() as usize;
            let node = functions
                .iter()
                .find(|n| n.start_pos().line() + 1 == line)
                .unwrap();
            requests.push(Request {
                start: node.range().start,
                end: node.range().end,
                kind: "function_definition".into(),
                fields: vec![
                    "body_sha256".into(),
                    "signature_sha256".into(),
                    "identifier".into(),
                ],
            });
            rows.push(Row::from([
                (
                    "body_sha256".into(),
                    entry["body_sha256"].as_str().unwrap().into(),
                ),
                (
                    "signature_sha256".into(),
                    entry["signature_sha256"].as_str().unwrap().into(),
                ),
                ("identifier".into(), entry["name"].as_str().unwrap().into()),
            ]));
        }
        documents.push(Document {
            source: source.into(),
            requests,
        });
        expected.push(rows);
    }
    let actual = batch(&documents).unwrap_or_else(|e| panic!("{}", e.0));
    assert_eq!(actual.len(), 30);
    for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(actual, expected, "case {index}");
    }
}
fn collect<'a, D: Doc>(node: &Node<'a, D>, functions: &mut Vec<Node<'a, D>>) {
    if node.kind() == "function_definition" {
        functions.push(node.clone());
    }
    for child in node.children() {
        collect(&child, functions);
    }
}
#[test]
fn empty_batches_do_no_external_work() {
    assert!(batch(&[]).unwrap().is_empty());
}
#[test]
fn source_is_parsed_without_executing_imports_or_statements() {
    let source = "import module_that_does_not_exist\nraise RuntimeError('must not execute')\n";
    let out = batch(&[Document {
        source: source.into(),
        requests: vec![Request {
            start: 0,
            end: source.len(),
            kind: "module".into(),
            fields: vec!["dump".into()],
        }],
    }])
    .unwrap();
    assert!(out[0][0]["dump"].contains("RuntimeError"));
}
#[test]
fn unknown_fields_and_invalid_source_fail_the_whole_batch() {
    for (source, field) in [("x=1", "unknown"), ("def f(", "dump")] {
        let error = batch(&[Document {
            source: source.into(),
            requests: vec![Request {
                start: 0,
                end: source.len(),
                kind: "module".into(),
                fields: vec![field.into()],
            }],
        }])
        .unwrap_err();
        assert!(error.0.contains("CPython AST batch failed"), "{}", error.0);
    }
}

#[test]
fn annotations_use_canonical_unparse() {
    let out = batch(&[Document {
        source: "x: dict[str,int] = {}\n".into(),
        requests: vec![Request {
            start: 3,
            end: 16,
            kind: "type".into(),
            fields: vec!["unparse".into()],
        }],
    }])
    .unwrap();
    assert_eq!(out[0][0]["unparse"], "dict[str, int]");
}

#[test]
fn literal_metadata_has_canonical_json_without_source_execution() {
    let source = "('x', ('y', 3))";
    let out = batch(&[Document {
        source: source.into(),
        requests: vec![Request {
            start: 0,
            end: source.len(),
            kind: "tuple".into(),
            fields: vec!["literal_json".into()],
        }],
    }])
    .unwrap();
    assert_eq!(out[0][0]["literal_json"], r#"["x",["y",3]]"#);
}
