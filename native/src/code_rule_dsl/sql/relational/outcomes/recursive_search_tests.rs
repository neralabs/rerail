//! Native parity for SQL recursive searches whose base is an ordinary outcome plan.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl, test_support::CAPABILITIES};
use serde_json::{Value, json};

fn policy(language: &str, rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            json!({
                "id":format!("c{index}"),
                "forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},
                "violation":{"code":format!("C{index}"),"message":"match","anchor":"witness.span"}
            })
        })
        .collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"recursive-search","revision":1,"mode":"steer","clauses":clauses}).to_string(), "json".into(), CAPABILITIES.into()).unwrap()
}

fn utilities(bases: &[Value], guard: Value) -> Value {
    let mut arms = bases.to_vec();
    arms.push(json!({"all":[guard,{"has":{"stopBy":"neighbor","matches":"u"}}]}));
    json!({"u":{"any":arms},"wrapper":{"matches":"u"}})
}

fn check(language: &str, rules: &[Value], utils: Value, sources: &[&str], work: i64) -> Value {
    let policy = policy(language, rules, utils);
    let report = check_work(&policy, rules, sources, None, work);
    assert_eq!(report["metrics"]["execution"]["matchingStatements"], 1);
    assert_eq!(report["metrics"]["framePublications"], 0);
    report
}

#[test]
fn recursive_semantic_wrapper_preserves_scope_fill_and_repeated_call_work() {
    let utils = utilities(
        &[json!({"kind":"identifier","resolves":{"global":["print"]}})],
        json!({"kind":"list"}),
    );
    for body in [
        json!({"matches":"wrapper"}),
        json!({"all":[{"matches":"wrapper"},{"matches":"wrapper"}]}),
        json!({"any":[{"matches":"wrapper"},{"matches":"wrapper"}]}),
        json!({"not":{"matches":"wrapper"}}),
    ] {
        let rule = json!({"kind":"list","all":[body]});
        for work in [0, 1, 2, 3, 4, 8, 16, 64, 1024] {
            check(
                "python",
                std::slice::from_ref(&rule),
                utils.clone(),
                &[
                    "a = [unknown, [print], print]\nb = [[unknown], 0]\n",
                    "def f(print):\n    return [[print]]\na = [print]\n",
                ],
                work,
            );
        }
    }
}

#[test]
fn recursive_search_checks_every_base_arm_before_continuing() {
    let utils = utilities(
        &[
            json!({"kind":"integer","regex":"^1$"}),
            json!({"kind":"integer","regex":"^2$"}),
            json!({"kind":"identifier","resolves":{"global":["print"]}}),
        ],
        json!({"kind":"list"}),
    );
    let rules = [
        json!({"kind":"list","matches":"u"}),
        json!({"kind":"list","matches":"wrapper"}),
    ];
    for work in [0, 1, 2, 3, 5, 8, 16, 128, 1024] {
        check(
            "python",
            &rules,
            utils.clone(),
            &["a=[0,[3,2],1]\nb=[unknown,[print]]\nc=[[0],3]\n"],
            work,
        );
    }
}

#[test]
fn recursive_capture_bases_rollback_false_arms_and_commit_first_success() {
    let utils = utilities(
        &[
            // Binding X on a failed base must not constrain the recursive continuation.
            json!({"all":[{"pattern":"$X"},{"regex":"^never$"}]}),
            json!({"kind":"identifier","regex":"^yes$","pattern":"$X"}),
        ],
        json!({"kind":"array"}),
    );
    for body in [
        json!({"matches":"u"}),
        json!({"all":[{"matches":"u"},{"matches":"u"}]}),
        json!({"any":[{"matches":"u"},{"pattern":"$Y"}]}),
    ] {
        let rule = json!({"kind":"array","all":[body]});
        for work in 0..=20 {
            check(
                "typescript",
                std::slice::from_ref(&rule),
                utils.clone(),
                &["[no,[no,yes],yes]; [no,[no]];"],
                work,
            );
        }
    }
}

#[test]
fn recursive_search_preserves_outer_unknown_capture_transactions() {
    let utils = utilities(
        &[json!({"kind":"identifier","regex":"^yes$","pattern":"$X"})],
        json!({"kind":"array"}),
    );
    for body in [
        json!({"any":[{"all":[{"testUnknown":".*"},{"matches":"u"}]},{"matches":"wrapper"}]}),
        json!({"all":[{"testUnknown":"^never$"},{"matches":"u"}]}),
        json!({"any":[{"all":[{"pattern":"$Y"},{"testUnknown":".*"}]},{"matches":"u"}]}),
    ] {
        let rule = json!({"kind":"array","all":[body]});
        for work in [0, 1, 2, 3, 4, 8, 16, 64] {
            check(
                "typescript",
                std::slice::from_ref(&rule),
                utils.clone(),
                &["[u,yes]; [[u],yes]; [no,[yes],u]; [u,no];"],
                work,
            );
        }
    }
}

#[test]
fn recursive_semantic_success_leaves_deep_suffix_unreached() {
    let utils = utilities(
        &[json!({"kind":"identifier","resolves":{"global":["print"]}})],
        json!({"kind":"list"}),
    );
    let rules = [json!({"kind":"list","regex":"^\\[print,","matches":"wrapper"})];
    let source = format!("a=[print,{}0{}]\n", "[".repeat(160), "]".repeat(160));
    for work in [0, 1, 2, 4, 16, 1024, 10000] {
        check("python", &rules, utils.clone(), &[&source], work);
    }
}

#[test]
fn recursive_semantic_bases_compose_pure_and_semantic_components() {
    let utils = json!({
        "pure":{"any":[{"regex":"^yes$"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"pure"}}]},
        "inner":{"any":[{"kind":"identifier","resolves":{"global":["print"]}},{"kind":"list","has":{"stopBy":"neighbor","matches":"inner"}}]},
        "u":{"any":[{"matches":"pure"},{"matches":"inner"},{"kind":"tuple","has":{"stopBy":"neighbor","matches":"u"}}]}
    });
    let rules = [json!({"kind":"tuple","matches":"u"})];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 1024] {
        check(
            "python",
            &rules,
            utils.clone(),
            &["a=(no, ((yes)), [print])\nb=((no,), [unknown], ([print],))\nc=(unknown, [no])\n"],
            work,
        );
    }
}

#[test]
fn recursive_semantic_sql_and_vm_scale_with_reached_input() {
    let utils = utilities(
        &[json!({"kind":"identifier","resolves":{"global":["print"]}})],
        json!({"kind":"list"}),
    );
    let rules = [json!({"kind":"list","matches":"u"})];
    let mut reports = vec![];
    for width in [8, 32, 128] {
        let source = format!("a=[{},print]\n", vec!["unknown"; width].join(","));
        let report = check("python", &rules, utils.clone(), &[&source], i64::MAX);
        reports.push(json!({"width":width,"vm":report["metrics"]["execution"]["vmSteps"],"sqlBytes":report["sql"].as_str().unwrap().len(),"metrics":report["metrics"],"output":report["output"]}));
    }
    assert_eq!(reports[0]["sqlBytes"], reports[2]["sqlBytes"]);
    let v0 = reports[0]["vm"].as_u64().unwrap();
    let v1 = reports[1]["vm"].as_u64().unwrap();
    let v2 = reports[2]["vm"].as_u64().unwrap();
    assert!(v1 < v0 * 6 && v2 < v1 * 6, "{reports:?}");
    if let Ok(path) = std::env::var("RERAIL_RECURSIVE_SEARCH_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}

#[test]
fn memoized_recursive_value_search_publishes_completed_siblings_and_replays_roots() {
    let utils = utilities(
        &[json!({"kind":"integer","value":{"eq":1}})],
        json!({"kind":"list"}),
    );
    for body in [
        json!({"matches":"u"}),
        json!({"all":[{"matches":"wrapper"},{"matches":"wrapper"}]}),
        json!({"not":{"matches":"wrapper"}}),
    ] {
        let rules = [json!({"kind":"list","all":[body]})];
        for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 1024, 10000] {
            check(
                "python",
                &rules,
                utils.clone(),
                &["a=[[2,3],[4,[1]],5]\nb=[[2],[3]]\nc=[1,[[4]]]\n"],
                work,
            );
        }
    }
}

#[test]
fn memoized_recursive_capture_search_rolls_back_bases_and_exports_first_success() {
    let utils = utilities(
        &[json!({"kind":"integer","all":[{"pattern":"$X"},{"value":{"eq":1}}]})],
        json!({"kind":"list"}),
    );
    let rules = [json!({"kind":"list","any":[{"matches":"u"},{"matches":"wrapper"}]})];
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 128, 1024, 10000] {
        check(
            "python",
            &rules,
            utils.clone(),
            &["a=[[2,3],[4,1],1]\nb=[[2],[3]]\n"],
            work,
        );
    }
}

#[test]
fn memoized_recursive_nested_siblings_seek_past_completed_subtrees() {
    let utils = utilities(
        &[json!({"kind":"integer","value":{"eq":1}})],
        json!({"kind":"list"}),
    );
    let rules = [json!({"kind":"list","regex":"^\\[\\[","matches":"u"})];
    let mut reports = Vec::new();
    for width in [8, 32, 128] {
        let source = format!("a=[{},[1]]\n", vec!["[2,3]"; width].join(","));
        let report = check("python", &rules, utils.clone(), &[&source], i64::MAX);
        reports.push(json!({"width":width,"sqlBytes":report["sql"].as_str().unwrap().len(),"vm":report["metrics"]["execution"]["vmSteps"],"metrics":report["metrics"],"output":report["output"],"sql":report["sql"]}));
    }
    assert_eq!(reports[0]["sqlBytes"], reports[2]["sqlBytes"]);
    for pair in reports.windows(2) {
        assert!(
            pair[1]["vm"].as_u64().unwrap() < pair[0]["vm"].as_u64().unwrap() * 5,
            "nested sibling work amplified: {reports:?}"
        );
    }
    if let Ok(path) = std::env::var("RERAIL_MEMO_RECURSIVE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}
