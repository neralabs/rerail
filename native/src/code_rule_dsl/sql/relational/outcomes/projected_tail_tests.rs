use super::super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::json;
#[test]
fn projected_tail_preserves_native_complete_and_failed_entry_prefixes() {
    let rules = [json!({"matches":"outer"})];
    let p = policy_utils(
        &rules,
        json!({
            "outer":{"any":[{"kind":"identifier","matches":"leaf"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"outer"}}]},
            "leaf":{"any":[{"matches":"first"},{"matches":"second"}]},
            "first":{"kind":"identifier","regex":"^a$"},
            "second":{"kind":"identifier","regex":"^b$"}
        }),
    );
    let g = &grammar_matchers(&p)[0];
    let mut lower = super::super::lower::Lower::new(g.language, g.three_valued);
    let plan = lower.plan(&g.matcher).unwrap();
    assert!(matches!(&plan.op,super::super::Op::PureRecursive(p) if p.projection.is_some()));
    for work in [0, 1, 2, 3, 4, 5, 6, 8, 16, 32, 1000, i64::MAX] {
        check_work(
            &p,
            &rules,
            &["a; ((b)); (((c)));", "f(((a)),((c)));"],
            None,
            work,
        );
    }
}
