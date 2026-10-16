//! Differential tests of the SQL engine against the native matcher, the parity oracle.

mod differential;
mod hard_cases;
mod nodes;
mod probe;
mod setops;

use crate::code_rule_dsl::CompiledCodeRuleDsl;
use serde_json::{Value, json};

/// A snapshot of `files`, each `(language, path, source)`.
pub(super) fn snapshot(files: &[(&str, &str, &str)]) -> Value {
    json!({
        "facts": [],
        "files": files
            .iter()
            .map(|(language, path, source)| json!({ "language": language, "path": path, "source": source }))
            .collect::<Vec<_>>(),
        "format": "rerail.repository-snapshot/v1",
        "revision": 0,
    })
}

/// The evaluation of `compiled` over `snapshot` by `engine`, as JSON.
pub(super) fn evaluate(compiled: &CompiledCodeRuleDsl, snapshot: &Value, engine: &str) -> Value {
    let output = compiled
        .evaluate(
            snapshot.to_string(),
            "report".to_owned(),
            Some(engine.to_owned()),
        )
        .unwrap_or_else(|error| panic!("{engine} evaluation fails: {}", error.reason));
    serde_json::from_str(&output).expect("evaluation is JSON")
}

/// Asserts that both engines evaluate `compiled` over `snapshot` alike, and returns the result.
pub(super) fn same(compiled: &CompiledCodeRuleDsl, snapshot: &Value) -> Value {
    let native = evaluate(compiled, snapshot, "native");
    let sql = evaluate(compiled, snapshot, "sql");
    assert_eq!(native, sql, "native and SQL disagree");
    native
}

/// The roots `grammar` matches in each of `docs`, by the SQL engine in batches of `batch` files on
/// one session.
pub(super) fn probe_sql(
    grammar: &crate::code_rule_dsl::matcher::GrammarMatcher,
    docs: &[crate::code_rule_dsl::tape::TapeDoc],
    batch: usize,
) -> (
    Vec<Vec<u32>>,
    std::time::Duration,
    crate::code_rule_dsl::sql::Stats,
) {
    use crate::code_rule_dsl::sql::ClauseRun;
    let started = std::time::Instant::now();
    let compiled = crate::code_rule_dsl::sql::compile(&[grammar]).expect("compiles");
    let mut session = compiled.session().expect("a session");
    let mut roots = Vec::with_capacity(docs.len());
    for chunk in docs.chunks(batch.max(1)) {
        let run = ClauseRun {
            statement: 0,
            files: (0..chunk.len()).map(|file| (file, None)).collect(),
        };
        let found = session.run(chunk.to_vec(), &[run]).expect("runs");
        roots.extend(
            found
                .into_iter()
                .flatten()
                .map(|matches| matches.into_iter().map(|found| found.root).collect()),
        );
    }
    (roots, started.elapsed(), session.stats())
}

/// The statements one batch runs for a sample clause set: each clause's frames, and the stages
/// the clauses share (two clauses stopping at the same rule share its stage).
#[test]
fn a_sample_clause_set_runs_a_bounded_number_of_statements_per_batch() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let functions =
        json!({ "any": [{ "kind": "function_declaration" }, { "kind": "arrow_function" }] });
    let rules = [
        json!({ "rule": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "pattern": "$X" } } } }),
        json!({ "rule": { "pattern": "console.log($$$A)", "inside": { "kind": "function_declaration", "stopBy": functions.clone() } } }),
        json!({ "rule": { "kind": "if_statement", "not": { "has": { "kind": "else_clause" } }, "any": [{ "regex": "^if \\(a" }, { "has": { "pattern": "return $R", "stopBy": "end" } }] } }),
        json!({ "rule": { "kind": "function_declaration", "count": { "has": { "kind": "if_statement", "stopBy": functions }, "min": 2, "as": "N" } } }),
    ];
    let compiled: Vec<CompiledCodeRuleDsl> = rules.into_iter().map(compiled).collect();
    let grammars: Vec<_> = compiled
        .iter()
        .map(|rule| &grammar_matchers(rule)[0])
        .collect();
    let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
    let (stages, clauses) = program.statement_count();
    eprintln!("sample clause set: {stages} stage statements, {clauses} clause statements");
    assert_eq!((stages, clauses), (6, 64));
}

/// Terminals run once per statement over their whole input set, never once per row: a fresh
/// pattern, an env-input pattern and a regex over a file of many candidate nodes.
#[test]
fn terminals_run_once_per_statement_never_per_row() {
    use crate::code_rule_dsl::sql::terminals::calls::{
        FRESH_MATCHES, PATTERN_FILTERS, TEXT_FILTERS,
    };
    use crate::code_rule_dsl::test_support::compiled;
    let rule = compiled(json!({ "rule": {
        "pattern": "$F($$$A)",
        "regex": "^[a-z]",
        "has": { "stopBy": "end", "pattern": "$F" },
    } }));
    let source = "f(f, g);\n".repeat(200);
    for counter in [&FRESH_MATCHES, &PATTERN_FILTERS, &TEXT_FILTERS] {
        counter.set(0);
    }
    let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
    assert_eq!(result["violations"].as_array().map(Vec::len), Some(200));
    assert_eq!(FRESH_MATCHES.get(), 1);
    assert_eq!(PATTERN_FILTERS.get(), 2);
    assert_eq!(TEXT_FILTERS.get(), 1);
}
