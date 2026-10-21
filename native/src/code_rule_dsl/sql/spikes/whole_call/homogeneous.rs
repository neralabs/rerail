//! Coalesce compatible rule shapes as plan data rather than hundreds of SQL syntax arms.
use super::{
    graph::{Boundary, Column, Graph},
    run, terminal_set,
};
use crate::code_rule_dsl::sql::tests::differential::tape;
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{exec::FileContext, terminals::Terminal},
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

fn columns(names: &[&str]) -> Vec<Column> {
    names
        .iter()
        .map(|s| Column {
            name: (*s).into(),
            integer: true,
        })
        .collect()
}
pub(super) fn program(rules: usize, kind: u16, terms: usize) -> (Graph, String) {
    let mut graph = Graph::default();
    let domain=graph.add(columns(&["k","file","node","term"]),format!("SELECT ROW_NUMBER() OVER (),file,node,term FROM (SELECT DISTINCT n.file AS file,n.id AS node,p.value AS term FROM json_each('{}') p CROSS JOIN node n ON n.kind={kind})",json!((0..terms).collect::<Vec<_>>())),Boundary::Identity);
    graph.handles.push(("heterogeneous".into(), domain));
    let query = format!(
        "WITH demands(k,file,node,term) AS MATERIALIZED ({}),\n published AS MATERIALIZED (SELECT publish_heterogeneous(k,file,node,term) AS handle FROM demands),\n answers AS MATERIALIZED (SELECT k FROM terminal_set WHERE input=(SELECT handle FROM published) AND k IN (SELECT k FROM demands)),\n rule_plan(rule,term) AS (SELECT CAST(key AS INTEGER),value FROM json_each('{}'))\nSELECT p.rule AS clause,d.file,d.node,json_array() AS captures FROM demands d JOIN answers a ON a.k=d.k JOIN rule_plan p ON p.term=d.term ORDER BY clause,file,node",
        graph.relations[domain].body,
        json!((0..rules).map(|i| i % terms).collect::<Vec<_>>())
    );
    (graph, query)
}
#[test]
fn heterogeneous_terminal_batch_deduplicates_across_six_hundred_rules() {
    let sources = ["f(a,b); g(c);", "a; b; c; a;"];
    let docs: Vec<_> = sources
        .iter()
        .map(|s| (tape(GrammarDialect::TypeScript, s), FileContext::default()))
        .collect();
    let terms = vec![
        Terminal::Regex(regex::Regex::new("^a$").unwrap()),
        Terminal::Regex(regex::Regex::new("^[bc]$").unwrap()),
    ];
    let (graph, query) = program(600, GrammarDialect::TypeScript.kind_id("identifier"), 2);
    crate::code_rule_dsl::sql::exec::with_terminals(docs, terms, |connection, batch, frames| {
        let counts = terminal_set::register(connection, batch, frames).unwrap();
        let got = run::execute(connection, frames, &graph, &query).unwrap();
        assert_eq!(counts.calls.load(Ordering::Relaxed), 1);
        assert_eq!(counts.demands.load(Ordering::Relaxed), 18); // nine identifier occurrences, two terms.
        let per_term = run::execute(
            connection,
            frames,
            &Graph::default(),
            &program_by_term(600, GrammarDialect::TypeScript.kind_id("identifier"), 2),
        )
        .unwrap();
        assert_eq!(got.hits, per_term.hits);
        let mut actual = vec![vec![Vec::new(); 2]; 600];
        for (rule, file, node, _) in got.hits {
            actual[rule][file].push(node);
        }
        for (rule, files) in actual.iter().enumerate() {
            for (file, nodes) in files.iter().enumerate() {
                let root = &batch.files[file].root;
                let expected: Vec<_> = (0..batch.files[file].facts.len())
                    .filter(|&node| {
                        let at = crate::code_rule_dsl::matcher::node_at(root, node);
                        at.kind() == "identifier"
                            && if rule % 2 == 0 {
                                at.text() == "a"
                            } else {
                                matches!(at.text().as_ref(), "b" | "c")
                            }
                    })
                    .collect();
                assert_eq!(*nodes, expected);
            }
        }
        eprintln!("whole-call heterogeneous {}", got.metrics);
    });
}

#[test]
#[ignore = "homogeneous architecture timing; uses RERAIL_WHOLE_CALL_CONFIG/OUTPUT"]
fn homogeneous_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_WHOLE_CALL_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let root = std::path::Path::new(config["corpus"].as_str().unwrap());
    let docs: Vec<_> = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                tape(
                    GrammarDialect::Python,
                    &std::fs::read_to_string(root.join(p.as_str().unwrap())).unwrap(),
                ),
                FileContext::default(),
            )
        })
        .collect();
    let patterns = [
        "^self$",
        "^cls$",
        "^sql$",
        "^query$",
        "^result$",
        "^cursor$",
        "^connection$",
        "^execute$",
    ];
    let rules = 128;
    use crate::code_rule_dsl::{
        matcher::{run_clause, with_file_memo},
        sql,
        sql::tests::hard_cases::compile,
        sql::timing::Started,
        test_support::grammar_matchers,
    };
    let policies: Vec<_> = patterns
        .iter()
        .map(|pattern| {
            compile(
                "python",
                json!({"rule":{"kind":"identifier","regex":pattern}}),
            )
        })
        .collect();
    let grammars: Vec<_> = (0..rules)
        .map(|i| &grammar_matchers(&policies[i % patterns.len()])[0])
        .collect();
    let legacy = sql::exec::compile(&grammars).unwrap();
    let mut session = legacy.session().unwrap();
    let runs: Vec<_> = (0..rules)
        .map(|statement| sql::ClauseRun {
            statement,
            files: (0..docs.len()).map(|f| (f, None)).collect(),
        })
        .collect();
    // Warm prepared statements, but each evaluation constructs a fresh Batch and terminal cache.
    session.run(docs.clone(), &runs).unwrap();
    let before = session.stats();
    let started = Started::now();
    let legacy_hits = session.run(docs.clone(), &runs).unwrap();
    let legacy_time = started.elapsed();
    let legacy_steps = session.stats().statements - before.statements;
    let mut native = Vec::new();
    let started = Started::now();
    for (rule, grammar) in grammars.iter().enumerate() {
        for (file, (doc, _)) in docs.iter().enumerate() {
            let root = ast_grep_core::AstGrep::doc(doc.clone());
            let mut work = usize::MAX;
            with_file_memo(&root, || {
                run_clause(&root, grammar, None, usize::MAX, &mut work, |node, _, _| {
                    native.push((rule, file, node, json!([])));
                    Ok::<_, std::convert::Infallible>(true)
                })
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("native benchmark limit"));
        }
    }
    let native_time = started.elapsed();
    let old: Vec<_> = legacy_hits
        .iter()
        .enumerate()
        .flat_map(|(rule, files)| {
            files.iter().enumerate().flat_map(move |(file, hits)| {
                hits.iter()
                    .map(move |hit| (rule, file, hit.root, json!([])))
            })
        })
        .collect();
    assert_eq!(old, native);
    let out = std::path::PathBuf::from(std::env::var("RERAIL_WHOLE_CALL_OUTPUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let mut alternatives = Vec::new();
    for style in ["per-term", "heterogeneous"] {
        let terms = patterns
            .iter()
            .map(|s| Terminal::Regex(regex::Regex::new(s).unwrap()))
            .collect();
        let (graph, query) = if style == "per-term" {
            (
                Graph::default(),
                program_by_term(
                    rules,
                    GrammarDialect::Python.kind_id("identifier"),
                    patterns.len(),
                ),
            )
        } else {
            program(
                rules,
                GrammarDialect::Python.kind_id("identifier"),
                patterns.len(),
            )
        };
        let started = Started::now();
        let mut result = sql::exec::with_terminals(
            docs.clone(),
            terms,
            |connection, batch, frames| {
                let counts = terminal_set::register(connection, batch, frames).unwrap();
                sql::terminals::calls::TEXT_FILTERS.set(0);
                let got = run::execute(connection, frames, &graph, &query).unwrap();
                assert_eq!(got.hits, native);
                json!({"style":style,"parity":true,"matches":got.hits.len(),"heterogeneousTerminalCalls":counts.calls.load(Ordering::Relaxed),"textTerminalCalls":sql::terminals::calls::TEXT_FILTERS.get(),"distinctDemands":counts.demands.load(Ordering::Relaxed),"metrics":got.metrics})
            },
        );
        let total = started.elapsed();
        result["wholeKernelCpuNs"] = json!(total.cpu);
        result["wholeKernelWallNs"] = json!(total.wall);
        std::fs::write(out.join(format!("{style}.sql")), query).unwrap();
        eprintln!("whole-call homogeneous {result}");
        alternatives.push(result);
    }
    let result = json!({"scope":"128 generated kind/regex rules with eight distinct predicates on the configured real corpus; not the original policy workload", "files":docs.len(),"rules":rules,"native":{"cpuNs":native_time.cpu,"wallNs":native_time.wall},"legacy":{"cpuNs":legacy_time.cpu,"wallNs":legacy_time.wall,"scheduledSteps":legacy_steps},"alternatives":alternatives});
    std::fs::write(
        out.join("homogeneous.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

pub(super) fn program_by_term(rules: usize, kind: u16, terms: usize) -> String {
    let mut definitions = vec![format!(
        "roots(k,file,node) AS MATERIALIZED(SELECT file*4294967296+id,file,id FROM node WHERE kind={kind})"
    )];
    for term in 0..terms {
        definitions.push(format!("t{term}(k) AS MATERIALIZED(SELECT ek FROM text_match WHERE term={term} AND ek IN(SELECT k FROM roots))"));
    }
    definitions.push(format!(
        "matches(term,k) AS({})",
        (0..terms)
            .map(|t| format!("SELECT {t},k FROM t{t}"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ")
    ));
    definitions.push(format!(
        "rule_plan(rule,term) AS(SELECT CAST(key AS INTEGER),value FROM json_each('{}'))",
        json!((0..rules).map(|i| i % terms).collect::<Vec<_>>())
    ));
    format!(
        "WITH {} SELECT p.rule AS clause,(m.k >> 32) AS file,(m.k & 4294967295) AS node,json_array() AS captures FROM matches m JOIN rule_plan p ON p.term=m.term ORDER BY clause,file,node",
        definitions.join(",")
    )
}
