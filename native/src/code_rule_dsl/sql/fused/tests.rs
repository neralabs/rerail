use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{
        self,
        exec::FileContext,
        tests::{differential::tape, hard_cases::compile, same, snapshot},
    },
    test_support::grammar_matchers,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn the_engine_fuses_many_rules_and_files_and_reuses_new_snapshots() {
    let policy = compile("typescript", json!({"rule":{"pattern":"f($X)"}}));
    let grammar = &grammar_matchers(&policy)[0];
    let program = sql::exec::compile(&vec![grammar; 140]).unwrap();
    let mut session = program.session().unwrap();
    let runs = (0..140)
        .map(|statement| sql::ClauseRun {
            statement,
            files: vec![(0, None), (1, None)],
        })
        .collect::<Vec<_>>();
    for (source, expected) in [("f(a); f(b);", 2), ("g(c);", 0), ("f(d);", 1)] {
        let before = session.stats().statements;
        let result = session
            .run(
                vec![
                    (
                        tape(GrammarDialect::TypeScript, source),
                        FileContext::default(),
                    ),
                    (
                        tape(GrammarDialect::TypeScript, "f(other);"),
                        FileContext::default(),
                    ),
                ],
                &runs,
            )
            .unwrap();
        assert_eq!(
            session.stats().statements - before,
            1,
            "one prepared execution across all rules and files"
        );
        assert!(
            result
                .iter()
                .all(|files| files[0].len() == expected && files[1].len() == 1)
        );
        assert!(
            result
                .iter()
                .flat_map(|files| files.iter().flatten())
                .all(|m| m.captures.len() == 1)
        );
    }
}

#[test]
fn the_engine_preserves_empty_selected_and_inactive_rule_inputs() {
    let policy = compile("typescript", json!({"rule":{"pattern":"f($X)"}}));
    let grammar = &grammar_matchers(&policy)[0];
    let program = sql::exec::compile(&[grammar, grammar, grammar]).unwrap();
    let doc = tape(GrammarDialect::TypeScript, "f(a); f(b);");
    let mut session = program.session().unwrap();
    let all = session
        .run(
            vec![(doc.clone(), FileContext::default())],
            &[sql::ClauseRun {
                statement: 0,
                files: vec![(0, None)],
            }],
        )
        .unwrap();
    let chosen = all[0][0][1].root;
    let selected = session
        .run(
            vec![(doc, FileContext::default())],
            &[
                sql::ClauseRun {
                    statement: 0,
                    files: vec![(0, Some(Arc::from([chosen, chosen])))],
                },
                sql::ClauseRun {
                    statement: 1,
                    files: vec![(0, Some(Arc::from([])))],
                },
                sql::ClauseRun {
                    statement: 2,
                    files: vec![],
                },
            ],
        )
        .unwrap();
    assert_eq!(selected[0][0], vec![all[0][0][1].clone()]);
    assert!(selected[1][0].is_empty());
    assert!(selected[2].is_empty());
}

#[test]
fn witness_segments_preserve_reached_dependency_depth_and_early_success() {
    let policy = compile(
        "typescript",
        json!({"rule":{"kind":"function_declaration","matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^self$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let program = sql::exec::compile(&[grammar]).unwrap();
    let mut session = program.session().unwrap();
    let runs = [sql::ClauseRun {
        statement: 0,
        files: vec![(0, None)],
    }];
    let deep = format!("{}missing{};", "(".repeat(270), ")".repeat(270));
    let late = format!("function f(){{{deep}self;}}");
    let error = session
        .run(
            vec![(
                tape(GrammarDialect::TypeScript, &late),
                FileContext::default(),
            )],
            &runs,
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("recursion deeper than 256 levels"),
        "{error}"
    );
    let early = format!("function f(){{self;{deep}}}");
    let before = session.stats().statements;
    let found = session
        .run(
            vec![(
                tape(GrammarDialect::TypeScript, &early),
                FileContext::default(),
            )],
            &runs,
        )
        .unwrap();
    assert_eq!(session.stats().statements - before, 1);
    assert_eq!(found[0][0].len(), 1);
    let output = same(&policy, &snapshot(&[("typescript", "a.ts", &early)]));
    assert_eq!(output["failures"], json!([]));
}

#[test]
fn immutable_run_memberships_preserve_affinity_and_replace_snapshots() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let inputs = Arc::new(crate::code_rule_dsl::sql::inputs::RootInputs::default());
    inputs.register(&db).unwrap();
    inputs.install_runs(vec![(1, 2), (1, 2), (-1, 3)]);
    let mut query = db
        .prepare("SELECT file FROM runs WHERE run=? ORDER BY file")
        .unwrap();
    for value in [
        rusqlite::types::Value::Integer(1),
        rusqlite::types::Value::Text("1".into()),
    ] {
        let rows = query
            .query_map([value], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows, vec![2]);
    }
    inputs.install_runs(vec![(1, 4)]);
    assert_eq!(query.query_row([1], |r| r.get::<_, i64>(0)).unwrap(), 4);
    inputs.clear();
    assert_eq!(
        db.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn plan_audit_distinguishes_closed_materialization_from_correlated_data_probes() {
    use super::differential::inner_scans;
    let plan = vec![
        (1, 0, "CORRELATED SCALAR SUBQUERY 1".into()),
        (2, 1, "MATERIALIZE q4".into()),
        (3, 2, "SCAN f".into()),
    ];
    assert!(inner_scans(&plan).is_empty());
    let mut repeated = plan.clone();
    repeated.extend([
        (4, 2, "CORRELATED SCALAR SUBQUERY 2".into()),
        (5, 4, "SCAN n VIRTUAL TABLE INDEX 0:".into()),
    ]);
    assert_eq!(
        inner_scans(&repeated),
        vec!["SCAN n VIRTUAL TABLE INDEX 0:"]
    );
    let mut joined = plan;
    joined.push((4, 2, "SCAN n VIRTUAL TABLE INDEX 0:".into()));
    assert_eq!(inner_scans(&joined), vec!["SCAN n VIRTUAL TABLE INDEX 0:"]);
}

#[test]
fn a_thousand_required_effects_fit_sqlite_and_execute_once_each() {
    use crate::code_rule_dsl::sql::fused::graph::{Boundary, Column, Graph, Policy};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.create_scalar_function(
        "observe",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            observed.fetch_add(1, Ordering::Relaxed);
            ctx.get::<i64>(0)
        },
    )
    .unwrap();
    let mut graph = Graph::default();
    for i in 0..1100 {
        graph.add(
            vec![Column {
                name: "v".into(),
                integer: true,
            }],
            format!("SELECT observe({i})"),
            Boundary::Terminal,
        );
    }
    let sql = graph.render(&[], Policy::All);
    let result: String = db.query_row(&sql, [], |r| r.get(3)).unwrap();
    assert_eq!(result, "1100");
    assert_eq!(calls.load(Ordering::Relaxed), 1100);
}

#[test]
fn witness_depth_counts_the_initial_utility_call_at_the_native_boundary() {
    let policy = compile(
        "python",
        json!({"rule":{"kind":"function_definition","matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^self$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let program = sql::exec::compile(&[grammar]).unwrap();
    let mut session = program.session().unwrap();
    for (depth, allowed) in [(251, true), (252, false), (250, true)] {
        let source = format!(
            "def f():\n    x = {}self{}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let input = snapshot(&[("python", "a.py", &source)]);
        let native: serde_json::Value = serde_json::from_str(
            &policy
                .evaluate(input.to_string(), "report".into(), Some("native".into()))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(native["failures"].as_array().unwrap().is_empty(), allowed);
        let result = session.run(
            vec![(
                tape(GrammarDialect::Python, &source),
                FileContext::default(),
            )],
            &[sql::ClauseRun {
                statement: 0,
                files: vec![(0, None)],
            }],
        );
        assert_eq!(result.is_ok(), allowed, "depth={depth}: {result:?}");
        if allowed {
            assert_eq!(result.unwrap()[0][0].len(), 1);
        }
    }
}

#[test]
fn rejected_components_cannot_leak_effects_or_poison_later_interning() {
    use crate::code_rule_dsl::sql::{
        emit::Emitted,
        fused::Builder,
        plan::{Op, Statement},
    };
    let statement = Statement {
        slots: vec![],
        kinds: None,
        op: Op::Never,
        units: vec![],
        sources: vec![],
    };
    let accepted = Emitted {
        statements: vec![
            "CREATE TEMP TABLE /*T*/a(k INTEGER PRIMARY KEY,file,node)".into(),
            "INSERT INTO /*T*/a SELECT 1,0,0".into(),
            "SELECT file,node FROM /*T*/a".into(),
        ],
        effects: vec![],
        root: None,
    };
    let rejected = Emitted {
        statements: vec![
            accepted.statements[0].clone(),
            accepted.statements[1].clone(),
            "INSERT INTO /*T*/a SELECT 2,0,1 FROM fact abandoned_fact".into(),
            "/*LOOP*/".into(),
        ],
        effects: vec![],
        root: None,
    };
    let mut builder = Builder::default();
    builder.clause(0, &statement, &accepted, &[], &[], false);
    builder.clause(1, &statement, &rejected, &[], &[], false);
    builder.clause(2, &statement, &accepted, &[], &[], false);
    let program = builder.finish(&[], 64).unwrap();
    assert_eq!(program.clauses, vec![0, 2]);
    assert!(
        program
            .plan
            .queries
            .iter()
            .all(|s| !s.contains("abandoned_fact"))
    );
    sql::exec::with_terminals(
        vec![(tape(GrammarDialect::TypeScript, ""), FileContext::default())],
        vec![],
        |db, _, frames| {
            program.register(db, frames).unwrap();
            let mut query = db.prepare(&program.plan.queries[0]).unwrap();
            let tags = query
                .query_map([], |r| r.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(tags, vec![-1, 0, 2]);
        },
    );
}
