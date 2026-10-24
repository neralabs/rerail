//! Adversarial native parity for constrained candidates and fresh constrained stops.
use super::publication_tests::{check_work, policy};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{AstMatcher, Related, StopBy, node_at},
    model::{ClauseKind, Query},
    sql::tests::differential::tape,
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};
use std::sync::Arc;

fn constrained(ast: Value) -> AstMatcher {
    let p = CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"constraint-walk-child","revision":1,"mode":"steer",
        "clauses":[{"id":"child","forbid":{"ast":ast},"violation":{"code":"CHILD","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    grammar_matchers(&p)[0].matcher.clone()
}

#[test]
fn constraint_walk_preserves_caller_target_aliases_fresh_stops_and_budget_failures() {
    let body = constrained(json!({"language":"typescript",
        "rule":{"pattern":"target($A)"},
        "constraints":{
            "A":{"kind":"array","has":{"kind":"call_expression","pattern":"leaf($B)","stopBy":{"pattern":"barrier($A)"}}},
            "B":{"value":{"gt":0}}
        }
    }));
    let stop = constrained(json!({"language":"typescript",
        "rule":{"pattern":"fence($STOP)"},
        "constraints":{"STOP":{"kind":"array","has":{"kind":"number","value":{"eq":0},"stopBy":"end"}}}
    }));
    let rules = [json!({"kind":"statement_block","pattern":"$CALLER"})];
    let mut p = policy(&rules);
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) =
        &mut Arc::make_mut(&mut p.inner).clauses[0].kind
    else {
        panic!("AST policy required")
    };
    // DSL constraints attach to query roots. Embed their typed plans to exercise the same
    // transaction below a search, while retaining the real statement-block root selector.
    let grammar = &mut matchers[0];
    grammar.matcher = AstMatcher::All(vec![
        grammar.matcher.clone(),
        AstMatcher::Has(Related::new(body), StopBy::Rule(Box::new(stop))),
    ]);
    grammar.three_valued = true;
    grammar.text = None;
    let language = grammar.language;
    let sources = [
        concat!(
            "{ target([barrier(leaf(1)), leaf(2)]); }\n",
            "{ target([barrier(leaf(1))]); target([leaf(3)]); }\n",
            "{ fence([0,target([leaf(9)])]); target([leaf(4)]); }\n",
            "{ target([leaf(u)]); target([leaf(5)]); }\n",
            "{ fence([1,target([leaf(6)])]); }\n",
            "{ target([leaf(0)]); target([leaf(7)]); }\n",
        ),
        "{ target([leaf(8)]); } { target([barrier(leaf(10))]); }",
    ];
    let result = check_work(&p, &rules, &sources, None, i64::MAX);
    let captures = result["relations"]["clauses"][0]["captures"]
        .as_array()
        .unwrap();
    let names = captures
        .iter()
        .map(|capture| capture["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["A", "B", "CALLER"],
        "private stop capture must not escape"
    );
    let output = result["output"].as_array().unwrap();
    assert_eq!(output.len(), 6);
    let trees = sources
        .iter()
        .map(|source| AstGrep::doc(tape(language, source)))
        .collect::<Vec<_>>();
    let selected = output
        .iter()
        .map(|row| {
            assert_eq!(row[4], 0);
            assert_eq!(
                row[2], row[3][2],
                "capture constraints changed the caller root"
            );
            assert_ne!(
                row[3][0], row[3][1],
                "nested capture must remain below its constraint target"
            );
            let tree = &trees[row[1].as_u64().unwrap() as usize];
            node_at(tree, row[3][1].as_u64().unwrap() as u32)
                .text()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(selected, ["2", "3", "4", "6", "7", "8"]);
    assert_eq!(result["relations"]["nativeComposition"], false);
    assert!(
        result["sql"]
            .as_str()
            .unwrap()
            .contains("relation_pattern_typed")
    );
    // The helper checks every capture, reached atomic call, remaining allowance, failure
    // diagnostic and file-local rollback. No grammar composition is delegated to a callback.
    for allowance in [0, 1, 2, 4, 16, 512] {
        check_work(&p, &rules, &sources, None, allowance);
    }
    let failed = check_work(&p, &rules, &sources, Some("2"), i64::MAX);
    let rows = failed["output"].as_array().unwrap();
    assert!(rows.iter().any(|row| row[1] == 0 && row[4] == -4));
    assert!(!rows.iter().any(|row| row[1] == 0 && row[4] == 0));
    assert!(rows.iter().any(|row| row[1] == 1 && row[4] == 0));
}

#[test]
fn constraint_core_and_outer_utility_share_native_answers_with_identical_captures() {
    fn wrap(matcher: &mut AstMatcher, remaining: &mut usize, constraint: &AstMatcher) -> bool {
        match matcher {
            AstMatcher::Util(_) if *remaining == 0 => {
                *matcher =
                    AstMatcher::Constrained(Box::new(crate::code_rule_dsl::matcher::Constrained {
                        rule: matcher.clone(),
                        constraints: vec![("X".into(), constraint.clone())],
                    }));
                true
            }
            AstMatcher::Util(_) => {
                *remaining -= 1;
                false
            }
            AstMatcher::All(items) => items
                .iter_mut()
                .any(|item| wrap(item, remaining, constraint)),
            _ => false,
        }
    }
    let kind_policy = policy(&[json!({"kind":"number"})]);
    let constraint = &grammar_matchers(&kind_policy)[0].matcher;
    let rules = [json!({"all":[
        {"kind":"number"}, {"pattern":"$X"}, {"matches":"v"}, {"matches":"v"}
    ]})];
    for constrained_call in [0, 1] {
        let mut p = super::publication_tests::policy_utils(&rules, json!({"v":{"value":{"gt":0}}}));
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) =
            &mut Arc::make_mut(&mut p.inner).clauses[0].kind
        else {
            panic!("AST policy required")
        };
        let mut remaining = constrained_call;
        assert!(wrap(&mut matchers[0].matcher, &mut remaining, constraint));
        // X is bound before either invocation. The pure constraint checks that same node
        // without changing captures, so native memo keys agree in both invocation orders.
        for allowance in 0..=5 {
            let result = check_work(&p, &rules, &["1;", "2;"], None, allowance);
            if allowance == 3 {
                assert!(
                    result["output"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|row| row[4] == -2)
                );
            }
            if allowance == 4 {
                assert_eq!(
                    result["sqlCalls"].as_array().unwrap().len(),
                    2,
                    "one reached value leaf per file, followed by native-equivalent replay"
                );
                assert_eq!(result["output"].as_array().unwrap().len(), 2);
                assert!(
                    result["output"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|row| row[4] == 0 && row[5] == 0)
                );
            }
        }
    }
}
