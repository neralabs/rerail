//! A real recursive utility reduced to a positive witness relation. The interval alternative
//! exploits tree order; the recursive CTE propagates witnesses through parent edges. Neither
//! implements ordered or negated recursive programs by pretending they are monotone.
use super::{graph::Graph, run, tests::decode};
use crate::code_rule_dsl::sql::tests::{differential::tape, hard_cases::compile, same, snapshot};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{self, exec::FileContext, lower::ProgramBuilder, terminals::Terminal},
    test_support::grammar_matchers,
};
use serde_json::json;

#[test]
fn recursive_witnesses_execute_as_one_query_and_keep_the_first_capture() {
    let source = "function f(a){ if(a){ target; g(target); } }\nfunction g(b){ z; }\n";
    let policy = compile(
        "typescript",
        json!({"rule":{"matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^target$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
    );
    same(
        &policy,
        &snapshot(&[
            ("typescript", "a.ts", source),
            ("typescript", "b.ts", "target; target;"),
        ]),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let mut builder = ProgramBuilder::default();
    let shape = builder.clause_each(grammar, None).unwrap();
    let docs = vec![
        tape(GrammarDialect::TypeScript, source),
        tape(GrammarDialect::TypeScript, "target; target;"),
    ];
    let compiled = sql::exec::compile(&[grammar]).unwrap();
    let expected = compiled
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
    let seed = "seed(file,node) AS MATERIALIZED (SELECT file,id FROM node WHERE (file*4294967296+id) IN (SELECT ek FROM text_match WHERE term=0 AND ek IN (SELECT file*4294967296+id FROM node)))";
    let variants = [
        (
            "recursive",
            format!(
                "WITH RECURSIVE {seed}, answer(file,node,witness) AS (SELECT file,node,node FROM seed UNION SELECT a.file,n.parent,a.witness FROM answer a CROSS JOIN node n ON n.file=a.file AND n.id=a.node WHERE n.parent IS NOT NULL) SELECT 0 AS clause,file,node,json_array(MIN(witness)) AS captures FROM answer GROUP BY file,node ORDER BY file,node"
            ),
        ),
        (
            "interval",
            format!(
                "WITH {seed} SELECT 0 AS clause,n.file,n.id AS node,json_array(MIN(s.node)) AS captures FROM node n CROSS JOIN seed s ON s.file=n.file AND s.node>=n.id AND s.node<=n.\"last\" GROUP BY n.file,n.id ORDER BY n.file,n.id"
            ),
        ),
    ];
    sql::exec::with_terminals(
        docs.into_iter()
            .map(|d| (d, FileContext::default()))
            .collect(),
        vec![Terminal::Regex(regex::Regex::new("^target$").unwrap())],
        |connection, _, frames| {
            for (name, query) in variants {
                let got = run::execute(connection, frames, &Graph::default(), &query).unwrap();
                let mut actual = vec![vec![Vec::new(); 2]];
                for (_, file, root, values) in got.hits {
                    actual[0][file].push(decode(&shape, root, &values));
                }
                assert_eq!(actual, expected, "{name} complete capture parity");
                eprintln!("whole-call recursion {name} {}", got.metrics);
            }
        },
    );
}
#[test]
fn sqlite_rejects_multiple_recursive_reads_in_a_recursive_member() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let sql = "WITH RECURSIVE r(x) AS (VALUES(1) UNION SELECT a.x+b.x FROM r a JOIN r b WHERE a.x+b.x<8) SELECT * FROM r";
    let error = connection
        .prepare(sql)
        .expect_err("two recursive reads are not legal")
        .to_string();
    assert!(error.contains("multiple references"), "{error}");
    eprintln!("whole-call recursion barrier: {error}");
}
#[test]
fn ordered_and_negated_recursive_answers_cannot_use_an_insert_only_fixpoint() {
    // An earlier candidate can become available after a later candidate already answered.
    // MIN's selected capture must be replaced, while an insert-only relation retains both.
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let later: i64 = connection
        .query_row("SELECT MIN(w) FROM (SELECT 9 AS w)", [], |r| r.get(0))
        .unwrap();
    let earlier: i64 = connection
        .query_row(
            "SELECT MIN(w) FROM (SELECT 9 AS w UNION ALL SELECT 2)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(later, earlier);
    // A <- not B cannot keep A=true once a newly discovered B becomes true.
    let snapshots: Vec<bool> = connection
        .prepare("SELECT NOT EXISTS(SELECT 1 WHERE b) FROM (SELECT 0 AS b UNION ALL SELECT 1)")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(snapshots, [true, false]);
}

pub(super) fn children(
    op: &crate::code_rule_dsl::sql::plan::Op,
) -> Vec<&crate::code_rule_dsl::sql::plan::Op> {
    use crate::code_rule_dsl::sql::plan::{OfRule, Op};
    match op {
        Op::All(items) | Op::Any(items) | Op::Seq(items) => items.iter().collect(),
        Op::AllOpen(items) => items.iter().map(|x| &x.op).collect(),
        Op::AnyOpen(items) => items.iter().map(|x| &x.0).collect(),
        Op::Not(op) | Op::Unknowable { op, .. } => vec![op],
        Op::Relation { sub, .. }
        | Op::Search { sub, .. }
        | Op::Count { sub, .. }
        | Op::Every { sub, .. }
        | Op::Follow { sub, .. } => vec![sub],
        Op::Call { body, .. } | Op::Parse { body, .. } => vec![body],
        Op::Host { rule, .. } => vec![rule],
        Op::Constrained { rule, constraints } => std::iter::once(rule.as_ref())
            .chain(constraints.iter().map(|x| &x.1))
            .collect(),
        Op::NthCommit { nth, commit } => vec![nth, commit],
        Op::NthChild {
            of_rule: Some(OfRule::Frame(op)),
            ..
        } => vec![op],
        _ => Vec::new(),
    }
}
pub(super) fn features(
    statement: &crate::code_rule_dsl::sql::plan::Statement,
) -> serde_json::Value {
    use crate::code_rule_dsl::sql::plan::Op;
    let mut ops = std::collections::BTreeMap::<&str, usize>::new();
    let mut pending = vec![&statement.op];
    pending.extend(statement.units.iter().map(|u| &u.body));
    while let Some(op) = pending.pop() {
        let name = match op {
            Op::Kinds(_) => "kind",
            Op::Regex(_) => "regex",
            Op::Pattern { env: true, .. } => "bound_pattern",
            Op::Pattern { .. } => "fresh_pattern",
            Op::Fact { reads, .. } if !reads.is_empty() => "capture_fact",
            Op::Fact { .. } => "fact",
            Op::Recur { .. } => "memo_reference",
            Op::Not(_) => "negation",
            Op::Search { .. } => "three_valued_search",
            Op::Relation { .. } => "relation",
            Op::AllOpen(_) | Op::AnyOpen(_) => "three_valued_composition",
            _ => "other",
        };
        *ops.entry(name).or_default() += 1;
        pending.extend(children(op));
    }
    let mut cyclic = std::collections::BTreeSet::new();
    for (unit, body) in statement.units.iter().enumerate() {
        let mut pending = vec![&body.body];
        while let Some(op) = pending.pop() {
            if let Op::Recur { unit: target, .. } = op
                && statement.units[*target].scc == body.scc
            {
                cyclic.insert(unit);
                cyclic.insert(*target);
            }
            pending.extend(children(op));
        }
    }
    json!({"units":statement.units.len(),"cyclicUnits":cyclic.len(),"memoOnlyUnits":statement.units.len()-cyclic.len(),"captureColumns":statement.slots.iter().enumerate().map(|(i,s)|s.columns(i).len()).sum::<usize>(),"operators":ops})
}

#[test]
fn a_full_state_sql_schedule_can_replace_answers_and_report_nonconvergence() {
    // Synthetic transfer system, not a lowering of arbitrary native recursion. B discovers
    // a seed; A observes not-B; W selects the first available witness. Appending answers fails.
    // A second SCC is N <- not N, which oscillates and must report an incomplete answer.
    sql::exec::with_terminals(Vec::new(), Vec::new(), |connection, _, frames| {
        let query = "WITH RECURSIVE state(round,b,a,w,n,previous) AS(VALUES(0,0,0,9,0,'') UNION ALL SELECT round+1,1,NOT b,CASE WHEN b THEN 2 ELSE 9 END,NOT n,json_array(b,a,w,n) FROM state WHERE round<6), stable AS(SELECT *,json_array(b,a,w,n)=previous AS converged FROM state) SELECT 0 AS clause,0 AS file,round AS node,json_array(b,a,w,n,converged,CASE WHEN round=6 AND NOT converged THEN 'incomplete' ELSE 'working' END) AS captures FROM stable ORDER BY round";
        let result = run::execute(connection, frames, &Graph::default(), query).unwrap();
        assert_eq!(result.hits[1].3, json!([1, 1, 9, 1, 0, "working"]));
        assert_eq!(result.hits[2].3, json!([1, 0, 2, 0, 0, "working"]));
        assert_eq!(
            result.hits.last().unwrap().3,
            json!([1, 0, 2, 0, 0, "incomplete"])
        );
        assert_eq!(result.metrics["sqlExecutions"], 1);
    });
}
