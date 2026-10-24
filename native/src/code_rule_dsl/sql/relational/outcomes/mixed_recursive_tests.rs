//! Native semantics, costs and probes for capture-free mixed recursive control.
use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::{matcher::AstMatcher, test_support::grammar_matchers};
use serde_json::{Value, json};
fn universal() -> Value {
    json!({"any":[{"kind":"number","value":{"gt":0}},{"kind":"identifier","value":{"gt":0}},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[\\[\\],]$"},{"matches":"u"}]}}}}]})
}
#[test]
fn mixed_universal_all_any_not_and_search_preserve_unknown_and_reached_work() {
    for negated in [false, true] {
        let utility = if negated {
            json!({"not":{"matches":"u"}})
        } else {
            json!({"matches":"u"})
        };
        let rules = [json!({"kind":"array","all":[utility]})];
        let p = policy_utils(&rules, json!({"u":universal()}));
        for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 256, 10000] {
            check_work(
                &p,
                &rules,
                &[
                    "f([1,[1,2],1]); f([x,0]); f([x,1]);",
                    "f([0,[x,1]]); f([]);",
                ],
                None,
                work,
            );
        }
    }
}
#[test]
fn mixed_mutual_scc_and_readonly_capture_regions_share_native_memo_answers() {
    let utils = json!({
     "u":{"any":[{"kind":"number","compare":{"left":"EXPECTED","op":"eq","right":"EXPECTED","text":"tokens"},"value":{"gt":0}},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[\\[\\],]$"},{"matches":"v"}]}}}}]},
     "v":{"matches":"u"}
    });
    let rules = [
        json!({"kind":"array","all":[{"pattern":"$EXPECTED"},{"matches":"u"},{"not":{"not":{"matches":"v"}}}]}),
    ];
    let p = policy_utils(&rules, utils);
    for work in [0, 1, 2, 3, 8, 16, 32, 64, 128, 10000] {
        check_work(&p, &rules, &["f([1,[1,2],1]); f([1,0]);"], None, work);
    }
}
#[test]
fn mixed_walk_directions_and_total_rule_stops_have_native_order() {
    for (axis, kind) in [
        ("inside", "parenthesized_expression"),
        ("follows", "expression_statement"),
        ("precedes", "expression_statement"),
    ] {
        for stop in [json!("end"), json!({"regex":"barrier"})] {
            let walk = json!({"stopBy":stop,"kind":kind,"matches":"u"});
            let mut recurse = json!({});
            recurse[axis] = walk;
            let positive = if axis == "inside" {
                json!({"kind":"number","value":{"gt":0}})
            } else {
                json!({"regex":"^1;?$","has":{"stopBy":"neighbor","kind":"number","value":{"gt":0}}})
            };
            let body = json!({"any":[{"kind":"identifier","value":{"gt":0}},positive,recurse,{"regex":"never"}]});
            let rules = [
                json!({"kind":if axis=="inside"{"identifier"}else{"expression_statement"},"matches":"u"}),
            ];
            let p = policy_utils(&rules, json!({"u":body}));
            for work in [0, 1, 2, 4, 8, 16, 64, 1024] {
                check_work(
                    &p,
                    &rules,
                    &["x; 0; 1; barrier;", "f((((x)))); barrier; 1;"],
                    None,
                    work,
                );
            }
        }
    }
}
#[test]
fn mixed_declines_unproved_or_capture_writing_components_without_lowering_side_effects() {
    for body in [
        json!({"any":[{"pattern":"$P"},{"kind":"array","not":{"has":{"not":{"matches":"u"}}}}]}),
        json!({"any":[{"value":{"gt":0}},{"resolves":{"binding":{"value":{"matches":"u"}}}}]}),
        json!({"any":[{"value":{"gt":0}},{"has":{"stopBy":"neighbor","inside":{"stopBy":"neighbor","matches":"u"}}}]}),
    ] {
        let p = policy_utils(&[json!({"matches":"u"})], json!({"u":body}));
        let g = &grammar_matchers(&p)[0];
        let AstMatcher::Util(u) = &g.matcher else {
            panic!("utility")
        };
        let mut lower = super::lower::Lower::new(g.language, true);
        assert!(lower.mixed_recursive(u).unwrap().is_none());
        assert!(lower.pure.program.terminals.is_empty());
        assert!(lower.slots.is_empty());
    }
    let p = policy_utils(&[json!({"matches":"u"})], json!({"u":universal()}));
    let g = &grammar_matchers(&p)[0];
    let AstMatcher::Util(u) = &g.matcher else {
        panic!("utility")
    };
    let mut lower = super::lower::Lower::new(g.language, false);
    assert!(lower.mixed_recursive(u).unwrap().is_none());
    assert!(lower.pure.program.terminals.is_empty());
    assert!(lower.slots.is_empty());
}
#[test]
fn mixed_nested_wide_siblings_keep_control_size_fixed_and_vm_growth_bounded() {
    let rules = [json!({"kind":"array","matches":"u"})];
    let p = policy_utils(&rules, json!({"u":universal()}));
    let mut samples = vec![];
    let mut evidence = vec![];
    for size in [8, 16, 32] {
        let source = format!("f([{}]);", vec!["[1,1,1]"; size].join(","));
        let r = check_work(&p, &rules, &[&source], None, 100000);
        assert!(r["sql"].as_str().unwrap().len() < 256000);
        evidence.push(json!({"width":size,"sourceBytes":source.len(),"sqlBytes":r["sql"].as_str().unwrap().len(),"metrics":r["metrics"],"nativeTraces":r["nativeTraces"],"sqlCalls":r["sqlCalls"],"output":r["output"],"relations":r["relations"]}));
        samples.push((
            r["sql"].as_str().unwrap().len(),
            r["metrics"]["execution"]["vmSteps"].as_u64().unwrap(),
        ));
    }
    for pair in samples.windows(2) {
        assert_eq!(pair[0].0, pair[1].0);
        assert!(pair[1].1 <= 3 * pair[0].1);
    }
    if let Ok(path) = std::env::var("RERAIL_MIXED_RECURSIVE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}
