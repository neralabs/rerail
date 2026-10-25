use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::json;

fn policy(
    language: &str,
    rule: serde_json::Value,
    utils: serde_json::Value,
) -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"arbitrary-shared-batch","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(),
        "json".into(), CAPABILITIES.into(),
    ).unwrap()
}

#[test]
fn shared_job_native_rows_zero_work_and_prepared_reexecution() {
    let policies = [
        policy(
            "python",
            json!({"kind":"identifier","regex":"^(alpha|beta)$"}),
            json!({}),
        ),
        policy(
            "python",
            json!({"kind":"integer","regex":"^[135]$"}),
            json!({}),
        ),
    ];
    let grammars = policies
        .iter()
        .map(|p| grammar_matchers(p)[0].clone())
        .collect::<Vec<_>>();
    let language = grammars[0].language;
    let source = "alpha\nbeta\nother\n1\n2\n3\n";
    for files in [1, 16] {
        for rules in [2, 32, 146] {
            // Two clauses must actually share a producer before selecting this
            // shape. A two-rule control therefore repeats the first grammar.
            let inputs = (0..rules)
                .map(|i| &grammars[if rules == 2 { 0 } else { i % 2 }])
                .collect::<Vec<_>>();
            let pack = super::try_compile(&inputs, &PlannerStats::default())
                .unwrap()
                .unwrap();
            assert_eq!(pack.arms.len(), rules);
            assert!(
                pack.arms
                    .iter()
                    .enumerate()
                    .all(|(i, arm)| arm.clause == i && arm.names.is_empty())
            );
            assert!(pack.sql.len() <= 256_000);
            let mut expected = Vec::new();
            for (clause, grammar) in inputs.iter().enumerate() {
                for file in 0..files {
                    let tree = AstGrep::doc(tape(language, source));
                    for node in tree.root().dfs() {
                        let mut work = 0;
                        if grammar
                            .match_root(node.clone(), &mut work)
                            .unwrap()
                            .is_some()
                        {
                            expected.push((clause as i64, file as i64, node.node_id() as i64));
                        }
                        assert_eq!(
                            work, 0,
                            "admitted native source must spend no work even at exhaustion"
                        );
                    }
                }
            }
            expected.sort();
            exec::with_terminals(
                (0..files)
                    .map(|_| (tape(language, source), Default::default()))
                    .collect(),
                pack.terminals,
                |db, _, _| {
                    let mut statement = db.prepare(&pack.sql).unwrap();
                    // Reuse this exact prepared program and terminal table. A
                    // different logical program requires compilation/preparation;
                    // this API does not silently mutate VALUES rule identities.
                    for _ in 0..3 {
                        let trace =
                            crate::code_rule_dsl::sql::relational::tests::ExecutionTrace::start(db);
                        let rows = statement
                            .query_map([], |row| {
                                assert_eq!(row.get::<_, String>(3)?, "[]");
                                assert_eq!(row.get::<_, i64>(4)?, 0);
                                assert_eq!(row.get::<_, i64>(5)?, i64::MAX);
                                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                            })
                            .unwrap()
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .unwrap();
                        assert_eq!(rows, expected);
                        assert_eq!(trace.count(), 1);
                    }
                },
            );
        }
    }
}

#[test]
fn shared_job_admission_excludes_charges_effects_and_unique_producers() {
    let pure = policy("python", json!({"kind":"identifier"}), json!({}));
    let g = &grammar_matchers(&pure)[0];
    for (rule, utils) in [
        (
            json!({"matches":"renamed_helper"}),
            json!({"renamed_helper":{"kind":"identifier"}}),
        ),
        (
            json!({"has":{"kind":"identifier","stopBy":"end"}}),
            json!({}),
        ),
        (json!({"pattern":"$NAME"}), json!({})),
    ] {
        let p = policy("python", rule, utils);
        let other = &grammar_matchers(&p)[0];
        assert!(
            super::try_compile(&[g, other, g], &PlannerStats::default())
                .unwrap()
                .is_none()
        );
    }
    let integer = policy("python", json!({"kind":"integer"}), json!({}));
    assert!(
        super::try_compile(
            &[g, &grammar_matchers(&integer)[0]],
            &PlannerStats::default()
        )
        .unwrap()
        .is_none()
    );
    let js = policy("javascript", json!({"kind":"identifier"}), json!({}));
    assert!(
        super::try_compile(&[g, &grammar_matchers(&js)[0], g], &PlannerStats::default())
            .unwrap()
            .is_none()
    );
    let mut oversized = g.clone();
    oversized.matcher = AstMatcher::All(vec![g.matcher.clone(); 256]);
    assert!(
        super::try_compile(&[g, &oversized, g], &PlannerStats::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn shared_job_false_applies_and_structural_preimages_keep_clause_identity() {
    let p = policy(
        "python",
        json!({"kind":"identifier","inside":{"kind":"call","stopBy":"neighbor"}}),
        json!({}),
    );
    let g = &grammar_matchers(&p)[0];
    let mut disabled = g.clone();
    disabled.applies = false;
    let inputs = [g, &disabled, g];
    let pack = super::try_compile(&inputs, &PlannerStats::default())
        .unwrap()
        .unwrap();
    let tree = AstGrep::doc(tape(g.language, "alpha(beta)\nother\n"));
    let expected = tree
        .root()
        .dfs()
        .filter_map(|node| {
            let mut work = 0;
            g.match_root(node.clone(), &mut work)
                .unwrap()
                .map(|_| node.node_id() as i64)
        })
        .collect::<Vec<_>>();
    assert!(!expected.is_empty());
    exec::with_terminals(
        vec![(tape(g.language, "alpha(beta)\nother\n"), Default::default())],
        pack.terminals,
        |db, _, _| {
            let mut statement = db.prepare(&pack.sql).unwrap();
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let wanted = [0, 2]
                .into_iter()
                .flat_map(|clause| expected.iter().map(move |node| (clause, *node)))
                .collect::<Vec<_>>();
            assert_eq!(rows, wanted);
        },
    );
}

#[test]
fn shared_job_common_hook_selects_only_reused_pure_batches() {
    let p = policy(
        "python",
        json!({"kind":"identifier","regex":"^alpha$"}),
        json!({}),
    );
    let g = &grammar_matchers(&p)[0];
    let packs =
        crate::code_rule_dsl::sql::relational::job::compile(&[g, g], &PlannerStats::default())
            .unwrap();
    assert_eq!(packs.len(), 1);
    assert!(
        packs[0].sql.contains("shared_family_0"),
        "common job hook must select the shared producer rather than independent closed branches"
    );
}
