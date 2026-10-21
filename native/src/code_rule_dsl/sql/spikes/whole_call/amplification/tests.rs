use super::super::{
    adapter::Compiler,
    graph::Graph,
    physical, recursive_machine, run,
    tests::{columns, decode},
};
use super::{constants, guards, relational};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{
        self, emit,
        exec::FileContext,
        lower::ProgramBuilder,
        tests::{differential::tape, hard_cases::compile},
    },
    test_support::grammar_matchers,
};
use serde_json::json;

#[test]
fn pure_guards_and_narrow_constants_preserve_bound_unknown_and_first_match_results() {
    let policies:Vec<_>=[
        json!({"rule":{"all":[{"pattern":"f($X)"},{"regex":"^f"},{"has":{"kind":"arguments","has":{"pattern":"$X"}}}]}}),
        json!({"rule":{"kind":"call_expression","has":{"stopBy":"end","all":[{"pattern":"$Y"},{"kind":"identifier"},{"regex":"^[ab]$"}]}}}),
        json!({"rule":{"all":[{"testUnknown":"^u"},{"pattern":"$X"},{"regex":"^[au]"}]}}),
    ].into_iter().map(|p|compile("typescript",p)).collect();
    let grammars: Vec<_> = policies.iter().map(|p| &grammar_matchers(p)[0]).collect();
    let docs: Vec<_> = ["f(a);f(f);u; a; g(b,a);", "f(b);g(a,b);u;"]
        .iter()
        .map(|s| (tape(GrammarDialect::TypeScript, s), FileContext::default()))
        .collect();
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
        .run(docs.clone(), &runs)
        .unwrap();
    let mut builder = ProgramBuilder::spike(guards::apply);
    let statements: Vec<_> = grammars
        .iter()
        .map(|g| builder.clause_each(g, None).unwrap())
        .collect();
    let mut compiler = Compiler::default();
    let mut outputs = Vec::new();
    for (i, s) in statements.iter().enumerate() {
        outputs.push((
            i,
            compiler
                .emitted(
                    &emit::clause(s),
                    &columns(s),
                    &builder.stages,
                    "SELECT file FROM node WHERE id=0",
                )
                .unwrap(),
        ));
    }
    let report = constants::apply(&mut compiler.graph, &outputs);
    eprintln!("constant narrowing {report}");
    sql::exec::with_terminals(docs, builder.terminals, |connection, _, frames| {
        let plan = physical::plan(&compiler.graph, &outputs, 8, 4);
        let got =
            run::execute_program(connection, frames, &plan.graph, &plan.queries, true).unwrap();
        let mut actual = vec![vec![Vec::new(); 2]; grammars.len()];
        for (rule, file, root, captures) in got.hits {
            actual[rule][file].push(decode(&statements[rule], root, &captures));
        }
        assert_eq!(actual, expected);
    });
}

#[test]
fn positive_recursion_compiles_to_narrow_relations_and_rejects_other_recursive_shapes() {
    let policy = compile(
        "typescript",
        json!({"rule":{"matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^[ab]$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let mut builder = ProgramBuilder::spike(guards::apply);
    let statement = builder.clause_each(grammar, None).unwrap();
    let queries: Vec<_> = [
        relational::Shape::Closure,
        relational::Shape::Interval,
        relational::Shape::Probe,
        relational::Shape::Segments,
    ]
    .into_iter()
    .map(|shape| {
        (
            format!("{shape:?}"),
            relational::compile(&statement, &builder.terminals, shape)
                .unwrap_or_else(|e| panic!("{e}: {statement:#?}")),
        )
    })
    .collect();
    let generic = recursive_machine::compile_variant(
        &[&statement],
        &builder.terminals,
        recursive_machine::LeafPlan::Partitioned,
    )
    .unwrap();
    let docs = vec![
        (
            tape(
                GrammarDialect::TypeScript,
                "function f(){g(b,a);a;}function z(){c;}",
            ),
            FileContext::default(),
        ),
        (
            tape(GrammarDialect::TypeScript, "b;a;"),
            FileContext::default(),
        ),
    ];
    sql::exec::with_terminals(docs, builder.terminals, |connection, batch, frames| {
        recursive_machine::register_text(connection, batch).unwrap();
        let expected = run::execute(connection, frames, &Graph::default(), &generic).unwrap();
        let normalize = |rows: Vec<(usize, usize, u32, serde_json::Value)>| {
            rows.into_iter()
                .map(|(r, f, n, v)| {
                    (
                        r,
                        f,
                        n,
                        v["result"].clone(),
                        v["env"].clone(),
                        v["fault"].clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = normalize(expected.hits);
        for (shape, query) in queries {
            let got = run::execute(connection, frames, &Graph::default(), &query).unwrap();
            assert_eq!(normalize(got.hits), expected, "{shape}");
            eprintln!("relational {shape} {}", got.metrics);
        }
    });
    let bad = compile(
        "typescript",
        json!({"rule":{"matches":"u"},"utils":{"u":{"any":[{"kind":"identifier","pattern":"$X"},{"inside":{"matches":"u"}}]}}}),
    );
    let mut builder = ProgramBuilder::default();
    let bad = builder
        .clause_each(&grammar_matchers(&bad)[0], None)
        .unwrap();
    assert!(relational::compile(&bad, &builder.terminals, relational::Shape::Probe).is_err());
}

#[test]
fn direct_capture_and_shared_binary_memos_respect_each_files_dialect() {
    let policy = compile("typescript", json!({"rule":{"pattern":"$X"}}));
    let grammar = &grammar_matchers(&policy)[0];
    let mut builder = ProgramBuilder::default();
    let statement = builder.clause_each(grammar, None).unwrap();
    let docs = vec![
        (
            tape(GrammarDialect::TypeScript, "f(a);"),
            FileContext::default(),
        ),
        (tape(GrammarDialect::Python, "f(a)"), FileContext::default()),
    ];
    let expected = sql::exec::compile(&[grammar])
        .unwrap()
        .session()
        .unwrap()
        .run(
            docs.clone(),
            &[sql::ClauseRun {
                statement: 0,
                files: vec![(0, None), (1, None)],
            }],
        )
        .unwrap();
    assert!(!expected[0][0].is_empty());
    assert!(expected[0][1].is_empty());
    let queries: Vec<_> = [
        recursive_machine::LeafPlan::Baseline,
        recursive_machine::LeafPlan::Capture,
        recursive_machine::LeafPlan::Partitioned,
    ]
    .into_iter()
    .map(|v| {
        recursive_machine::compile_scoped(&[&statement], &builder.terminals, v, 64, true).unwrap()
    })
    .collect();
    sql::exec::with_terminals(docs, builder.terminals, |connection, batch, frames| {
        recursive_machine::register_text(connection, batch).unwrap();
        for query in queries {
            let got = run::execute(connection, frames, &Graph::default(), &query).unwrap();
            let mut actual = vec![vec![Vec::new(); 2]];
            for (_, file, root, value) in got.hits {
                if value["result"] == 1 {
                    actual[0][file].push(sql::exec::SqlMatch {
                        root,
                        captures: vec![(
                            statement.slots[0].name.clone(),
                            sql::exec::Captured::Node(value["env"]["0"].as_u64().unwrap() as u32),
                        )],
                    });
                }
            }
            assert_eq!(actual, expected);
        }
    });
}

#[test]
fn nonnull_constants_remain_nullable_on_the_optional_side_of_a_join() {
    use super::super::graph::{Boundary, Column};
    let cols = |names: &[&str]| {
        names
            .iter()
            .map(|name| Column {
                name: (*name).into(),
                integer: true,
            })
            .collect()
    };
    let mut graph = Graph::default();
    let left = graph.add(
        cols(&["k", "file", "node"]),
        "SELECT 1,0,1".into(),
        Boundary::None,
    );
    let right = graph.add(
        cols(&["k", "file", "node", "s0"]),
        "SELECT 2,0,2,0".into(),
        Boundary::None,
    );
    let output = graph.add(
        cols(&["file", "node", "s0"]),
        format!("SELECT l.file,l.node,r.s0 FROM q{left} l LEFT JOIN q{right} r ON r.k=l.k"),
        Boundary::None,
    );
    constants::apply(&mut graph, &[(0, output)]);
    let plan = physical::plan(&graph, &[(0, output)], 8, 1);
    sql::exec::with_terminals(
        vec![(
            tape(GrammarDialect::TypeScript, "a;"),
            FileContext::default(),
        )],
        Vec::new(),
        |connection, _, frames| {
            let got = run::execute_program(connection, frames, &plan.graph, &plan.queries, false)
                .unwrap();
            assert_eq!(got.hits.len(), 1);
            assert_eq!(got.hits[0].3, json!([null]));
        },
    );
}
