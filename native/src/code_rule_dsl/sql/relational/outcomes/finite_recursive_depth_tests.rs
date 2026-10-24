//! Independent two-function entry oracle, including work-zero and rejected depth.
use super::super::{Slots, finite_lower, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
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
    capture: Option<i64>,
    cost: i64,
    peak: u32,
}
fn enter(depth: u32, work: &mut i64, peak: &mut u32) -> Result<(), i64> {
    if depth >= 256 {
        return Err(-1);
    }
    *peak = (*peak).max(depth + 1);
    if *work == 0 {
        return Err(-2);
    }
    *work -= 1;
    Ok(())
}
fn oracle(
    node: Node<'_, TapeDoc>,
    depth: u32,
    work: &mut i64,
    peak: &mut u32,
    memo: &mut BTreeMap<(bool, usize), Answer>,
    enabled: [bool; 2],
    leaf: bool,
) -> (i64, Option<i64>) {
    let key = (leaf, node.node_id());
    if enabled[usize::from(leaf)]
        && let Some(a) = memo.get(&key).copied().filter(|a| depth + a.peak <= 256)
    {
        *peak = (*peak).max(depth + a.peak);
        if *work < a.cost {
            *work = 0;
            return (-2, None);
        }
        *work -= a.cost;
        return (a.truth, a.capture);
    }
    let start = *work;
    let mut reached = 0;
    let answer = (|| {
        if let Err(failure) = enter(depth, work, &mut reached) {
            return (failure, None);
        }
        if leaf {
            return if node.text() == "1" {
                (1, Some(node.node_id() as i64))
            } else {
                (0, None)
            };
        }
        if node.kind() == "number" {
            let answer = oracle(
                node.clone(),
                depth + 1,
                work,
                &mut reached,
                memo,
                enabled,
                true,
            );
            if answer.0 != 0 {
                return answer;
            }
        }
        if node.kind() == "array" {
            for child in node
                .children()
                .filter(|n| matches!(n.kind().as_ref(), "array" | "number"))
            {
                let answer = oracle(child, depth + 1, work, &mut reached, memo, enabled, false);
                if answer.0 != 0 {
                    return answer;
                }
            }
        }
        (0, None)
    })();
    *peak = (*peak).max(reached);
    if enabled[usize::from(leaf)] && answer.0 >= 0 {
        memo.insert(
            key,
            Answer {
                truth: answer.0,
                capture: answer.1,
                cost: start - *work,
                peak: reached - depth,
            },
        );
    }
    answer
}
#[test]
fn finite_recursive_incoming_depth_and_negative_work_unwind_every_handle() {
    let p = policy_utils(
        &[json!({"matches":"u"})],
        json!({"u":{"any":[{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}},{"kind":"number","matches":"leaf"}]},"leaf":{"regex":"^1$","pattern":"$X"}}),
    );
    let grammar = &grammar_matchers(&p)[0];
    for dynamic in [false, true] {
        let mut lower = Lower::new_finite(grammar.language, grammar.three_valued);
        let plan = lower.plan(&grammar.matcher).unwrap();
        let program = finite_lower::lower(&plan).unwrap();
        assert_eq!(program.functions.len(), 2);
        assert!(
            program.functions[0].memo.is_some(),
            "recursive calls must exercise cache fit/miss"
        );
        let enabled = [
            program.functions[0].memo.is_some(),
            program.functions[1].memo.is_some(),
        ];
        let slot = *program.writes.iter().next().unwrap();
        let layout = lower.memo_layout(true);
        exec::with_terminals(
            vec![(tape(grammar.language, "f([0,[0,1]]);"), Default::default())],
            lower.pure.program.terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let root = (0..file.facts.len())
                    .find(|id| node_at(&file.root, *id).kind() == "array")
                    .unwrap();
                db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                    .unwrap();
                let mut memo = BTreeMap::new();
                for incoming in [0, 1, 252, 253, 254, 255, 256, 0] {
                    for allowance in [0, 1, 2, 3, 4, 8, 16, 64] {
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
                                ..Default::default()
                            },
                        };
                        let input = Demand {
                            name: sql.relation(
                                "finite invocation",
                                format!("SELECT 1 k,0 file,{root} node,{allowance} work"),
                                true,
                                "one input",
                            ),
                            slots: Slots::new(),
                        };
                        sql.generation.components.borrow_mut().roots = input.name.clone();
                        let out = sql.finite_program(&program, &input).unwrap();
                        sql.publish_components();
                        let query = format!(
                            "WITH RECURSIVE {} SELECT truth,work,c{slot} FROM {}",
                            sql.definitions.join(","),
                            out.name
                        );
                        let token: i64 = db
                            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                            .unwrap();
                        let read = |r: &rusqlite::Row<'_>| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, Option<i64>>(2)?,
                            ))
                        };
                        let actual = if dynamic {
                            db.query_row(&query, [incoming], read)
                        } else {
                            db.query_row(&query, [], read)
                        }
                        .unwrap();
                        let peak: i64 = db
                            .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                            .unwrap();
                        let mut work = allowance;
                        let mut expected_peak = 0;
                        let (truth, capture) = oracle(
                            node_at(&file.root, root),
                            incoming,
                            &mut work,
                            &mut expected_peak,
                            &mut memo,
                            enabled,
                            false,
                        );
                        assert_eq!(
                            actual,
                            (truth, work, capture),
                            "dynamic{dynamic} incoming{incoming} allowance{allowance}"
                        );
                        assert_eq!(peak, i64::from(expected_peak));
                        let leaks:(i64,i64,i64)=db.query_row("SELECT relation_continuation_rows(),relation_state_rows(),relation_state_scopes()",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                        assert_eq!(leaks, (0, 0, 0));
                    }
                }
            },
        );
    }
}
