//! Worked examples kept within the structural compiler's supported policy surface.

use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const NO_UNUSED_VARS: &str = include_str!("examples/eslint-no-unused-vars.json");
const A005: &str = include_str!("examples/ruff-A005.json");

fn configured(source: &str, config: Value) -> crate::policy_dsl::CompiledPolicyDsl {
    let mut document: Value = serde_json::from_str(source).expect("example");
    let define = document["clauses"]
        .as_array_mut()
        .expect("checks")
        .iter_mut()
        .find(|check| check["define"].get("options").is_some())
        .expect("an options define");
    define["define"]["config"] = config;
    compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn messages(
    compiled: &crate::policy_dsl::CompiledPolicyDsl,
    language: &str,
    path: &str,
    source: &str,
) -> Vec<String> {
    let result = evaluate_file(compiled, language, path, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["message"].as_str().expect("message").to_owned())
        .collect()
}

#[test]
fn no_unused_vars_is_one_document_configured_per_variant() {
    assert_eq!(
        messages(
            &configured(NO_UNUSED_VARS, json!({ "enabled": true })),
            "typescript",
            "a.ts",
            "const a = 1; let b = 2;\n",
        ),
        ["variable declaration", "variable declaration"],
    );
}

#[test]
fn a005_takes_its_scope_and_allowed_modules_from_options() {
    let compiled = configured(A005, json!({ "enabled": true }));
    assert!(compiled.in_scope("src/module.py".to_owned()));
    assert!(!compiled.in_scope("src/module.ts".to_owned()));
    assert_eq!(
        messages(&compiled, "python", "src/module.py", "x = 1\n"),
        ["module assignment"],
    );
}
