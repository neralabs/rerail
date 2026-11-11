use super::*;

#[test]
fn syntax_error_facts_are_batched_and_do_not_walk_invalid_modules() {
    let documents =
        ["def invalid(\n", "raise RuntimeError('never execute')\n"].map(|source| Document {
            source: source.into(),
            requests: vec![Request {
                start: 0,
                end: source.len(),
                kind: "module".into(),
                fields: vec!["syntax_error".into()],
            }],
        });
    let result = batch_with_metrics(&documents).unwrap_or_else(|error| panic!("{}", error.0));
    assert_eq!(result.parses, 2);
    assert_eq!(result.module_walks, 0);
    let error: serde_json::Value =
        serde_json::from_str(&result.documents[0][0]["syntax_error"]).unwrap();
    assert_eq!(error["msg"], "'(' was never closed");
    assert_eq!(error["lineno"], 1);
    assert_eq!(error["offset"], 12);
    assert_eq!(error["text"], "def invalid(\n");
    assert_eq!(result.documents[1][0]["syntax_error"], "null");
    let invalid = Document {
        source: "def invalid(\n".into(),
        requests: vec![Request {
            start: 0,
            end: 13,
            kind: "module".into(),
            fields: vec!["syntax_error".into(), "dump".into()],
        }],
    };
    assert!(batch(&[invalid]).is_err());
}
use crate::code_rule_dsl::{language::GrammarDialect};
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

#[test]
fn canonical_names_and_annotation_repr_preserve_python_semantics() {
    let source = "import ｍｏｄ.ｎａｍｅ as alias\nvalue: \"OptimizerRunCache\"\n";
    let name = "ｍｏｄ.ｎａｍｅ";
    let annotation = "\"OptimizerRunCache\"";
    let make = |text: &str, kind: &str, fields: &[&str]| Request {
        start: source.find(text).unwrap(),
        end: source.find(text).unwrap() + text.len(),
        kind: kind.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let out = batch(&[Document {
        source: source.into(),
        requests: vec![
            make(name, "dotted_name", &["identifier"]),
            make(annotation, "type", &["unparse", "repr_unparse"]),
        ],
    }])
    .unwrap();
    assert_eq!(out[0][0]["identifier"], "mod.name");
    assert_eq!(out[0][1]["unparse"], "'OptimizerRunCache'");
    assert_eq!(out[0][1]["repr_unparse"], "\"'OptimizerRunCache'\"");
}

#[test]
fn text_columns_do_not_parse_or_walk_the_module() {
    let result = batch_with_metrics(&[Document {
        source: "  ｃａｃｈｅ\n".into(),
        requests: vec![
            Request {
                start: 2,
                end: 17,
                kind: "identifier".into(),
                fields: vec!["identifier".into()],
            },
            Request {
                start: 2,
                end: 17,
                kind: "identifier".into(),
                fields: vec!["column".into()],
            },
        ],
    }])
    .unwrap_or_else(|error| panic!("{}", error.0));
    assert_eq!(result.parses, 0);
    assert_eq!(result.module_walks, 0);
    assert_eq!(result.documents[0][0]["identifier"], "cache");
    assert_eq!(result.documents[0][1]["column"], "2");
}

#[test]
fn function_fields_share_one_real_parse_and_module_walk() {
    let source = "def a(): return 1\ndef b(): return 2\n";
    let result = batch_with_metrics(&[Document {
        source: source.into(),
        requests: vec![
            Request {
                start: 0,
                end: 17,
                kind: "function_definition".into(),
                fields: vec!["body_sha256".into(), "signature_sha256".into()],
            },
            Request {
                start: 18,
                end: 35,
                kind: "function_definition".into(),
                fields: vec!["dump".into()],
            },
        ],
    }])
    .unwrap_or_else(|error| panic!("{}", error.0));
    assert_eq!(result.parses, 1);
    assert_eq!(result.module_walks, 1);
    assert_eq!(result.documents[0].len(), 2);
}
