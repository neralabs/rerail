//! Native differential coverage for total, capture-free linear walks.
use super::super::tests::{measure, policy};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};

fn checked(rule: Value, sources: &[(String, String)]) -> Value {
    let policy = policy(rule, json!({}));
    let result = measure(&grammar_matchers(&policy)[0], sources);
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
    result
}

#[test]
fn relational_pure_catalog_walks_match_native_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("packs/python");
    let cases = [
        ("ruff-b021-f-string-docstring", "flake8_bugbear/B021"),
        (
            "ruff-b026-star-arg-unpacking-after-keyword-arg",
            "flake8_bugbear/B026",
        ),
        ("ruff-f701-break-outside-loop", "pyflakes/F701"),
        ("ruff-f702-continue-outside-loop", "pyflakes/F702"),
        ("ruff-f706-return-outside-function", "pyflakes/F706"),
        ("ruff-f707-default-except-not-last", "pyflakes/F707"),
        ("ruff-ret502-implicit-return-value", "flake8_return/RET502"),
        (
            "ruff-ret505-superfluous-else-return",
            "flake8_return/RET505",
        ),
        ("ruff-ret506-superfluous-else-raise", "flake8_return/RET506"),
        (
            "ruff-ret507-superfluous-else-continue",
            "flake8_return/RET507",
        ),
        ("ruff-ret508-superfluous-else-break", "flake8_return/RET508"),
        (
            "ruff-sim107-return-in-try-except-finally",
            "flake8_simplify/SIM107",
        ),
    ];
    for (name, fixture) in cases {
        let compiled = CodeRuleDslRuntime::compile(
            std::fs::read_to_string(root.join(format!("rules/{name}.json"))).unwrap(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        let code = fixture.rsplit('/').next().unwrap();
        let sources = [
            (
                format!("{code}.py"),
                std::fs::read_to_string(root.join(format!("upstream/ruff/fixtures/{fixture}.py")))
                    .unwrap(),
            ),
            (
                format!("noqa/{code}.py"),
                std::fs::read_to_string(
                    root.join(format!("upstream/ruff/fixtures/rerail/noqa/{code}.py")),
                )
                .unwrap(),
            ),
            ("empty.py".into(), String::new()),
        ];
        for grammar in grammar_matchers(&compiled) {
            let result = measure(grammar, &sources);
            assert!(result["matches"].as_u64().unwrap() > 0, "{name}");
            assert_eq!(result["matchingStatements"], 1, "{name}");
            assert_eq!(result["framePublications"], 0, "{name}");
        }
    }
}

#[test]
fn relational_pure_sibling_walks_preserve_inclusive_stops_and_direction() {
    let sources = [("siblings.py".into(), "f(a, b, c)\nf(c, b, a)\n".into())];
    for direction in ["precedes", "follows"] {
        for (stop, expected) in [
            (json!("end"), 1),
            (json!({"regex":"^c$"}), 1),
            (json!({"regex":"^b$"}), 0),
            (json!({"regex":"^,$"}), 0),
        ] {
            let result = checked(
                json!({"kind":"identifier","regex":"^a$",
                    direction:{"stopBy":stop,"kind":"identifier","regex":"^c$"}}),
                &sources,
            );
            assert_eq!(result["matches"], expected, "{direction}, {stop}");
        }
        let anonymous = checked(
            json!({"kind":"identifier","regex":"^a$",
                direction:{"stopBy":{"regex":"^,$"},"regex":"^,$"}}),
            &sources,
        );
        assert_eq!(
            anonymous["matches"], 1,
            "{direction}: anonymous match at stop"
        );
        let self_match = checked(
            json!({"kind":"identifier","regex":"^a$",
                direction:{"stopBy":"end","regex":"^a$"}}),
            &sources,
        );
        assert_eq!(self_match["matches"], 0, "{direction}: root is excluded");
    }
}

#[test]
fn relational_pure_ancestor_walks_keep_nearest_stop_and_exclude_self() {
    let sources = [(
        "ancestors.py".into(),
        "for item in xs:\n    def barrier():\n        x\n    x\nx\n".into(),
    )];
    for (target, stop, expected) in [
        ("for_statement", json!("end"), 2),
        ("for_statement", json!({"kind":"function_definition"}), 1),
        (
            "function_definition",
            json!({"kind":"function_definition"}),
            1,
        ),
        ("for_statement", json!({"kind":"for_statement"}), 2),
    ] {
        let result = checked(
            json!({"kind":"identifier","regex":"^x$",
                "inside":{"kind":target,"stopBy":stop}}),
            &sources,
        );
        assert_eq!(result["matches"], expected, "{target}, {stop}");
    }
    let root_is_not_its_own_ancestor = checked(
        json!({"kind":"for_statement", "inside":{
            "kind":"for_statement","stopBy":{"kind":"for_statement"}}}),
        &sources,
    );
    assert_eq!(root_is_not_its_own_ancestor["matches"], 0);
}

#[test]
fn relational_pure_walks_keep_files_and_parent_lists_separate() {
    let sources = [
        ("a.py".into(), "f(a)\n".into()),
        ("c.py".into(), "f(c)\n".into()),
        ("separate.py".into(), "f(a)\nf(c)\n".into()),
        ("empty.py".into(), String::new()),
    ];
    for direction in ["follows", "precedes"] {
        for stop in [json!("end"), json!({"kind":"comment"})] {
            let rule = json!({"kind":"identifier","regex":"^a$",
                direction:{"regex":"^c$","stopBy":stop}});
            assert_eq!(checked(rule.clone(), &sources)["matches"], 0);
            let negative = json!({"kind":"identifier","regex":"^a$","not":rule});
            assert_eq!(checked(negative, &sources)["matches"], 2);
        }
    }
}
