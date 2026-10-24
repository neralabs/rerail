use super::super::{
    Plan, effects::Effects, lower::Lower, predicate_memo, publication_tests::policy_utils,
};
use super::{Demand, Generation, Op, Slots, Sql};
use crate::code_rule_dsl::{
    matcher::node_at,
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::json;
#[test]
fn predicate_memo_static_peak_one_fits_255_and_rejects_256_before_work() {
    let body = json!({"kind":"function_declaration","not":{"has":{"stopBy":"end","kind":"return_statement","has":{"stopBy":"neighbor","kind":"number","regex":"^0$"}}}});
    let policy = policy_utils(&[json!({"matches":"deep"})], json!({"deep":body}));
    let grammar = &grammar_matchers(&policy)[0];
    for dynamic in [false, true] {
        let mut lower = Lower::new(grammar.language, false);
        let plan = lower.plan(&grammar.matcher).unwrap();
        let Op::Charge(id, body) = plan.op else {
            panic!("ordinary utility");
        };
        assert!(predicate_memo::eligible(&body));
        let op = Op::MemoPredicate(id, body);
        let plan = Plan {
            effects: Effects::of(&op),
            op,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        };
        exec::with_terminals(
            vec![(
                tape(grammar.language, "function f(){return 1;return 2;}"),
                Default::default(),
            )],
            lower.pure.program.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let node = (0..file.facts.len())
                    .find(|id| node_at(&file.root, *id).kind() == "function_declaration")
                    .unwrap();
                for (incoming, work) in [
                    (0, 0),
                    (0, 1),
                    (255, 0),
                    (256, 0),
                    (255, 1),
                    (256, 5),
                    (0, 5),
                ] {
                    let mut sql = Sql {
                        definitions: vec![],
                        profile: vec![],
                        correlated: false,
                        generation: Generation {
                            memo_clause: Some(0),
                            utility_depth: if dynamic { 0 } else { incoming },
                            dynamic_depth: dynamic.then(|| "?1".into()),
                            ..Generation::default()
                        },
                    };
                    let input = Demand {
                        name: sql.relation(
                            "predicate memo depth input",
                            format!("SELECT 1 k,0 file,{node} node,{work} work"),
                            true,
                            "single helper",
                        ),
                        slots: Slots::new(),
                    };
                    let out = sql.emit(&plan, &input).unwrap();
                    sql.publish_components();
                    let query = format!(
                        "WITH RECURSIVE {} SELECT truth,work FROM {}",
                        sql.definitions.join(","),
                        out.name
                    );
                    let token: i64 = db
                        .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                        .unwrap();
                    let row =
                        |r: &rusqlite::Row<'_>| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?));
                    let actual = if dynamic {
                        db.query_row(&query, [incoming], row)
                    } else {
                        db.query_row(&query, [], row)
                    }
                    .unwrap();
                    let peak: i64 = db
                        .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                        .unwrap();
                    let expected = if incoming >= 256 {
                        ((-1, work), 0)
                    } else if work == 0 {
                        ((-2, 0), i64::from(incoming + 1))
                    } else {
                        ((1, work - 1), i64::from(incoming + 1))
                    };
                    assert_eq!(
                        (actual, peak),
                        expected,
                        "dynamic={dynamic}, incoming={incoming}, work={work}"
                    );
                }
            },
        );
    }
}
