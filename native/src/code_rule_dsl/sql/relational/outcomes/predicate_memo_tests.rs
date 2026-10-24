use super::{
    Op, Plan, predicate_memo,
    publication_tests::{check_work, policy_utils},
};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::json;
fn helper() -> serde_json::Value {
    json!({"kind":"function_declaration","not":{"has":{"stopBy":"end","kind":"return_statement","has":{"stopBy":"neighbor","kind":"number","regex":"^0$"}}}})
}
fn count(p: &Plan) -> usize {
    match &p.op {
        Op::MemoPredicate(..) => 1,
        Op::All(v) | Op::Any(v) => v.iter().map(count).sum(),
        Op::Charge(_, p) | Op::Shared(p) => count(p),
        Op::Not(p) | Op::FieldNeighbor { child: p, .. } | Op::FieldEnd { child: p, .. } => count(p),
        Op::Search(_, _, p, stop) => count(p) + stop.as_ref().map_or(0, |p| count(p)),
        _ => 0,
    }
}
#[test]
fn predicate_cache_requires_overlapping_demand_and_excludes_typed_callbacks() {
    for (rule, expected) in [
        (json!({"matches":"deep"}), 0),
        (
            json!({"kind":"return_statement","inside":{"stopBy":"end","kind":"function_declaration","matches":"deep"}}),
            1,
        ),
    ] {
        let rules = [rule];
        let policy = policy_utils(&rules, json!({"deep":helper()}));
        let grammar = &grammar_matchers(&policy)[0];
        let mut lower = super::lower::Lower::new(grammar.language, false);
        let plan = predicate_memo::prepare(lower.plan(&grammar.matcher).unwrap());
        assert_eq!(count(&plan), expected);
        assert!(!plan.effects.needs_peak);
    }
    for body in [
        json!({"all":[helper(),{"value":{"gt":0}}]}),
        json!({"all":[helper(),{"pattern":"$P"}]}),
    ] {
        let policy = policy_utils(&[json!({"matches":"deep"})], json!({"deep":body}));
        let grammar = &grammar_matchers(&policy)[0];
        let mut lower = super::lower::Lower::new(grammar.language, true);
        let plan = lower.plan(&grammar.matcher).unwrap();
        let Op::Charge(_, body) = plan.op else {
            panic!("ordinary utility");
        };
        assert!(!predicate_memo::eligible(&body));
    }
}
#[test]
fn predicate_cache_replays_one_step_for_repeated_ancestors_and_preserves_failures() {
    let rules = [
        json!({"kind":"return_statement","inside":{"stopBy":"end","kind":"function_declaration","matches":"deep"}}),
    ];
    let policy = policy_utils(&rules, json!({"deep":helper()}));
    for work in [0, 1, 2, 3, 4, 8, 32, 1000] {
        let report = check_work(
            &policy,
            &rules,
            &[
                "function f(){return 1;return 2;return 3;}",
                "function g(){return 0;return 1;}",
            ],
            None,
            work,
        );
        if work == 1000 {
            assert!(
                report["metrics"]["execution"]["memoStorage"]["hits"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
    }
}
