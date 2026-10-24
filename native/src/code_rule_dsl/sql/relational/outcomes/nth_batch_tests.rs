//! A batch is not a batch of cache misses: every nth transaction completes before the next row.
use super::super::{Slots, lower::Lower, publication_tests::policy_utils};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    matcher::{self, node_at, sql_bound_leaf::leaf_env_budget},
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use ast_grep_core::{AstGrep, meta_var::MetaVarEnv};
use serde_json::json;

#[test]
fn batched_sibling_queries_observe_prior_completed_fills_and_ignore_failed_prefixes() {
    for kind in ["identifier", "string"] {
        let rule = json!({"nthChild":{"position":1,"ofRule":{"kind":kind}}});
        let p = policy_utils(std::slice::from_ref(&rule), json!({}));
        let grammar = &grammar_matchers(&p)[0];
        let doc = tape(grammar.language, "f(a,b,c);");
        let tree = AstGrep::doc(doc.clone());
        let ids: Vec<_> = (0..doc.transducer_len() as u32)
            .filter(|id| {
                let node = node_at(&tree, *id);
                node.kind() == "identifier"
                    && node
                        .parent()
                        .is_some_and(|parent| parent.kind() == "arguments")
            })
            .collect();
        assert_eq!(ids.len(), 3);
        for allowances in [
            vec![3, 3, 3],
            vec![0, 3, 3],
            vec![2, 2, 3, 3],
            vec![3, 0, 0],
        ] {
            let _scopes = matcher::FileScopeSlot::install(Default::default(), None, false);
            let expected = matcher::with_file_memo(&tree, || {
                allowances
                    .iter()
                    .enumerate()
                    .map(|(index, work)| {
                        let (result, left) = leaf_env_budget(
                            &grammar.matcher,
                            node_at(&tree, ids[index % ids.len()]),
                            &MetaVarEnv::new(),
                            *work,
                        );
                        let truth = match result {
                            Ok((Some(true), _)) => 1,
                            Ok((Some(false), _)) => 0,
                            Ok((None, _)) => 2,
                            Err(limit) => {
                                super::super::super::failure::encode(&format!("{limit:?}")).unwrap()
                            }
                        };
                        ((index + 1) as i64, truth, left as i64)
                    })
                    .collect::<Vec<_>>()
            });
            let mut lower = Lower::new(grammar.language, grammar.three_valued);
            let plan = lower.plan(&grammar.matcher).unwrap();
            exec::with_terminals(
                vec![(doc.clone(), Default::default())],
                lower.pure.program.terminals,
                |db, batch, _| {
                    super::super::super::leaves::register(db, batch).unwrap();
                    let mut sql = Sql {
                        definitions: vec![],
                        profile: vec![],
                        correlated: false,
                        generation: Generation::default(),
                    };
                    let values = allowances
                        .iter()
                        .enumerate()
                        .map(|(index, work)| {
                            format!("({},0,{},{work})", index + 1, ids[index % ids.len()])
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    let input=Demand{name:sql.relation("batched sibling demands",format!("SELECT column1 AS k,column2 AS file,column3 AS node,column4 AS work FROM (VALUES {values})"),true,"several sibling invocations sharing one parent"),slots:Slots::new()};
                    let output = sql.emit(&plan, &input).unwrap();
                    let query = format!(
                        "WITH RECURSIVE {} SELECT k,truth,work FROM {} ORDER BY k",
                        sql.definitions.join(",\n"),
                        output.name
                    );
                    let trace = super::super::super::tests::ExecutionTrace::start(db);
                    let actual = db
                        .prepare(&query)
                        .unwrap()
                        .query_map([], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    assert_eq!(trace.count(), 1);
                    drop(trace);
                    assert_eq!(actual, expected, "kind={kind}, allowances={allowances:?}");
                    assert_eq!(sql.generation.return_peak.get(), 2);
                    assert!(sql.profile.iter().any(
                        |item| item["childPrograms"]["siblingIndex"]["singleReachedDemand"] == true
                    ));
                },
            );
        }
    }
}

#[test]
fn sibling_transaction_accounts_for_two_active_return_cells() {
    let rule = json!({"nthChild":{"position":1,"ofRule":{"kind":"identifier"}}});
    let p = policy_utils(std::slice::from_ref(&rule), json!({}));
    let grammar = &grammar_matchers(&p)[0];
    let mut lower = Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let mut sql = Sql {
        definitions: vec![],
        profile: vec![],
        correlated: true,
        generation: Generation {
            return_cells: 127,
            ..Generation::default()
        },
    };
    let input = Demand {
        name: "demand".into(),
        slots: Slots::new(),
    };
    let error = sql.emit(&plan, &input).err().unwrap();
    assert!(error.contains("sibling index return payload"), "{error}");
}
