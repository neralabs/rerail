//! Composite pattern captures must preserve native witness identity through SQL composition.
use super::publication_tests::{check_work, policy};
use crate::code_rule_dsl::{
    language::GrammarDialect, matcher::node_at, sql::tests::differential::tape,
    test_support::grammar_matchers,
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

fn check(rules: &[Value], sources: &[&str]) -> Value {
    check_work(&policy(rules), rules, sources, None, i64::MAX)
}

fn text(source: &str, id: &Value) -> String {
    let tree = AstGrep::doc(tape(GrammarDialect::TypeScript, source));
    node_at(&tree, id.as_u64().unwrap() as u32)
        .text()
        .into_owned()
}

#[test]
fn composed_patterns_export_nested_witnesses_and_repeated_variable_identity() {
    let rules = [
        json!({"kind":"call_expression","pattern":"f($X)"}),
        json!({"kind":"call_expression","pattern":"pair($X, $X)"}),
    ];
    let sources = [
        "f(a + b); f({key: 1}); pair(obj.x, obj.x); pair(obj.x, other.x);",
        "pair(a, a); pair(a, b); f([a, b]);",
    ];
    let result = check(&rules, &sources);
    let output = result["output"].as_array().unwrap();
    assert_eq!(output.len(), 5);
    for row in output {
        assert_ne!(
            row[2], row[3][0],
            "pattern capture must be its argument, not the call root"
        );
    }
    let captured = output
        .iter()
        .map(|row| text(sources[row[1].as_u64().unwrap() as usize], &row[3][0]))
        .collect::<Vec<_>>();
    assert_eq!(captured, ["a + b", "{key: 1}", "[a, b]", "obj.x", "a"]);
}

#[test]
fn composed_patterns_consume_bound_capture_during_ellipsis_alignment() {
    let rules = [json!({"kind":"call_expression","all":[
        {"has":{"kind":"arguments","stopBy":"neighbor","has":{"kind":"identifier","regex":"^b$","pattern":"$X","stopBy":"neighbor"}}},
        {"pattern":"f($$$, $X, $$$)"}
    ]})];
    let sources = ["f(a, b, c); f(b, a, b); g(a, b, c);", "f(c, b, a);"];
    let result = check(&rules, &sources);
    assert_eq!(result["output"].as_array().unwrap().len(), 3);
    for row in result["output"].as_array().unwrap() {
        assert_eq!(
            text(sources[row[1].as_u64().unwrap() as usize], &row[3][0]),
            "b"
        );
    }
}

#[test]
fn composed_patterns_rollback_failed_branches_and_private_negation_bindings() {
    for rule in [
        json!({"kind":"call_expression","any":[{"all":[{"pattern":"f($X)"},{"regex":"^never$"}]},{"pattern":"f($Y)"}]}),
        json!({"kind":"call_expression","all":[{"not":{"all":[{"pattern":"f($X)"},{"regex":"^never$"}]}},{"pattern":"f($Y)"}]}),
    ] {
        let sources = ["f(a); f(obj.x); g(b);", "f([z]);"];
        let result = check(&[rule], &sources);
        assert_eq!(result["output"].as_array().unwrap().len(), 3);
        for row in result["output"].as_array().unwrap() {
            let captures = row[3].as_array().unwrap();
            assert!(captures.last().unwrap().is_number());
            assert!(captures[..captures.len() - 1].iter().all(Value::is_null));
        }
    }
}

#[test]
fn composed_patterns_preserve_unknown_binding_barriers() {
    let sources = ["f(u); f(a); g(b);", "f(u2); g(c);"];
    for rule in [
        json!({"kind":"call_expression","any":[{"all":[{"pattern":"f($X)"},{"testUnknown":"^f\\(u"}]},{"pattern":"g($Y)"}]}),
        json!({"kind":"call_expression","all":[{"pattern":"f($X)"},{"not":{"all":[{"pattern":"f($X)"},{"testUnknown":"^f\\(u"},{"regex":"^never$"}]}}]}),
    ] {
        let result = check(&[rule], &sources);
        assert!(!result["output"].as_array().unwrap().is_empty());
    }
}

#[test]
fn composed_patterns_stopped_walks_keep_selected_and_private_witnesses_separate() {
    for stop in [json!("end"), json!({"pattern":"barrier($PRIVATE)"})] {
        let rule = json!({"kind":"statement_block","has":{"kind":"call_expression","pattern":"target($X)","stopBy":stop}});
        let sources = [
            "{ barrier(a); target(b); } { target(c); barrier(d); }",
            "{ wrap(target(e)); } { empty(); }",
        ];
        let result = check(&[rule], &sources);
        assert!(!result["output"].as_array().unwrap().is_empty());
        for row in result["output"].as_array().unwrap() {
            assert_eq!(
                row[3].as_array().unwrap().len(),
                1,
                "stop captures must not escape"
            );
        }
    }
}

#[test]
fn composed_patterns_keep_failure_publication_and_remaining_allowances() {
    let rules = [
        json!({"kind":"call_expression","all":[{"pattern":"f($X)"},{"has":{"kind":"arguments","stopBy":"neighbor","has":{"kind":"number","pattern":"$X","value":{"gt":0},"stopBy":"neighbor"}}}]}),
        json!({"kind":"call_expression","pattern":"g($Y)"}),
    ];
    let sources = ["f(1); f(2); g(3);", "f(4); g(5);"];
    let compiled = policy(&rules);
    for allowance in 0..=4 {
        check_work(&compiled, &rules, &sources, None, allowance);
    }
    let result = check_work(&compiled, &rules, &sources, Some("2"), i64::MAX);
    let output = result["output"].as_array().unwrap();
    assert!(
        output
            .iter()
            .any(|row| row[0] == 0 && row[1] == 0 && row[4] == -4)
    );
    assert!(
        !output
            .iter()
            .any(|row| row[0] == 0 && row[1] == 0 && row[4] == 0)
    );
    assert!(
        output
            .iter()
            .any(|row| row[0] == 1 && row[1] == 0 && row[4] == 0)
    );
    assert!(
        output
            .iter()
            .any(|row| row[0] == 0 && row[1] == 1 && row[4] == 0)
    );
}

#[test]
fn composed_patterns_repeat_without_cross_file_or_execution_capture_leakage() {
    let rules = [json!({"kind":"call_expression","pattern":"pair($X, $X)"})];
    let p = policy(&rules);
    let sources = [
        "pair(a,a); pair(a,b);",
        "pair(b,b);",
        "pair(a,a); pair(a,b);",
    ];
    let first = check_work(&p, &rules, &sources, None, i64::MAX);
    let second = check_work(&p, &rules, &sources, None, i64::MAX);
    assert_eq!(first["output"], second["output"]);
    assert_eq!(first["output"].as_array().unwrap().len(), 3);
    let reversed = [sources[1], sources[0], sources[2]];
    let result = check_work(&p, &rules, &reversed, None, i64::MAX);
    assert_eq!(result["output"].as_array().unwrap().len(), 3);
}

#[test]
fn composed_patterns_use_main_relational_compile_dispatch() {
    let p = super::tests::policy(
        json!({"kind":"call_expression","pattern":"f($X)"}),
        json!({}),
    );
    let grammar = &grammar_matchers(&p)[0];
    let result = super::super::tests::measure(
        grammar,
        &[
            ("a.ts".into(), "f(a+b); g(c);".into()),
            ("b.ts".into(), "f(obj.x);".into()),
        ],
    );
    assert_eq!(result["matches"], 2);
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
}

// This deliberately omits an explicit DSL `kind`: the pattern itself proves
// that only call_expression roots can match. Unrelated AST growth must not
// increase either root demand or atomic pattern requests.
#[test]
fn composed_pattern_root_kind_demand_does_not_grow_with_unrelated_nodes() {
    use crate::code_rule_dsl::{sql::relational::tests::measure, test_support::grammar_matchers};
    use serde_json::json;

    let compiled = super::tests::policy(json!({"pattern":"f($X)"}), json!({}));
    let grammars = grammar_matchers(&compiled);
    let grammar = &grammars[0];
    assert!(
        !grammar.three_valued,
        "exercise the nonfallible standalone compile path"
    );

    // Collect all samples before checking physical gates so the failing old
    // operator records its scaling at every size. `measure` independently
    // requires exact native root/capture parity for each sample.
    let samples = [32, 128, 512]
        .into_iter()
        .map(|unrelated_declarations| {
            let mut source = String::from("f(a); g(b);\n");
            for index in 0..unrelated_declarations {
                source.push_str(&format!("let unused{index} = {index};\n"));
            }
            let measured = measure(grammar, &[("pattern-demand.ts".into(), source)]);
            json!({
                "unrelatedDeclarations": unrelated_declarations,
                "parity": measured["parity"],
                "matches": measured["matches"],
                "patternRequests": measured["patternRequests"],
                "nodeFilters": measured["nodeFilters"],
                "nodeAdvances": measured["nodeAdvances"],
                "matchingStatements": measured["matchingStatements"],
                "framePublications": measured["framePublications"],
                "vmSteps": measured["vmSteps"],
                "sql": measured["sql"],
                "explain": measured["explain"],
            })
        })
        .collect::<Vec<_>>();
    eprintln!("PATTERN_KIND_SCALING {}", json!(samples));

    for sample in &samples {
        assert_eq!(sample["parity"], true, "{sample}");
        assert_eq!(sample["matches"], 1, "positive f(a) witness: {sample}");
        assert_eq!(
            sample["patternRequests"], 2,
            "only f(a) and g(b) are pattern candidates: {sample}"
        );
        assert_eq!(sample["nodeFilters"], 1, "one root kind scan: {sample}");
        assert_eq!(
            sample["nodeAdvances"], 2,
            "root scan must visit only the two calls: {sample}"
        );
        assert_eq!(sample["matchingStatements"], 1, "{sample}");
        assert_eq!(sample["framePublications"], 0, "{sample}");
    }
}
