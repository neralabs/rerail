//! Compose actual grammar matches and select_when in the same SQLite statement. This reuses
//! the existing decision SQL and its independent native oracle, without loading leaf tables.
use super::{adapter::Compiler, graph::Policy, run, tests::columns};
use crate::code_rule_dsl::sql::tests::{differential::tape, hard_cases::compile};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    model::{Query, RegexSource},
    query::select_when::{self, LeafRow, Leaves, Plan},
    sql::{self, emit, exec::FileContext, lower::ProgramBuilder},
    test_support::grammar_matchers,
};
use serde_json::json;

struct Rows(Vec<Vec<LeafRow>>);
impl Leaves for Rows {
    fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String> {
        Ok(self.0.get(leaf).and_then(|r| r.get(position)).copied())
    }
}
#[test]
fn grammar_matching_and_policy_decisions_share_one_statement() {
    let capture_regex = regex::Regex::new("x").unwrap();
    for source in ["f(a); g(b);", "g(a); g(b);"] {
        let policy0 = compile(
            "typescript",
            json!({"rule":{"kind":"call_expression","regex":"^f"}}),
        );
        let policy1 = compile("typescript", json!({"rule":{"kind":"identifier"}}));
        let grammars = vec![
            &grammar_matchers(&policy0)[0],
            &grammar_matchers(&policy1)[0],
        ];
        let docs = vec![
            tape(GrammarDialect::TypeScript, source),
            tape(GrammarDialect::TypeScript, "g(c); d;"),
        ];
        let runs: Vec<_> = (0..2)
            .map(|statement| sql::ClauseRun {
                statement,
                files: vec![(0, None), (1, None)],
            })
            .collect();
        let legacy = sql::exec::compile(&grammars)
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
        let leaf = || Query::Regex {
            regex: capture_regex.clone(),
            source: RegexSource::Content,
        };
        let query = Query::SelectWhen {
            exists: Box::new(leaf()),
            select: Box::new(leaf()),
        };
        let plan = Plan::new(&query, false, |_, _| ());
        let rows = Rows(
            legacy
                .iter()
                .map(|files| {
                    files
                        .iter()
                        .map(|matches| LeafRow {
                            failed: false,
                            partial: false,
                            witnesses: matches.len(),
                        })
                        .collect()
                })
                .collect(),
        );
        let mut builder = ProgramBuilder::default();
        let mut compiler = Compiler::default();
        let mut outputs = Vec::new();
        for (i, g) in grammars.iter().enumerate() {
            let statement = builder.clause_each(g, None).unwrap();
            outputs.push((
                i,
                compiler
                    .emitted(
                        &emit::clause(&statement),
                        &columns(&statement),
                        &builder.stages,
                        "SELECT 0 UNION ALL SELECT 1",
                    )
                    .unwrap(),
            ));
        }
        let evidence = compiler.graph.render(&outputs, Policy::Reuse);
        sql::exec::with_terminals(
            docs.into_iter()
                .map(|d| (d, FileContext::default()))
                .collect(),
            builder.terminals,
            |connection, _, frames| {
                for cap in [0, 1, 3, 100] {
                    let native = select_when::answer(&plan, cap, &rows).unwrap().unwrap();
                    let mut expected: Vec<_> = native
                        .witnesses
                        .iter()
                        .map(|&(leaf, pos, take)| (0usize, leaf, pos as u32, json!([2, take])))
                        .collect();
                    expected.sort_by_key(|r| (r.1, r.2, r.3.to_string()));
                    let decision = sql::select_when::expression().replace("?1", &cap.to_string());
                    let sql = format!(
                        "WITH evidence AS MATERIALIZED({evidence}), sw_leaf(leaf,decision,reported) AS(VALUES(0,1,0),(1,0,1)), files(file) AS(VALUES(0),(1)), sw_row(leaf,position,failed,partial,witnesses) AS(SELECT l.leaf,f.file,0,0,COUNT(e.node) FROM sw_leaf l CROSS JOIN files f LEFT JOIN evidence e ON e.clause=l.leaf AND e.file=f.file GROUP BY l.leaf,f.file), decision AS({decision}) SELECT 0 AS clause,column2 AS file,column3 AS node,json_array(column1,column4) AS captures FROM decision ORDER BY file,node"
                    );
                    // Explicit CTE column names avoid backend-generated labels for the compound result.
                    let sql = sql
                        .replace("decision AS(", "decision(tag,leaf,position,take) AS(")
                        .replace(
                            "column2 AS file,column3 AS node,json_array(column1,column4)",
                            "leaf AS file,position AS node,json_array(tag,take)",
                        );
                    let got = run::execute(connection, frames, &compiler.graph, &sql).unwrap();
                    let mut actual = got.hits;
                    actual.sort_by_key(|r| (r.1, r.2, r.3.to_string()));
                    assert_eq!(actual, expected);
                }
            },
        );
    }
}
