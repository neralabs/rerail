//! One-statement enumeration for a proved simple each shape: a first identifier witness in a
//! subtree, with occurrence ranges excluded per root. General exclusion-sensitive bodies remain
//! a separate compiler obligation.
use super::{graph::Graph, run};
use crate::code_rule_dsl::sql::tests::{differential::tape, same, snapshot};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{
        self,
        exec::{Captured, FileContext, SqlMatch},
    },
    test_support::{CAPABILITIES, clause_document, compile_document_with, grammar_matchers},
};
use serde_json::json;

#[test]
fn simple_each_enumerates_occurrences_for_every_root_and_file_in_one_statement() {
    let policy=compile_document_with(clause_document(json!({"id":"each","forbid":{"ast":{"language":"typescript","rule":{"kind":"call_expression","has":{"stopBy":"end","kind":"identifier","pattern":"$X"}}}},"violation":{"anchor":"witness.span","code":"X","message":"x","each":"X"}})),&serde_json::from_str(CAPABILITIES).unwrap()).unwrap();
    let sources = ["f(g(a),a); h(b);", "f(a,a);"];
    same(
        &policy,
        &snapshot(&[
            ("typescript", "a.ts", sources[0]),
            ("typescript", "b.ts", sources[1]),
        ]),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let docs: Vec<_> = sources
        .iter()
        .map(|s| tape(GrammarDialect::TypeScript, s))
        .collect();
    let expected = sql::exec::compile_enumerating(&[grammar], Some("X"))
        .unwrap()
        .session()
        .unwrap()
        .run(
            docs.iter()
                .cloned()
                .map(|d| (d, FileContext::default()))
                .collect(),
            &[sql::ClauseRun {
                statement: 0,
                files: vec![(0, None), (1, None)],
            }],
        )
        .unwrap();
    let query = format!(
        "SELECT 0 AS clause,r.file,r.id AS node,json_array(c.id) AS captures FROM node r CROSS JOIN node c ON c.file=r.file AND c.id>r.id AND c.id<=r.\"last\" AND c.kind={} WHERE r.kind={} ORDER BY r.file,r.id,c.id",
        GrammarDialect::TypeScript.kind_id("identifier"),
        GrammarDialect::TypeScript.kind_id("call_expression")
    );
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        Vec::new(),
        |connection, _, frames| {
            let got = run::execute(connection, frames, &Graph::default(), &query).unwrap();
            let mut actual = vec![vec![Vec::new(); 2]];
            for (_, file, root, value) in got.hits {
                actual[0][file].push(SqlMatch {
                    root,
                    captures: vec![(
                        "X".into(),
                        Captured::Node(value[0].as_u64().unwrap() as u32),
                    )],
                });
            }
            assert_eq!(actual, expected);
            eprintln!("whole-call each {}", got.metrics);
        },
    );
}
