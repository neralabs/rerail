//! The SQL logical utility bound, independent of the native physical-stack guard.
use super::super::{lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Op, Slots, Sql};
use crate::code_rule_dsl::{
    matcher::node_at,
    sql::{exec, tests::differential::tape},
    tape::TapeDoc,
    test_support::grammar_matchers,
};
use ast_grep_core::Node;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Answer {
    truth: i64,
    cost: i64,
    peak: u32,
}
fn oracle(
    node: Node<'_, TapeDoc>,
    level: u32,
    work: &mut i64,
    peak: &mut u32,
    memo: &mut BTreeMap<usize, Answer>,
) -> i64 {
    if let Some(answer) = memo
        .get(&node.node_id())
        .copied()
        .filter(|a| level + a.peak <= 256)
    {
        *peak = (*peak).max(level + answer.peak);
        if *work < answer.cost {
            *work = 0;
            return -2;
        }
        *work -= answer.cost;
        return answer.truth;
    }
    if level >= 256 {
        return -1;
    }
    let start = *work;
    let mut entered = level + 1;
    let truth = (|| {
        if *work == 0 {
            return -2;
        }
        *work -= 1;
        if node.kind() == "identifier" {
            return i64::from(node.text() == "a");
        }
        if node.kind() != "array" {
            return 0;
        }
        for child in node.children() {
            if matches!(child.text().as_ref(), "[" | "]" | ",") {
                continue;
            }
            let truth = oracle(child, level + 1, work, &mut entered, memo);
            if truth <= 0 {
                return truth;
            }
        }
        1
    })();
    *peak = (*peak).max(entered);
    if truth >= 0 {
        memo.insert(
            node.node_id(),
            Answer {
                truth,
                cost: start - *work,
                peak: entered - level,
            },
        );
    }
    truth
}
#[test]
fn captured_logical_depth_memo_fit_and_completed_cost_match_independent_model() {
    let body = json!({"any":[{"kind":"identifier","pattern":"$P","regex":"^a$"},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":r"^[\[\],]$"},{"matches":"u"}]}}}}]});
    let policy = policy_utils(&[json!({"matches":"u"})], json!({"u":body}));
    let grammar = &grammar_matchers(&policy)[0];
    for dynamic in [false, true] {
        let mut lower = Lower::new(grammar.language, false);
        let plan = lower.plan(&grammar.matcher).unwrap();
        assert!(matches!(&plan.op,Op::MixedRecursive{program,..} if program.captures));
        let layout = lower.memo_layout(true);
        exec::with_terminals(
            vec![(
                tape(grammar.language, "f([a,[a,a],a]);"),
                Default::default(),
            )],
            lower.pure.program.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let root = (0..file.facts.len())
                    .find(|id| node_at(&file.root, *id).kind() == "array")
                    .unwrap();
                let mut memo = BTreeMap::new();
                for (incoming, allowance) in [
                    (0, 0),
                    (0, 2),
                    (0, 1000),
                    (254, 1000),
                    (253, 1000),
                    (255, 0),
                    (256, 0),
                    (0, 2),
                    (0, 1000),
                ] {
                    let mut sql = Sql {
                        definitions: vec![],
                        profile: vec![],
                        correlated: false,
                        generation: Generation {
                            peak_tracking: true,
                            memo_clause: Some(0),
                            memo_layout: layout.clone(),
                            utility_depth: if dynamic { 0 } else { incoming },
                            dynamic_depth: dynamic.then(|| "?1".into()),
                            ..Generation::default()
                        },
                    };
                    let input = Demand {
                        name: sql.relation(
                            "captured oracle input",
                            format!("SELECT 1 k,0 file,{root} node,{allowance} work"),
                            true,
                            "single invocation",
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
                    let mut work = allowance;
                    let mut reached = 0;
                    let truth = oracle(
                        node_at(&file.root, root),
                        incoming,
                        &mut work,
                        &mut reached,
                        &mut memo,
                    );
                    assert_eq!(
                        (actual, peak),
                        ((truth, work), i64::from(reached)),
                        "dynamic={dynamic}, incoming={incoming}, work={allowance}"
                    );
                    let live:(i64,i64,i64)=db.query_row("SELECT relation_state_rows(),relation_state_scopes(),relation_continuation_rows()",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                    assert_eq!(live, (0, 0, 0));
                }
            },
        );
    }
}
