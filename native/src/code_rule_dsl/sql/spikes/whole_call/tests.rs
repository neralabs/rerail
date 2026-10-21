use super::{
    adapter::Compiler,
    graph::{Column, Policy},
    run,
};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::sql::tests::{differential::tape, hard_cases::compile, same, snapshot};
use crate::code_rule_dsl::{
    sql::{
        self, emit,
        exec::{Captured, FileContext, SqlMatch},
        lower::ProgramBuilder,
        plan::{SlotKind, Statement},
    },
    test_support::grammar_matchers,
};
use serde_json::{Value, json};

pub(super) fn columns(statement: &Statement) -> String {
    let mut names = vec!["file".into(), "node".into()];
    names.extend(
        statement
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.scope == 0)
            .flat_map(|(id, s)| s.columns(id)),
    );
    names.join(",")
}
pub(super) fn decode(statement: &Statement, root: u32, values: &Value) -> SqlMatch {
    let values = values.as_array().expect("capture payload");
    let mut at = 0;
    let mut captures = Vec::new();
    for (id, slot) in statement
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.scope == 0)
    {
        let int = |i: usize| values.get(at + i).and_then(Value::as_i64);
        let value = match slot.kind {
            SlotKind::Node => int(0).map(|n| Captured::Node(n as u32)),
            SlotKind::Count => int(0).map(Captured::Count),
            SlotKind::List => match (int(0), int(1)) {
                (Some(-1), _) => Some(Captured::List(None)),
                (Some(a), Some(b)) => Some(Captured::List(Some((a as u32, b as u32)))),
                _ => None,
            },
            SlotKind::Text => values
                .get(at)
                .and_then(Value::as_str)
                .zip(values.get(at + 1).and_then(Value::as_str))
                .and_then(|(text, range)| {
                    let (a, b) = range.split_once(':')?;
                    Some(Captured::Text(
                        text.into(),
                        a.parse().ok()?..b.parse().ok()?,
                    ))
                }),
        };
        if let Some(value) = value {
            captures.push((slot.name.clone(), value));
        }
        at += slot.columns(id).len();
    }
    SqlMatch { root, captures }
}
fn check(rules: Vec<Value>, sources: &[&str]) -> Value {
    let compiled: Vec<_> = rules
        .into_iter()
        .map(|rule| compile("typescript", json!({"rule":rule})))
        .collect();
    for c in &compiled {
        let files: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| ("typescript", format!("f{i}.ts"), *s))
            .collect();
        let refs: Vec<_> = files.iter().map(|(l, p, s)| (*l, p.as_str(), *s)).collect();
        same(c, &snapshot(&refs));
    }
    let grammars: Vec<_> = compiled.iter().map(|c| &grammar_matchers(c)[0]).collect();
    let docs: Vec<_> = sources
        .iter()
        .map(|s| tape(GrammarDialect::TypeScript, s))
        .collect();
    let runs: Vec<_> = (0..grammars.len())
        .map(|statement| sql::ClauseRun {
            statement,
            files: (0..docs.len()).map(|i| (i, None)).collect(),
        })
        .collect();
    let legacy = sql::exec::compile(&grammars).unwrap();
    let mut session = legacy.session().unwrap();
    let expected = session
        .run(
            docs.iter()
                .cloned()
                .map(|d| (d, FileContext::default()))
                .collect(),
            &runs,
        )
        .unwrap();
    let mut builder = ProgramBuilder::spike(super::amplification::guards::apply);
    let statements: Vec<_> = grammars
        .iter()
        .map(|g| builder.clause_each(g, None).unwrap())
        .collect();
    let files = format!(
        "SELECT CAST(key AS INTEGER) FROM json_each('{}')",
        json!(vec![0; sources.len()])
    );
    let mut compiler = Compiler::default();
    let mut outputs = Vec::new();
    for (i, s) in statements.iter().enumerate() {
        let id = compiler
            .emitted(&emit::clause(s), &columns(s), &builder.stages, &files)
            .unwrap();
        outputs.push((i, id));
    }
    let graph = compiler.graph;
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        builder.terminals,
        |connection, _, frames| {
            let mut chosen = None;
            for policy in [Policy::All, Policy::Reuse, Policy::Boundaries] {
                let sql = graph.render(&outputs, policy);
                let got = run::execute(connection, frames, &graph, &sql)
                    .unwrap_or_else(|e| panic!("{e}\n{sql}"));
                let mut actual = vec![vec![Vec::new(); sources.len()]; grammars.len()];
                for (clause, file, root, values) in got.hits {
                    actual[clause][file].push(decode(&statements[clause], root, &values));
                }
                assert_eq!(
                    actual, expected,
                    "{policy:?}: complete matching roots and captures"
                );
                eprintln!("whole-call materialization {policy:?} {}", got.metrics);
                if policy == Policy::Reuse {
                    chosen = Some(got.metrics);
                }
            }
            for policy in [Policy::All, Policy::Reuse, Policy::Boundaries] {
                let (physical, query) = super::buffers::render(&graph, &outputs, policy);
                let got = run::execute(connection, frames, &physical, &query)
                    .unwrap_or_else(|e| panic!("buffer {policy:?}: {e}\n{query}"));
                let mut actual = vec![vec![Vec::new(); sources.len()]; grammars.len()];
                for (clause, file, root, values) in got.hits {
                    actual[clause][file].push(decode(&statements[clause], root, &values));
                }
                assert_eq!(
                    actual, expected,
                    "buffer {policy:?}: complete roots and captures"
                );
                eprintln!("whole-call buffers {policy:?} {}", got.metrics);
            }
            for narrow in [false, true] {
                for phases in [1, 4] {
                    let mut candidate = graph.clone();
                    if narrow {
                        super::amplification::constants::apply(&mut candidate, &outputs);
                    }
                    let plan = super::physical::plan(&candidate, &outputs, 16, phases);
                    let got =
                        run::execute_program(connection, frames, &plan.graph, &plan.queries, true)
                            .unwrap_or_else(|e| {
                                panic!("shaped {phases}: {e}\n{}", plan.queries.join(";\n"))
                            });
                    let mut actual = vec![vec![Vec::new(); sources.len()]; grammars.len()];
                    for (clause, file, root, values) in got.hits {
                        actual[clause][file].push(decode(&statements[clause], root, &values));
                    }
                    assert_eq!(
                        actual, expected,
                        "shaped {phases}: complete roots and captures"
                    );
                    assert!(plan.queries.len() <= 4);
                    assert_eq!(got.metrics["sqlExecutions"], plan.queries.len());
                    assert!(
                        got.metrics["explain"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|p| p["plan"].is_array())
                    );
                }
            }
            chosen.unwrap()
        },
    )
}
#[test]
fn whole_call_runs_all_files_and_rules_once_with_shared_relations() {
    let rule = json!({"kind":"call_expression","regex":"^f","has":{"kind":"arguments","has":{"pattern":"$X","kind":"identifier"}}});
    let metrics = check(
        vec![
            rule.clone(),
            rule,
            json!({"kind":"identifier","not":{"regex":"^missing$"}}),
        ],
        &["f(a,b); g(c);", "f(d); f(e);", ""],
    );
    assert!(metrics["sharedRelations"].as_u64().unwrap() > 0);
    eprintln!("whole-call shared {metrics}");
}
#[test]
fn whole_call_preserves_first_captures_counts_nullable_slots_and_unknowns() {
    let rules = vec![
        json!({"pattern":"$F($$$A)","has":{"stopBy":"end","kind":"identifier","pattern":"$X"}}),
        json!({"kind":"call_expression","any":[{"pattern":"f($X)"},{"pattern":"g($$$)"}],"has":{"stopBy":"end","kind":"identifier","pattern":"$X"}}),
        json!({"kind":"call_expression","count":{"has":{"stopBy":"end","kind":"identifier"},"min":1,"as":"N"}}),
        json!({"kind":"identifier","not":{"all":[{"testUnknown":"^a$"},{"regex":"^a$"}]}}),
        json!({"kind":"expression_statement","follows":{"stopBy":"end","pattern":"$X"}}),
    ];
    eprintln!(
        "whole-call captures {}",
        check(rules, &["f(a); g(b); h(c,d); a; z;", "g(a); f(b);", ""])
    );
}
#[test]
fn whole_call_batched_environment_pattern_reads_a_typed_demand_handle() {
    let metrics = check(
        vec![
            json!({"pattern":"$F($$$)","has":{"kind":"arguments","has":{"stopBy":"end","kind":"identifier","pattern":"$F"}}}),
        ],
        &["f(f,g); g(f,g); h(a);", "f(f); g(g);"],
    );
    assert!(metrics["publishedSets"].as_u64().unwrap() > 0);
    eprintln!("whole-call environment {metrics}");
}
#[test]
fn whole_call_handles_stop_sets_and_nearest_without_frame_inserts() {
    let rules = vec![
        json!({"kind":"identifier","inside":{"kind":"function_declaration","stopBy":{"kind":"statement_block"}}}),
        json!({"kind":"identifier","inside":{"kind":"function_declaration","stopBy":"end"}}),
        json!({"kind":"identifier","inside":{"kind":"statement_block","stopBy":{"kind":"function_declaration"}}}),
    ];
    eprintln!(
        "whole-call stops {}",
        check(
            rules,
            &[
                "function f(a){ if(a){ return a; } }",
                "function g(b){ return b; }"
            ]
        )
    );
}
#[test]
fn lexical_substitution_preserves_literals_and_identifier_boundaries() {
    let mapping = std::collections::HashMap::from([("f1".into(), "q7".into())]);
    assert_eq!(
        super::lex::replace(
            "SELECT f1.k FROM f1 WHERE x='f1' AND y='a''f1' AND f10.k=1",
            &mapping
        ),
        "SELECT q7.k FROM q7 WHERE x='f1' AND y='a''f1' AND f10.k=1"
    );
    let c = Column {
        name: "k".into(),
        integer: true,
    };
    let mut g = super::graph::Graph::default();
    let a = g.add(
        vec![c.clone()],
        "SELECT 1".into(),
        super::graph::Boundary::None,
    );
    assert_eq!(
        a,
        g.add(vec![c], "SELECT 1".into(), super::graph::Boundary::None)
    );
}

#[test]
fn a_terminal_failure_cannot_disappear_behind_a_later_false_filter() {
    use crate::code_rule_dsl::sql::{
        plan::Op,
        terminals::{FactTerm, Terminal, Verdicts},
    };
    struct Fails;
    impl Verdicts for Fails {
        fn rows<'t>(
            &self,
            _: ast_grep_core::Node<'t, crate::code_rule_dsl::tape::TapeDoc>,
            _: &ast_grep_core::meta_var::MetaVarEnv<'t, crate::code_rule_dsl::tape::TapeDoc>,
        ) -> Result<Vec<crate::code_rule_dsl::sql::terminals::Verdict>, String> {
            Err("injected terminal failure".into())
        }
    }
    let mut statement = Statement {
        slots: Vec::new(),
        kinds: None,
        op: Op::All(vec![
            Op::Fact {
                term: 0,
                reads: Vec::new(),
                at_node: true,
                binds: Vec::new(),
            },
            Op::Never,
        ]),
        units: Vec::new(),
        sources: Vec::new(),
    };
    super::amplification::guards::apply(&mut statement);
    let mut compiler = Compiler::default();
    let id = compiler
        .emitted(&emit::clause(&statement), "file,node", &[], "SELECT 0")
        .unwrap();
    let sql = compiler.graph.render(&[(0, id)], Policy::Reuse);
    sql::exec::with_terminals(
        vec![(
            tape(GrammarDialect::TypeScript, "a;"),
            FileContext::default(),
        )],
        vec![Terminal::Fact(FactTerm {
            verdicts: Box::new(Fails),
            reads: Vec::new(),
        })],
        |connection, _, frames| {
            let error = run::execute(connection, frames, &compiler.graph, &sql)
                .unwrap_err()
                .to_string();
            assert!(error.contains("injected terminal failure"), "{error}");
            for phases in [1, 4] {
                let plan = super::physical::plan(&compiler.graph, &[(0, id)], 1, phases);
                let mut observation = Value::Null;
                let error = run::execute_observed(
                    connection,
                    frames,
                    &plan.graph,
                    &plan.queries,
                    true,
                    Some(&mut observation),
                )
                .unwrap_err()
                .to_string();
                assert!(
                    error.contains("injected terminal failure"),
                    "shaped {phases}: {error}"
                );
                assert!(observation["vmSteps"].as_u64().unwrap() > 0);
                assert!(
                    !observation["explain"][0]["plan"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    observation["profile"]["stagesByCpu"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s["incomplete"] == true)
                );
            }
        },
    );
}

#[test]
fn direct_symbolic_compiler_shares_across_all_rules_without_allocating_frames() {
    let rules = [
        json!({"kind":"identifier","regex":"^[ab]$"}),
        json!({"kind":"identifier","not":{"regex":"^missing$"}}),
        json!({"kind":"call_expression","has":{"stopBy":"end","kind":"identifier","regex":"^a$"}}),
    ];
    let compiled: Vec<_> = rules
        .into_iter()
        .map(|r| compile("typescript", json!({"rule":r})))
        .collect();
    let grammars: Vec<_> = compiled.iter().map(|c| &grammar_matchers(c)[0]).collect();
    let docs = vec![
        tape(GrammarDialect::TypeScript, "f(a,b); g(c);"),
        tape(GrammarDialect::TypeScript, "h(a); z;"),
    ];
    let runs: Vec<_> = (0..grammars.len())
        .map(|statement| sql::ClauseRun {
            statement,
            files: vec![(0, None), (1, None)],
        })
        .collect();
    let expected = sql::exec::compile(&grammars)
        .unwrap()
        .session()
        .unwrap()
        .run(
            docs.iter()
                .cloned()
                .map(|d| (d, FileContext::default()))
                .collect(),
            &runs,
        )
        .unwrap();
    let mut builder = ProgramBuilder::default();
    let mut compiler = super::direct::Direct::default();
    let mut outputs = Vec::new();
    for (i, g) in grammars.iter().enumerate() {
        let shape = builder.clause_each(g, None).unwrap();
        let id = compiler.clause(&shape).unwrap();
        outputs.push((i, id));
    }
    // Add six hundred policy consumers. Their computations retain the same symbolic references.
    let originals = outputs.clone();
    for i in 0..600 {
        outputs.push((i + 3, originals[i % 3].1));
    }
    assert!(compiler.graph.handles.is_empty());
    let query = compiler.graph.render(&outputs, Policy::Boundaries);
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        builder.terminals,
        |connection, _, frames| {
            let got = run::execute(connection, frames, &compiler.graph, &query).unwrap();
            let mut actual = vec![vec![Vec::new(); 2]; 603];
            for (clause, file, root, _) in got.hits {
                actual[clause][file].push(SqlMatch {
                    root,
                    captures: Vec::new(),
                });
            }
            for (i, found) in actual.iter().enumerate() {
                assert_eq!(found, &expected[if i < 3 { i } else { (i - 3) % 3 }]);
            }
            assert_eq!(frames.len(), 0);
            eprintln!("whole-call direct {}", got.metrics);
        },
    );
}

#[test]
fn wide_capture_inputs_publish_only_the_columns_the_terminal_reads() {
    let captures = (0..96)
        .map(|i| format!("$A{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let first = format!("f({captures})");
    let source = format!("f({});", vec!["a"; 96].join(","));
    let metrics = check(
        vec![json!({"all":[{"pattern":first},{"pattern":"f($$$, $A0)"}]})],
        &[&source],
    );
    assert_eq!(metrics["publishedSets"], 1);
    assert_eq!(
        metrics["publishedCells"], 4,
        "key, file, node, and one read capture"
    );
    eprintln!("whole-call wide captures {metrics}");
}

#[test]
fn materialized_cte_diamonds_still_expand_during_sqlite_preparation() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE base(x)").unwrap();
    let mut definitions = vec!["q0 AS MATERIALIZED(SELECT x FROM base)".to_owned()];
    for i in 1..=16 {
        definitions.push(format!(
            "q{i} AS MATERIALIZED(SELECT a.x FROM q{} a JOIN q{} b ON a.x=b.x)",
            i - 1,
            i - 1
        ));
    }
    let query = format!("WITH {} SELECT * FROM q16", definitions.join(","));
    let error = connection
        .prepare(&query)
        .expect_err("CTE materialization does not bound name-resolution expansion")
        .to_string();
    assert!(
        error.contains("too many references") && error.contains("65535"),
        "{error}"
    );
    eprintln!("whole-call CTE preparation barrier: {error}");
}

#[test]
fn physical_materialization_bounds_diamond_expansion_and_releases_dead_buffers() {
    use super::graph::{Boundary, Graph};
    let mut graph = Graph::default();
    let columns = vec![
        Column {
            name: "file".into(),
            integer: true,
        },
        Column {
            name: "node".into(),
            integer: true,
        },
    ];
    let mut previous = graph.add(columns.clone(), "SELECT 0,0".into(), Boundary::None);
    for _ in 0..40 {
        let name = Graph::name(previous);
        previous=graph.add(columns.clone(),format!("SELECT a.file,a.node FROM {name} a JOIN {name} b ON a.file=b.file AND a.node=b.node"),Boundary::None);
    }
    let (physical, query) = super::buffers::render(&graph, &[(0, previous)], Policy::All);
    sql::exec::with_terminals(Vec::new(), Vec::new(), |connection, _, frames| {
        let got = run::execute(connection, frames, &physical, &query).unwrap();
        assert_eq!(got.hits, vec![(0, 0, 0, json!([]))]);
        assert!(
            got.metrics["peakFrameRows"].as_u64().unwrap() <= 3,
            "dead buffer rows must be released: {}",
            got.metrics
        );
    });
}
