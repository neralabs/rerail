//! Capture identity and rollback through monotone recursive utility components.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
use serde_json::{Value, json};

fn b020() -> (crate::code_rule_dsl::CompiledCodeRuleDsl, Vec<Value>) {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../packs/python/rules/ruff-b020-loop-variable-overrides-iterator.json"
    ));
    let input: Value = serde_json::from_str(source).unwrap();
    let rules = vec![input["clauses"][0]["forbid"]["ast"]["rule"].clone()];
    (
        CodeRuleDslRuntime::compile(source.into(), "json".into(), CAPABILITIES.into()).unwrap(),
        rules,
    )
}

#[test]
fn recursive_capture_single_component_preserves_mutation_and_branch_rollback() {
    let mut input: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../packs/python/rules/ruff-b020-loop-variable-overrides-iterator.json"
    )))
    .unwrap();
    let rules = vec![json!({"kind":"list","matches":"mentions"})];
    input["clauses"][0]["forbid"]["ast"]["rule"] = rules[0].clone();
    let policy =
        CodeRuleDslRuntime::compile(input.to_string(), "json".into(), CAPABILITIES.into()).unwrap();
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 10_000] {
        check_work(
            &policy,
            &rules,
            &[
                "a=[1,[x,y],z]\nb=[lambda x: [x], obj.field]\n",
                "a=[[x for x in y], [x for x in z]]\nb=[lambda x: x, lambda y: [y]]\n",
            ],
            None,
            work,
        );
    }
}

#[test]
fn recursive_capture_entry_exports_do_not_withhold_another_members_private_names() {
    let utils = json!({
        "u":{"not":{"has":{"stopBy":"neighbor","matches":"v"}}},
        "v":{"any":[{"all":[{"kind":"identifier"},{"pattern":"$Q"},{"pattern":"$P"}]},{"matches":"u"}]}
    });
    let rules = [json!({"kind":"identifier","all":[
        {"all":[{"value":{"gt":0}},{"pattern":"$Q"}]},
        {"matches":"u"},
        {"all":[{"pattern":"$P"},{"value":{"gt":0}}]}
    ]})];
    let policy = super::publication_tests::policy_utils(&rules, utils);
    for work in [0, 1, 2, 3, 4, 8, 32, 1000] {
        let report = check_work(&policy, &rules, &["x; y;"], None, work);
        if work == 1000 {
            assert_eq!(report["sqlCalls"].as_array().unwrap().len(), 4);
        }
    }
}

#[test]
fn recursive_captures_preserve_identity_aliases_shadowing_and_reached_work() {
    let (policy, rules) = b020();
    let sources = [
        "for x in x:\n pass\nfor (x, x) in [x]:\n pass\nfor (x,y) in [x,y]:\n pass\n",
        "for x in (lambda x: x):\n pass\nfor x in (lambda x: [x]):\n pass\nfor x in (lambda x: [x], x):\n pass\n",
        "for x in [y for y in x]:\n pass\nfor x in [x for x in y]:\n pass\nfor x in obj.x:\n pass\n",
    ];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 512, 10_000] {
        check_work(&policy, &rules, &sources, None, work);
    }
}

#[test]
fn recursive_capture_depth_and_work_failures_rollback_every_provisional_finding() {
    let (policy, rules) = b020();
    // The native physical-stack guard precedes its logical depth bound on
    // sufficiently nested comprehensions. That separate gap is recorded below.
    for depth in [4, 24, 96] {
        let mut iterable = "x".to_owned();
        for _ in 0..depth {
            iterable = format!("[0 for y in {iterable}]");
        }
        let source = format!("for x in {iterable}:\n pass\nfor x in x:\n pass\n");
        for work in [0, 1, 4, 16, 128, i64::MAX] {
            check_work(&policy, &rules, &[&source], None, work);
        }
    }
}

#[test]
fn recursive_capture_native_physical_stack_gap_is_explicit_diagnostic_evidence() {
    let (policy, rules) = b020();
    let mut iterable = "x".to_owned();
    for _ in 0..260 {
        iterable = format!("[0 for y in {iterable}]");
    }
    let source = format!("for x in {iterable}:\n pass\nfor x in x:\n pass\n");
    let observation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_work(&policy, &rules, &[&source], None, i64::MAX)
    }));
    let evidence = match observation {
        Ok(report) => {
            json!({"nativePhysicalStackQualified":false,"diagnosticOnly":true,"observedDifference":null,"output":report["output"]})
        }
        Err(error) => {
            let message = error
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|m| (*m).to_owned()))
                .expect("diagnostic panic text");
            let rows = message
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("left:")
                        || line.trim_start().starts_with("right:")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                rows.len(),
                2,
                "unexpected failure instead of physical-stack diagnostic: {message}"
            );
            assert!(
                rows.iter().all(|row| row.contains("Number(-1)")),
                "unexpected non-depth divergence: {message}"
            );
            json!({"nativePhysicalStackQualified":false,"diagnosticOnly":true,"observedDifference":"native physical stack guard is not modeled by SQL logical depth","nativeAndSqlRows":rows,"logicalDepthBound":256,"nativeStackByteBound":1048576})
        }
    };
    println!("CAPTURE_PHYSICAL_STACK_DIAGNOSTIC {evidence}");
    if let Ok(path) = std::env::var("RERAIL_CAPTURE_PHYSICAL_STACK_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}

#[test]
fn recursive_capture_width_keeps_sql_fixed_and_vm_growth_bounded() {
    let (policy, rules) = b020();
    let mut evidence = vec![];
    for width in [8, 16, 32] {
        let source = format!("for x in [{},x]:\n pass\n", vec!["[y]"; width].join(","));
        let report = check_work(&policy, &rules, &[&source], None, 100_000);
        evidence.push(json!({"width":width,"sourceBytes":source.len(),"sqlBytes":report["sql"].as_str().unwrap().len(),"metrics":report["metrics"],"output":report["output"],"nativeTraces":report["nativeTraces"],"relations":report["relations"]}));
    }
    for pair in evidence.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(
            pair[1]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
                <= 3 * pair[0]["metrics"]["execution"]["vmSteps"].as_u64().unwrap()
        );
    }
    if let Ok(path) = std::env::var("RERAIL_CAPTURE_RECURSIVE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}
#[test]
fn inherited_unknown_mode_does_not_force_memo_for_total_recursive_capture_bases() {
    use crate::code_rule_dsl::matcher::{AstMatcher, UtilRef};
    fn utility(m: &AstMatcher) -> Option<&UtilRef> {
        match m {
            AstMatcher::Util(u) => Some(u),
            AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().find_map(utility),
            _ => None,
        }
    }
    for local_unknown in [false, true] {
        let base = if local_unknown {
            json!({"kind":"identifier","value":{"gt":0}})
        } else {
            json!({"kind":"identifier","regex":"^base$"})
        };
        let utils = json!({"u":{"any":[base,{"kind":"identifier","pattern":"$P","regex":"^mark$"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}});
        for incoming_unknown in [false, true] {
            let root = if incoming_unknown {
                json!({"kind":"parenthesized_expression","all":[{"value":{"gt":0}},{"matches":"u"}]})
            } else {
                json!({"kind":"parenthesized_expression","any":[{"matches":"u"},{"regex":"^impossible$","value":{"gt":0}}]})
            };
            let rules = [root];
            let policy = super::publication_tests::policy_utils(&rules, utils.clone());
            let grammar = &crate::code_rule_dsl::test_support::grammar_matchers(&policy)[0];
            let mut lower = super::lower::Lower::new(grammar.language, true);
            let op = lower
                .recursive_search(utility(&grammar.matcher).unwrap())
                .unwrap()
                .unwrap();
            let super::Op::RecursiveSearch { component, .. } = op else {
                panic!("ordered recursive component");
            };
            assert_eq!(component.memo, local_unknown);
            for work in [0, 1, 2, 3, 4, 8, 16, 64, 1000] {
                check_work(
                    &policy,
                    &rules,
                    &["((base)); ((mark)); ((other));"],
                    None,
                    work,
                );
            }
        }
    }
}
