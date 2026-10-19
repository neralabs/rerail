//! Differential tests of the SQL engine against the native matcher, the parity oracle.

mod census;
mod differential;
mod hard_cases;
mod keyed;
mod leaves;
mod nodes;
mod probe;
mod recursion;
mod setops;
mod three_valued;

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
        let files = chunk
            .iter()
            .map(|doc| {
                (
                    doc.clone(),
                    crate::code_rule_dsl::sql::FileContext::default(),
                )
            })
            .collect();
        let found = session.run(files, &[run]).expect("runs");
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
    // `count` groups into a table keyed by row (`Emitter::grouped`). No rule walks over a frame
    // that repeats a key (the nested `has` walks children, one row each): nothing is keyed.
    assert_eq!((stages, clauses), (6, 66));
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

#[test]
fn a_files_documents_are_batch_documents_of_their_own() {
    // A Blade view is two documents (`embed`): its template and the PHP it compiles to, each a
    // tape of one language. A clause over either reads that document alone, beside a PHP file's
    // own tree in the same batch, and reports where its nodes stand in the file.
    let view = "<div>{{ $a }}</div>\n@if ($b)\n  <p>{{ $c }}</p>\n@endif\n";
    let files = snapshot(&[
        ("blade", "a.blade.php", view),
        ("php", "b.php", "<?php\n$a = 1;\nif ($b) { echo $c; }\n"),
    ]);
    let rules = [
        (
            "blade",
            None,
            json!({ "regex": "^.", "precedes": { "stopBy": "end", "regex": "^." } }),
        ),
        (
            "blade",
            Some("compiled-view"),
            json!({ "kind": "variable_name" }),
        ),
        (
            "blade",
            Some("compiled-view"),
            json!({ "kind": "variable_name", "inside": { "stopBy": "end", "kind": "if_statement" } }),
        ),
        ("php", None, json!({ "kind": "variable_name" })),
    ];
    for (language, embedded, rule) in rules {
        let mut ast = json!({ "language": language, "rule": rule });
        if let Some(via) = embedded {
            ast["embedded"] = json!(via);
        }
        let document = crate::code_rule_dsl::test_support::clause_document(json!({
            "id": "ast-match",
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
        }));
        let compiled = crate::code_rule_dsl::test_support::compile_document_with(
            document,
            &serde_json::from_str::<Value>(crate::code_rule_dsl::test_support::CAPABILITIES)
                .expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = same(&compiled, &files);
        let violations = result["violations"].as_array().expect("violations");
        assert!(!violations.is_empty(), "{ast}: {result}");
        // Each clause reads the files of its own language only.
        let path = if language == "blade" {
            "a.blade.php"
        } else {
            "b.php"
        };
        assert!(
            violations.iter().all(|violation| violation["path"] == path),
            "{ast}: {result}"
        );
        // A PHP node of the view reports the template bytes it stands for (`origin`).
        if embedded.is_some() {
            let found = hard_cases::texts(&result, view);
            assert!(
                found
                    .iter()
                    .all(|text| ["$a", "$b", "$c"].contains(&text.as_str())),
                "{ast}: {found:?}"
            );
        }
    }
}
