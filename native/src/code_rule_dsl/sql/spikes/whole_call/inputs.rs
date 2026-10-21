use super::{
    adapter::Compiler,
    graph::Policy,
    run,
    tests::{columns, decode},
};
use crate::code_rule_dsl::sql::tests::{differential::tape, hard_cases::compile};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{self, emit, exec::FileContext, lower::ProgramBuilder},
    test_support::grammar_matchers,
};
use serde_json::json;

#[test]
fn one_compiled_connection_reads_new_file_versions_without_old_answers() {
    let policy = compile("typescript", json!({"rule":{"pattern":"f($X)"}}));
    let grammar = &grammar_matchers(&policy)[0];
    let mut builder = ProgramBuilder::default();
    let statement = builder.clause_each(grammar, None).unwrap();
    let mut compiler = Compiler::default();
    let id = compiler
        .emitted(
            &emit::clause(&statement),
            &columns(&statement),
            &builder.stages,
            "SELECT 0 UNION ALL SELECT 1",
        )
        .unwrap();
    let (physical, query) = super::buffers::render(&compiler.graph, &[(0, id)], Policy::Reuse);
    let requests = [["f(a);", "f(c);"], ["g(b);", "f(c);"], ["", "f(d);"]];
    let batches = requests
        .iter()
        .map(|sources| {
            sources
                .iter()
                .map(|s| (tape(GrammarDialect::TypeScript, s), FileContext::default()))
                .collect()
        })
        .collect();
    let mut call = 0;
    sql::exec::with_call_batches(batches, builder.terminals, |connection, batch, frames| {
        let got = run::execute(connection, frames, &physical, &query).unwrap();
        let actual: Vec<_> = got
            .hits
            .into_iter()
            .map(|(_, file, root, captures)| {
                let found = decode(&statement, root, &captures);
                let env = sql::exec::environment(&batch.files[file].root, &found);
                env.get_match("X").unwrap().text().into_owned()
            })
            .collect();
        assert_eq!(
            actual,
            match call {
                0 => vec!["a", "c"],
                1 => vec!["c"],
                _ => vec!["d"],
            }
        );
        call += 1;
    });
    assert_eq!(call, 3);
}

#[test]
fn all_selected_empty_and_duplicate_root_scopes_share_one_call() {
    use crate::code_rule_dsl::sql::inputs::RootInputs;
    use std::sync::Arc;
    let policy = compile("typescript", json!({"rule":{"pattern":"f($X)"}}));
    let grammar = &grammar_matchers(&policy)[0];
    let docs: Vec<_> = ["f(a); f(b);", "f(c);"]
        .iter()
        .map(|s| tape(GrammarDialect::TypeScript, s))
        .collect();
    let selected: Vec<_> = (0..docs[0].transducer_len() as u32)
        .filter(|&id| {
            docs[0].transducer_kind(id) == GrammarDialect::TypeScript.kind_id("call_expression")
        })
        .collect();
    let runs = vec![
        sql::ClauseRun {
            statement: 0,
            files: vec![(0, None), (1, None)],
        },
        sql::ClauseRun {
            statement: 1,
            files: vec![
                (0, Some(Arc::from(vec![selected[1], selected[1]]))),
                (1, Some(Arc::from([]))),
            ],
        },
        sql::ClauseRun {
            statement: 2,
            files: vec![(0, Some(Arc::from([]))), (1, Some(Arc::from([])))],
        },
    ];
    let legacy = sql::exec::compile(&[grammar, grammar, grammar]).unwrap();
    let expected = legacy
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
    let mut compiler = Compiler::default();
    compiler.stage_files = Some("SELECT 0 UNION ALL SELECT 1".into());
    let mut outputs = Vec::new();
    let mut statements = Vec::new();
    for i in 0..3 {
        let statement = builder.clause_each(grammar, None).unwrap();
        compiler.selected_roots = Some(format!("SELECT file,node FROM roots WHERE run={i}"));
        let files = if i == 0 {
            "SELECT 0 UNION ALL SELECT 1"
        } else {
            "SELECT NULL WHERE 0"
        };
        let id = compiler
            .emitted(
                &emit::clause(&statement),
                &columns(&statement),
                &builder.stages,
                files,
            )
            .unwrap();
        outputs.push((i, id));
        statements.push(statement);
    }
    let (physical, query) = super::buffers::render(&compiler.graph, &outputs, Policy::Reuse);
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        builder.terminals,
        |connection, _, frames| {
            let roots = Arc::new(RootInputs::default());
            roots.register(connection).unwrap();
            roots.install(&runs);
            let got = run::execute(connection, frames, &physical, &query).unwrap();
            let mut actual = vec![vec![Vec::new(); 2]; 3];
            for (clause, file, root, captures) in got.hits {
                actual[clause][file].push(decode(&statements[clause], root, &captures));
            }
            assert_eq!(actual, expected);
            assert_eq!(actual[1][0].len(), 1);
            assert!(actual[2].iter().all(Vec::is_empty));
        },
    );
}
