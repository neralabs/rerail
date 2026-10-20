//! The shipped root guards must preserve findings and captures on positive Ruff fixtures.

use super::{evaluate, snapshot};
use crate::code_rule_dsl::test_support::{CAPABILITIES, compile_document_with};
use serde_json::Value;

#[test]
fn python_root_guards_preserve_positive_findings_and_suppressions() {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    for (policy, source, noqa) in [
        (
            include_str!(
                "../../../../../packs/python/rules/ruff-b020-loop-variable-overrides-iterator.json"
            ),
            include_str!(
                "../../../../../packs/python/upstream/ruff/fixtures/flake8_bugbear/B020.py"
            ),
            include_str!("../../../../../packs/python/upstream/ruff/fixtures/rerail/noqa/B020.py"),
        ),
        (
            include_str!(
                "../../../../../packs/python/rules/ruff-b023-function-uses-loop-variable.json"
            ),
            include_str!(
                "../../../../../packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py"
            ),
            include_str!("../../../../../packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py"),
        ),
    ] {
        let guarded: Value = serde_json::from_str(policy).unwrap();
        let mut original = guarded.clone();
        let root = &mut original["clauses"][0]["forbid"]["ast"]["rule"];
        *root = root["all"].as_array().unwrap().last().unwrap().clone();
        let input = snapshot(&[
            ("python", "positive.py", source),
            ("python", "noqa.py", noqa),
        ]);
        let original = compile_document_with(original, &capabilities).unwrap();
        let guarded = compile_document_with(guarded, &capabilities).unwrap();
        let expected = evaluate(&original, &input, "native");
        assert!(!expected["violations"].as_array().unwrap().is_empty());
        assert!(expected["failures"].as_array().unwrap().is_empty());
        for engine in ["native", "sql"] {
            let actual = evaluate(&guarded, &input, engine);
            assert_eq!(actual["violations"], expected["violations"], "{engine}");
            assert_eq!(actual["failures"], expected["failures"], "{engine}");
        }
    }
}
