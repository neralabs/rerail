//! Exact native truth, capture transactions, reached calls and remaining finite work.
use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl, sql::relational::bound_leaf::trace,
    test_support::CAPABILITIES,
};
use serde_json::{Value, json};

fn dialect_policy(language: &str, rules: &[Value], utils: Value) -> CompiledCodeRuleDsl {
    let clauses: Vec<_> = rules.iter().enumerate().map(|(i,rule)| json!({
        "id":format!("c{i}"),"forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},
        "violation":{"code":format!("C{i}"),"message":"match","anchor":"witness.span"}
    })).collect();
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"capture-predicate-dialect","revision":1,"mode":"steer","clauses":clauses}).to_string(), "json".into(), CAPABILITIES.into()).unwrap()
}

fn dialect_budgets(language: &str, rules: &[Value], sources: &[&str]) {
    let p = dialect_policy(language, rules, json!({}));
    for work in 0..=16 {
        check_work(&p, rules, sources, None, work);
    }
    check_work(&p, rules, sources, None, i64::MAX);
}

fn budgets(rules: &[Value], utils: Value, sources: &[&str]) {
    let p = policy_utils(rules, utils);
    for work in 0..=16 {
        check_work(&p, rules, sources, None, work);
    }
    check_work(&p, rules, sources, None, i64::MAX);
}

#[test]
fn capture_layout_reads_present_and_missing_inputs_with_token_edges() {
    let rules = [
        json!({"kind":"number","all":[{"any":[{"regex":".*"},{"pattern":"$X"}]},{"layout":{"before":{"to":"token","edge":true,"breaks":{"eq":0}},"after":{"to":"capture.X","breaks":{"eq":0}}}}]}),
        json!({"kind":"number","all":[{"any":[{"regex":".*"},{"pattern":"$X"}]},{"layout":{"before":{"to":"capture.X","breaks":{"eq":0}}}}]}),
        json!({"kind":"arguments","all":[{"has":{"kind":"number","pattern":"$X","stopBy":"neighbor"}},{"has":{"kind":"number","layout":{"before":{"to":"capture.X","blank":{"min":0}}},"stopBy":"neighbor"}}]}),
    ];
    budgets(&rules, json!({}), &["1; f(2,3);\n", "\n4; f(5,\n\n6);\n"]);
}

#[test]
fn capture_comparisons_match_native_text_node_index_and_semantic_unknowns() {
    for mode in [
        "source", "unquoted", "tokens", "node", "index", "value", "key",
    ] {
        let rules = [
            json!({"kind":"call_expression","all":[{"pattern":"pair($L,$R)"},{"compare":{"left":"L","op":"eq","right":"R","text":mode}}]}),
        ];
        budgets(
            &rules,
            json!({}),
            &[
                "pair(1,1); pair(1,2); pair(x,y); pair('a',\"a\");",
                "pair(obj.x,obj.x); pair([1],[1]);",
            ],
        );
    }
    // Structural modes are language syntax comparisons, admitted for Python/PHP.
    for mode in ["structure", "structure-hash"] {
        let rules = [
            json!({"kind":"call","all":[{"pattern":"pair($L,$R)"},{"compare":{"left":"L","op":"eq","right":"R","text":mode}}]}),
        ];
        dialect_budgets(
            "python",
            &rules,
            &[
                "pair(1, 0x1)\npair(True, 1)\npair(1, 1.0)\npair('a', \"a\")\n",
                "pair([1, 2], [1, 2])\npair(obj.x, obj.x)\npair([1], [2])\n",
            ],
        );
    }
    // Deduplicated input names and absent inputs must retain native semantics.
    budgets(
        &[
            json!({"kind":"number","all":[{"pattern":"$X"},{"compare":{"left":"X","op":"eq","right":"X","text":"node"}}]}),
        ],
        json!({}),
        &["1;", "2;"],
    );
    budgets(
        &[
            json!({"kind":"number","all":[{"any":[{"regex":".*"},{"pattern":"$X"}]},{"compare":{"left":"X","op":"eq","right":"X","text":"source"}}]}),
        ],
        json!({}),
        &["1;", "2;"],
    );
}

#[test]
fn commuted_token_comparison_spends_native_work_and_rolls_back_branches() {
    let comparison =
        json!({"compare":{"left":"L","op":"eq","right":"R","text":"tokens","commute":["+","*"]}});
    let rules = [
        json!({"kind":"call_expression","all":[{"pattern":"pair($L,$R)"},{"any":[{"all":[comparison,{"regex":"^never$"}]},{"compare":{"left":"L","op":"ne","right":"R","text":"node"}}]}]}),
    ];
    budgets(
        &rules,
        json!({}),
        &[
            "pair(a+b,b+a); pair((a+b)*c,c*(b+a));",
            "pair(a+b,a+c); pair(x,x);",
        ],
    );
}

#[test]
fn capture_predicate_utilities_preserve_memo_costs_and_clause_file_ownership() {
    for mode in ["source", "value", "tokens"] {
        let rules = [
            json!({"kind":"call_expression","all":[{"pattern":"pair($L,$R)"},{"matches":"same"},{"not":{"not":{"matches":"same"}}},{"matches":"same"}]}),
            json!({"kind":"call_expression","all":[{"pattern":"pair($L,$R)"},{"not":{"matches":"same"}}]}),
        ];
        let utils = json!({"same":{"compare":{"left":"L","op":"eq","right":"R","text":mode}}});
        let p = policy_utils(&rules, utils.clone());
        check_work(
            &p,
            &rules,
            &["pair(1,1); pair(1,2);", "pair(2,2);"],
            Some("2"),
            i64::MAX,
        );
        budgets(
            &rules,
            utils,
            &["pair(1,1); pair(1,2); pair(x,y);", "pair(2,2); pair(2,3);"],
        );
    }
    let rules = [
        json!({"kind":"arguments","all":[{"has":{"kind":"number","pattern":"$X","stopBy":"neighbor"}},{"has":{"kind":"number","all":[{"matches":"gap"},{"matches":"gap"}],"stopBy":"neighbor"}}]}),
    ];
    budgets(
        &rules,
        json!({"gap":{"layout":{"before":{"to":"capture.X","blank":{"min":0}}}}}),
        &["f(1,\n\n2);", "f(3,4);"],
    );
}

#[test]
fn unreached_capture_predicates_make_no_native_calls() {
    let rules = [
        json!({"kind":"number","any":[{"regex":".*"},{"all":[{"pattern":"$X"},{"compare":{"left":"X","op":"eq","right":"X","text":"value"}},{"layout":{"before":{"to":"capture.X","blank":{"min":0}}}}]}]}),
    ];
    let p = policy_utils(&rules, json!({}));
    let (_, calls) = trace(|| check_work(&p, &rules, &["1;2;", "3;"], None, 0));
    assert!(calls.is_empty(), "{calls:?}");
}

#[test]
fn capture_predicate_demand_does_not_grow_with_unrelated_nodes_or_repeat_inputs() {
    let rules = [
        json!({"kind":"call_expression","all":[{"pattern":"pair($L,$R)"},{"compare":{"left":"L","op":"eq","right":"R","text":"value"}}]}),
    ];
    let p = policy_utils(&rules, json!({}));
    let mut expected_calls = None;
    for count in [32, 128, 512] {
        let mut source = String::from("pair(1,1); pair(2,3);\n");
        for i in 0..count {
            source.push_str(&format!("let unrelated{i}={i};\n"));
        }
        let (report, calls) = trace(|| check_work(&p, &rules, &[&source], None, i64::MAX));
        assert_eq!(report["output"].as_array().unwrap().len(), 1);
        assert_eq!(
            calls.len(),
            2,
            "each reached comparison runs once: {calls:?}"
        );
        if let Some(prior) = expected_calls {
            assert_eq!(calls.len(), prior);
        } else {
            expected_calls = Some(calls.len());
        }
    }
}
