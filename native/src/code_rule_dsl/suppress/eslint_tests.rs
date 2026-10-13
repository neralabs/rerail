//! ESLint's directive matrix: a rule reporting every identifier, with `suppress.eslint`, against
//! the reports ESLint 10.11.0 keeps after applying the fixtures' directive comments
//! (`fixtures/ecma/suppress/generate_eslint.mjs`).

use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("ecma/suppress/eslint.expected.json");

/// ESLint's `Identifier`: JSX tag names are `JSXIdentifier`s.
fn every_name() -> Value {
    json!({ "kind": "identifier", "not": { "inside": { "stopBy": "neighbor",
        "any": [{ "kind": "jsx_opening_element" }, { "kind": "jsx_closing_element" }] } } })
}

fn starts(fixture: &str, source: &str, suppress: Option<Value>) -> Vec<u64> {
    let mut violation = json!({ "anchor": "witness.span", "code": "NAME", "message": "name" });
    if let Some(suppress) = suppress {
        violation["suppress"] = suppress;
    }
    let document = clause_document(json!({ "id": "every-name", "violation": violation,
        "forbid": { "ast": { "language": "typescript", "rule": every_name() } } }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let language = if fixture.ends_with(".tsx") {
        "tsx"
    } else {
        "typescript"
    };
    let result = evaluate_file(&compiled, language, fixture, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut starts = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts
}

#[test]
fn eslint_directives_suppress_as_eslint_applies_them() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["versions"]["eslint"], "10.11.0");
    let fixtures = [
        ("blocks.ts", oracle!("ecma/suppress/eslint/blocks.ts")),
        ("lines.ts", oracle!("ecma/suppress/eslint/lines.ts")),
        (
            "not-directives.ts",
            oracle!("ecma/suppress/eslint/not-directives.ts"),
        ),
        ("jsx.tsx", oracle!("ecma/suppress/eslint/jsx.tsx")),
        (
            "terminators.ts",
            oracle!("ecma/suppress/eslint/terminators.ts"),
        ),
    ];
    let suppress = json!({ "eslint": { "rules": ["test/every-name"] } });
    let mut report = Vec::new();
    for (fixture, source) in fixtures {
        let expected = oracle[fixture]
            .as_array()
            .unwrap_or_else(|| panic!("{fixture} in the oracle"))
            .iter()
            .map(|start| start.as_u64().expect("start"))
            .collect::<Vec<_>>();
        let every = starts(fixture, source, None);
        assert!(
            expected.iter().all(|start| every.contains(start)),
            "{fixture}: the port reports every name ESLint keeps"
        );
        let ours = starts(fixture, source, Some(suppress.clone()));
        if ours != expected {
            report.push(format!("{fixture}: ours {ours:?}, ESLint's {expected:?}"));
        }
    }
    assert_eq!(report, Vec::<String>::new(), "\n{}", report.join("\n"));
    // A directive for another name of the rule suppresses nothing.
    let other = json!({ "eslint": { "rules": ["other/rule"] } });
    let source = oracle!("ecma/suppress/eslint/lines.ts");
    let every = starts("lines.ts", source, None);
    let named = starts("lines.ts", source, Some(other));
    assert!(named.len() < every.len() && named.len() > 10, "{named:?}");
}
