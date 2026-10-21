//! Actual capture-bearing grammar relations composed with cross-file anti-joins and budgets.
//! This closes the spike's simple name/file projection contract, not every policy operand type.
use super::{adapter::Compiler, graph::Policy, run, tests::columns};
use crate::code_rule_dsl::sql::tests::{differential::tape, hard_cases::compile};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    query::crossfile::{self, Relation, Requirement},
    sql::{
        self, emit,
        exec::{Captured, FileContext},
        lower::ProgramBuilder,
    },
    test_support::grammar_matchers,
};
use serde_json::json;

#[test]
fn captures_feed_cross_file_requirements_and_budgets_in_one_statement() {
    let sources = [
        "use(a); use(b); use(α);",
        "provide(a); use(a); use(c);",
        "provide(b); provide(α);",
    ];
    let p0 = compile("typescript", json!({"rule":{"pattern":"use($X)"}}));
    let p1 = compile("typescript", json!({"rule":{"pattern":"provide($X)"}}));
    let grammars = vec![&grammar_matchers(&p0)[0], &grammar_matchers(&p1)[0]];
    let docs: Vec<_> = sources
        .iter()
        .map(|s| tape(GrammarDialect::TypeScript, s))
        .collect();
    let runs: Vec<_> = (0..2)
        .map(|statement| sql::ClauseRun {
            statement,
            files: vec![(0, None), (1, None), (2, None)],
        })
        .collect();
    let found = sql::exec::compile(&grammars)
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
    let mut subjects = Relation::default();
    let mut evidence = Relation::default();
    let mut subject_files = Vec::new();
    for (rule, files) in found.iter().enumerate() {
        for (file, hits) in files.iter().enumerate() {
            let root = ast_grep_core::AstGrep::doc(docs[file].clone());
            for hit in hits {
                let Some((_, Captured::Node(id))) =
                    hit.captures.iter().find(|(name, _)| name == "X")
                else {
                    panic!("X capture");
                };
                let row=json!({"name":crate::code_rule_dsl::matcher::node_at(&root,*id).text(),"file":file}).as_object().unwrap().clone();
                if rule == 0 {
                    subjects.rows.push(row);
                    subject_files.push(file);
                } else {
                    evidence.rows.push(row);
                }
            }
        }
    }
    let mut compiler = Compiler::default();
    let mut builder = ProgramBuilder::default();
    let mut outputs = Vec::new();
    for (i, g) in grammars.iter().enumerate() {
        let s = builder.clause_each(g, None).unwrap();
        outputs.push((
            i,
            compiler
                .emitted(
                    &emit::clause(&s),
                    &columns(&s),
                    &builder.stages,
                    "SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2",
                )
                .unwrap(),
        ));
    }
    let grammar_sql = compiler.graph.render(&outputs, Policy::Reuse);
    let source_rows = sources
        .iter()
        .enumerate()
        .map(|(i, s)| format!("({i},'{}')", s.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        builder.terminals,
        |connection, _, frames| {
            for incomplete in [false, true] {
                if incomplete {
                    evidence.incomplete.insert("unread.ts".into());
                    subjects.incomplete.insert("unread.ts".into());
                }
                for maximum in [0, 1, 3] {
                    let requirements = crossfile::require_each(
                        ("s", &subjects),
                        None,
                        ("e", &evidence),
                        Some(&json!({"eq":["s.name","e.name"]})),
                        || false,
                    )
                    .unwrap();
                    let groups = crossfile::budget(
                        ("u", &subjects),
                        None,
                        &["u.name".into()],
                        &[],
                        maximum,
                        || false,
                    )
                    .unwrap();
                    let mut expected = Vec::new();
                    for requirement in requirements {
                        let (index, status) = match requirement {
                            Requirement::Missing(i) => (i, "missing"),
                            Requirement::Unknown(i) => (i, "unknown"),
                        };
                        expected.push((
                            0usize,
                            subject_files[index],
                            index as u32,
                            json!([status]),
                        ));
                    }
                    for group in groups {
                        expected.push((
                            1,
                            0,
                            group.first as u32,
                            json!([group.key, group.count, group.partial]),
                        ));
                    }
                    expected.sort_by_key(|r| (r.0, r.1, r.2, r.3.to_string()));
                    let query = format!(
                        "WITH matches AS MATERIALIZED({grammar_sql}), sources(file,source) AS(VALUES {source_rows}), projected AS MATERIALIZED(SELECT m.clause,m.file,m.node,CAST(substr(CAST(s.source AS BLOB),n.start+1,n.\"end\"-n.start) AS TEXT) AS name FROM matches m JOIN sources s ON s.file=m.file JOIN node n ON n.file=m.file AND n.id=json_extract(m.captures,'$[0]') WHERE m.clause>=0), subjects AS MATERIALIZED(SELECT ROW_NUMBER() OVER(ORDER BY file,node)-1 AS rid,* FROM projected WHERE clause=0), provided AS(SELECT * FROM projected WHERE clause=1), groups AS(SELECT MIN(rid) AS first,json_quote(name) AS key,COUNT(*) AS n FROM subjects GROUP BY name) SELECT 0 AS clause,s.file,s.rid AS node,json_array('{}') AS captures FROM subjects s WHERE NOT EXISTS(SELECT 1 FROM provided e WHERE e.name IS s.name) UNION ALL SELECT 1,0,first,json_array(key,n,json('{}')) FROM groups WHERE n>{maximum} OR {} ORDER BY clause,file,node",
                        if incomplete { "unknown" } else { "missing" },
                        incomplete,
                        usize::from(incomplete)
                    );
                    let got = run::execute(connection, frames, &compiler.graph, &query).unwrap();
                    let mut actual = got.hits;
                    actual.sort_by_key(|r| (r.0, r.1, r.2, r.3.to_string()));
                    assert_eq!(actual, expected);
                }
            }
        },
    );
}
