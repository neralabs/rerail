use super::catalog::{CatalogBuilder, Term};
use super::{BatchCounters, QueryConnection, tables};
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::policy_dsl::test_support::CAPABILITIES;
use crate::policy_dsl::transducer::sql_backend::{BatchFile, Terminal};
use crate::policy_dsl::{CompiledPolicyDsl, PolicyDslRuntime};
use ast_grep_core::matcher::KindMatcher;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::LanguageExt;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

fn tape(source: &str, language: GrammarDialect) -> Option<TapeDoc> {
    let grammar = Grammar::new(language).ok()?;
    let native = StrDoc::new(source, language);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).ok()?;
    let bytes = image.to_bytes().ok()?;
    let image = TapeImage::from_bytes(&bytes, &grammar).ok()?;
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).ok()
}

fn python(path: &str, source: &str) -> BatchFile {
    BatchFile::new(
        tape(source, GrammarDialect::Python).expect("python tape"),
        Arc::from(path),
        Arc::from("python"),
        None,
    )
}

/// A policy's statements are compiled once, by the policy compiler: stateless evaluation runs
/// them as compiled, however often it evaluates, `finding.each` and `select_when` sides included.
#[test]
fn stateless_evaluation_runs_the_statements_the_compiler_built() {
    use crate::policy_dsl::model::{CheckKind, Query};
    use crate::policy_dsl::test_support::{compile_document_with, evaluate_file, finding};
    fn matchers(query: &Query) -> usize {
        match query {
            Query::Ast { matchers, .. } => matchers.len(),
            Query::SelectWhen { select, exists } => matchers(select) + matchers(exists),
            Query::Regex { .. } => 0,
        }
    }
    let mut each = finding();
    each["each"] = json!("X");
    let calls = json!({ "language": "typescript", "rule": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X" } } } });
    let gate = json!({ "language": "typescript", "rule": { "pattern": "f($$$A)" } });
    let checks = json!([
        { "id": "plain", "forbid": { "ast": calls.clone() }, "finding": finding() },
        { "id": "each", "forbid": { "ast": calls.clone() }, "finding": each.clone() },
        { "id": "gated", "forbid": { "select_when": { "exists": { "ast": gate }, "select": { "ast": calls } } }, "finding": each },
    ]);
    let before = super::compile::checks_compiled();
    let policy = compile_document_with(
        json!({ "format": "rerail.policy/v1", "id": "once", "revision": 1, "checks": checks }),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let compiled = policy
        .inner
        .checks
        .iter()
        .map(|check| match &check.kind {
            CheckKind::Forbid(query) => matchers(query),
            _ => unreachable!("every check is a forbid"),
        })
        .sum::<usize>();
    assert_eq!(super::compile::checks_compiled() - before, compiled);
    for (language, path) in [
        ("typescript", "a.ts"),
        ("tsx", "a.tsx"),
        ("typescript", "b.ts"),
    ] {
        let result = evaluate_file(&policy, language, path, "f(a, b);\n");
        assert_eq!(
            result["findings"].as_array().map(Vec::len),
            Some(5),
            "{result}"
        );
    }
    assert_eq!(super::compile::checks_compiled() - before, compiled);
}

/// `precedes`/`follows` with `stopBy: end` start each candidate's walk at ast-grep's sibling
/// anchor (`next_all`/`prev_all`). Finding the anchor and taking the first sibling past it must
/// cost O(1) per candidate, not a scan of every earlier sibling of the parent.
#[test]
fn precedes_end_walks_only_the_first_later_sibling_per_candidate() {
    const SIBLINGS: usize = 2_000;

    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(Vec::new()),
        vec![python("siblings.py", &"a();\n".repeat(SIBLINGS))],
        Arc::new(super::Catalog {
            terms: Vec::new(),
            ops: Vec::new(),
        }),
    );
    for (axis, related) in [("next_all", SIBLINGS), ("prev_all", SIBLINGS - 1)] {
        tables::reset_axis_walk_nodes();
        crate::policy_dsl::tape::reset_anchor_siblings();
        let matched = connection
            .connection()
            .query_row(
                &format!(
                    "SELECT count(*) FROM rerail_nodes candidate \
                     WHERE candidate.batch = ?1 AND candidate.file_index = 0 \
                       AND candidate.axis = 'all' AND candidate.kind = ?2 \
                       AND EXISTS (SELECT 1 FROM rerail_nodes related \
                                   WHERE related.batch = ?1 AND related.file_index = 0 \
                                     AND related.axis = '{axis}' AND related.of = candidate.node \
                                   LIMIT 1)"
                ),
                (batch.token(), kind_id("expression_statement")),
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        assert_eq!(matched, related as i64, "{axis}");
        let walked = tables::axis_walk_nodes() + crate::policy_dsl::tape::anchor_siblings();
        assert!(
            walked <= SIBLINGS * 9,
            "{axis}: the axis walk and its anchors visited {walked} nodes for {SIBLINGS} siblings"
        );
    }
}

/// The node ids of `kinds` in `file`, in preorder.
fn nodes_of_kinds(file: &BatchFile, kinds: &[i64]) -> Vec<i64> {
    (0..file.len() as u32)
        .filter(|&node| kinds.contains(&i64::from(file.tape().transducer_kind(node))))
        .map(i64::from)
        .collect()
}

fn empty_catalog() -> Arc<super::Catalog> {
    Arc::new(super::Catalog {
        terms: Vec::new(),
        ops: Vec::new(),
    })
}

/// A check's root statement (`compile_check`'s `sql`) reads only the nodes of its root kinds,
/// from the file's kind index, in preorder.
#[test]
fn root_kind_scan_reads_only_nodes_of_the_root_kinds() {
    let source = "def f():\n    a = 1\n    return a\n".repeat(300);
    let file = python("kinds.py", &source);
    let kinds = [kind_id("return_statement"), kind_id("function_definition")];
    let both = nodes_of_kinds(&file, &kinds);
    let first = nodes_of_kinds(&file, &kinds[..1]);
    assert_eq!((both.len(), first.len()), (600, 300));
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(Arc::new(Vec::new()), vec![file], empty_catalog());
    for (sql, wanted) in [
        (
            format!(
                "SELECT r.node FROM rerail_nodes r \
                 WHERE r.batch = ?1 AND r.file_index = 0 AND r.axis = 'all' \
                   AND r.kind IN ({}, {}) ORDER BY r.pos",
                kinds[0], kinds[1]
            ),
            both,
        ),
        (
            format!(
                "SELECT r.node FROM rerail_nodes r \
                 WHERE r.batch = ?1 AND r.file_index = 0 AND r.axis = 'all' AND r.kind = {} \
                 ORDER BY r.pos",
                kinds[0]
            ),
            first,
        ),
    ] {
        tables::reset_axis_walk_nodes();
        let nodes = connection
            .connection()
            .prepare(&sql)
            .unwrap()
            .query_map([batch.token()], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(nodes, wanted, "{sql}");
        assert_eq!(tables::axis_walk_nodes(), wanted.len(), "{sql}");
    }
}

/// `finding.each` runs a check's `root_sql` (`r.node = ?3`) once per enumeration step: each run
/// reads the one node asked for, not every node before it.
#[test]
fn root_lookup_by_node_reads_only_that_node() {
    let source = "def f():\n    a = 1\n    return a\n".repeat(300);
    let file = python("roots.py", &source);
    let kind = kind_id("return_statement");
    let roots = nodes_of_kinds(&file, &[kind]);
    let others = nodes_of_kinds(&file, &[kind_id("function_definition")]);
    let len = file.len() as i64;
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(Arc::new(Vec::new()), vec![file], empty_catalog());
    let sql = format!(
        "SELECT r.node FROM rerail_nodes r \
         WHERE r.batch = ?1 AND r.file_index = 0 AND r.axis = 'all' AND r.kind IN ({kind}) \
           AND r.node = ?2"
    );
    let lookup = |node: Option<i64>| {
        connection
            .connection()
            .prepare_cached(&sql)
            .unwrap()
            .query_map((batch.token(), node), |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    tables::reset_axis_walk_nodes();
    for &root in &roots {
        assert_eq!(lookup(Some(root)), vec![root]);
    }
    assert_eq!(tables::axis_walk_nodes(), roots.len());
    // Another kind's node, and nodes outside the file, are no root.
    for node in [others[0], others[others.len() - 1], len, -1, i64::MAX] {
        assert_eq!(lookup(Some(node)), Vec::<i64>::new(), "{node}");
    }
}

/// `node = NULL` is SQL's NULL comparison: it holds for no row, on every axis.
#[test]
fn null_node_constraint_matches_no_node() {
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(Vec::new()),
        vec![python("null.py", "def f():\n    return 1\n")],
        empty_catalog(),
    );
    for (axis, of) in [
        ("all", ""),
        ("descendant", " AND r.of = 0"),
        ("child", " AND r.of = 0"),
    ] {
        let rows = connection
            .connection()
            .query_row(
                &format!(
                    "SELECT count(*) FROM rerail_nodes r \
                     WHERE r.batch = ?1 AND r.file_index = 0 AND r.axis = '{axis}'{of} \
                       AND r.node = ?2"
                ),
                (batch.token(), Option::<i64>::None),
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "{axis}");
    }
}

/// `nera-data-layer:module-level-getattr-trampoline` as one hand-written statement: a module-level
/// `def __getattr__` that no class contains.
const MODULE_LEVEL_GETATTR: &str = "\
SELECT m.file_index, m.start_ordinal, m.start_byte, m.end_byte
FROM rerail_files f
JOIN rerail_matches m ON m.batch = ?1 AND m.program_index = 0 AND m.file_index = f.file_index
WHERE f.batch = ?1
  AND m.match_index IS NOT NULL
  AND tq_regex(?1, 0, m.file_index, m.start_ordinal, 0) IS NOT NULL
  AND EXISTS (SELECT 1 FROM rerail_nodes a
              WHERE a.batch = ?1 AND a.file_index = m.file_index AND a.axis = 'ancestor'
                AND a.of = m.start_ordinal AND a.kind = ?2)
  AND NOT EXISTS (SELECT 1 FROM rerail_nodes a
                  WHERE a.batch = ?1 AND a.file_index = m.file_index AND a.axis = 'ancestor'
                    AND a.of = m.start_ordinal AND a.kind = ?3)
ORDER BY m.file_index, m.start_ordinal";

fn kind_id(kind: &str) -> i64 {
    i64::from(
        GrammarDialect::Python
            .get_ts_language()
            .id_for_node_kind(kind, true),
    )
}

/// The statement's findings over `files`, as (file, start byte, end byte), with the batch's
/// counters.
fn run_module_level_getattr(
    files: Vec<BatchFile>,
) -> (Vec<(usize, i64, i64)>, BatchCounters, String) {
    let connection = QueryConnection::open().unwrap();
    let mut catalog = CatalogBuilder::default();
    catalog.terminal(Terminal::Kind(
        KindMatcher::new("function_definition", GrammarDialect::Python),
        GrammarDialect::Python,
    ));
    catalog.term(Term::Regex(Regex::new("^def __getattr__").unwrap()));
    let (terminals, catalog) = catalog.finish();
    let batch = connection.register(Arc::new(terminals), files, Arc::new(catalog));
    let parameters = (
        batch.token(),
        kind_id("module"),
        kind_id("class_definition"),
    );
    let plan = connection
        .connection()
        .prepare(&format!("EXPLAIN QUERY PLAN {MODULE_LEVEL_GETATTR}"))
        .unwrap()
        .query_map(parameters, |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    let rows = connection
        .connection()
        .prepare(MODULE_LEVEL_GETATTR)
        .unwrap()
        .query_map(parameters, |row| {
            Ok((
                row.get::<_, i64>(0)? as usize,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (rows, batch.counters(), plan)
}

/// The same rule through the matcher, unscoped.
fn module_level_getattr_policy() -> CompiledPolicyDsl {
    let document = json!({
        "format": "rerail.policy/v1",
        "id": "gate",
        "revision": 1,
        "checks": [{
            "id": "module-level-getattr",
            "finding": { "anchor": "witness.span", "code": "X", "message": "x", "severity": "warning" },
            "forbid": { "ast": { "language": "python", "rule": {
                "inside": { "kind": "module", "stopBy": "end" },
                "kind": "function_definition",
                "not": { "inside": { "kind": "class_definition", "stopBy": "end" } },
                "regex": "^def __getattr__",
            } } },
        }],
    });
    PolicyDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap()
}

fn byte_spans(value: &Value, spans: &mut BTreeSet<(i64, i64)>) {
    match value {
        Value::Object(map) => {
            if let (Some(start), Some(end)) = (
                map.get("startByte").and_then(Value::as_i64),
                map.get("endByte").and_then(Value::as_i64),
            ) {
                spans.insert((start, end));
                return;
            }
            map.values().for_each(|value| byte_spans(value, spans));
        }
        Value::Array(items) => items.iter().for_each(|value| byte_spans(value, spans)),
        _ => {}
    }
}

/// The matcher's findings for one file as byte spans, and whether it reported any failure.
fn matcher_spans(policy: &CompiledPolicyDsl, path: &str, source: &str) -> (Vec<(i64, i64)>, bool) {
    let result = crate::policy_dsl::test_support::evaluate_file(policy, "python", path, source);
    let mut spans = BTreeSet::new();
    for finding in result["findings"].as_array().unwrap() {
        byte_spans(&finding["span"], &mut spans);
    }
    let failed = !result["failures"].as_array().unwrap().is_empty();
    (spans.into_iter().collect(), failed)
}

const FIXTURES: [(&str, &str); 4] = [
    (
        "lazy/__init__.py",
        "import importlib\n\ndef __getattr__(name):\n    return importlib.import_module(name)\n",
    ),
    (
        "proxy.py",
        "class Proxy:\n    def __getattr__(self, name):\n        return name\n",
    ),
    (
        "nested.py",
        "def outer():\n    def __getattr__(name):\n        return name\n    return __getattr__\n",
    ),
    ("plain.py", "def __getattribute__(name):\n    return name\n"),
];

#[test]
fn a_hand_written_statement_matches_the_matcher_and_runs_its_terminal_once_per_batch() {
    let policy = module_level_getattr_policy();
    let files = FIXTURES
        .iter()
        .map(|(path, source)| python(path, source))
        .collect();
    let (rows, counters, plan) = run_module_level_getattr(files);
    let mut expected = Vec::new();
    for (file, (path, source)) in FIXTURES.iter().enumerate() {
        let (spans, failed) = matcher_spans(&policy, path, source);
        assert!(!failed);
        expected.extend(spans.into_iter().map(|(start, end)| (file, start, end)));
    }
    // The module-level definition and the one nested in a function; not the method.
    assert_eq!(rows, expected);
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), [0, 2]);
    // One terminal execution for the whole batch, however often SQLite read the table.
    assert_eq!(counters.terminal_executions, [1]);
    assert!(counters.filters >= FIXTURES.len(), "{counters:?}\n{plan}");
}

#[test]
fn pattern_terminal_rows_carry_captures_and_answer_file_sets() {
    let pattern =
        ast_grep_core::Pattern::try_new("importlib.import_module($NAME)", GrammarDialect::Python)
            .unwrap();
    let files = FIXTURES
        .iter()
        .map(|(path, source)| python(path, source))
        .collect();
    let connection = QueryConnection::open().unwrap();
    let mut catalog = CatalogBuilder::default();
    catalog.pattern(&pattern, GrammarDialect::Python);
    let (terminals, catalog) = catalog.finish();
    let batch = connection.register(Arc::new(terminals), files, Arc::new(catalog));
    let rows = connection
        .connection()
        .prepare(
            "SELECT file_index, capture_label, start_byte, end_byte, complete FROM rerail_matches \
             WHERE batch = ?1 AND program_index = 0 AND file_index IN (0, 1) ORDER BY file_index",
        )
        .unwrap()
        .query_map([batch.token()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let source = FIXTURES[0].1;
    let name = source.rfind("(name)").unwrap() as i64 + 1;
    // File 0 has the `$NAME` capture; file 1's empty pair has its completion row.
    assert_eq!(
        rows,
        [
            (0, Some(0), Some(name), Some(name + 4), 1),
            (1, None, None, None, 1),
        ]
    );
    assert_eq!(batch.counters().terminal_executions, [1]);
}

#[test]
fn scalar_pattern_lookup_only_matches_the_selected_node() {
    let source = format!(
        "{}importlib.import_module(needle)\n",
        "importlib.import_module(other)\n".repeat(2_000)
    );
    let document = tape(&source, GrammarDialect::Python).unwrap();
    let node = (0..document.transducer_len() as u32)
        .rfind(|&node| {
            document.transducer_text_str(node).as_bytes() == b"importlib.import_module(needle)"
        })
        .unwrap();
    let pattern =
        ast_grep_core::Pattern::try_new("importlib.import_module($NAME)", GrammarDialect::Python)
            .unwrap();
    let connection = QueryConnection::open().unwrap();
    let mut catalog = CatalogBuilder::default();
    let term = catalog.pattern(&pattern, GrammarDialect::Python);
    let (terminals, catalog) = catalog.finish();
    let batch = connection.register(
        Arc::new(terminals),
        vec![BatchFile::new(
            document,
            Arc::from("source.py"),
            Arc::from("python"),
            None,
        )],
        Arc::new(catalog),
    );
    let value = connection
        .connection()
        .query_row(
            "SELECT tq_pattern(?1, ?2, 0, ?3, 0)",
            (batch.token(), term as i64, i64::from(node)),
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap();
    assert!(value.is_some());
    assert_eq!(batch.counters().pattern_match_attempts, 1);
}

/// Step 2 gate over a corpus: `RERAIL_QUERY_CORPUS` is a checkout root and
/// `RERAIL_QUERY_MANIFEST` a JSON array of Python paths under it.
#[test]
#[ignore = "corpus gate; set RERAIL_QUERY_CORPUS and RERAIL_QUERY_MANIFEST"]
fn corpus_gate_module_level_getattr_statement_equals_the_matcher() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_QUERY_CORPUS").unwrap());
    let manifest: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_QUERY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let policy = module_level_getattr_policy();
    let mut files = Vec::new();
    let mut sources = Vec::new();
    let mut untaped = 0;
    for path in &manifest {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        match tape(&source, GrammarDialect::Python) {
            Some(doc) => {
                files.push(BatchFile::new(
                    doc,
                    Arc::from(path.as_str()),
                    Arc::from("python"),
                    None,
                ));
                sources.push((path.clone(), source));
            }
            None => untaped += 1,
        }
    }
    let started = std::time::Instant::now();
    let (rows, counters, plan) = run_module_level_getattr(files);
    let sql_elapsed = started.elapsed();
    let started = std::time::Instant::now();
    let mut expected = Vec::new();
    let mut failed_files = 0;
    for (file, (path, source)) in sources.iter().enumerate() {
        let (spans, failed) = matcher_spans(&policy, path, source);
        failed_files += usize::from(failed);
        expected.extend(spans.into_iter().map(|(start, end)| (file, start, end)));
    }
    let matcher_elapsed = started.elapsed();
    eprintln!(
        "files={} untaped={untaped} sql_rows={} matcher_rows={} matcher_failed_files={failed_files} \
         terminal_executions={:?} filters={} sql={sql_elapsed:?} matcher={matcher_elapsed:?}\nplan:\n{plan}",
        sources.len(),
        rows.len(),
        expected.len(),
        counters.terminal_executions,
        counters.filters,
    );
    for (file, start, end) in &rows {
        eprintln!("sql finding {} {start}..{end}", sources[*file].0);
    }
    assert_eq!(rows, expected);
    assert_eq!(counters.terminal_executions, [1]);
}

#[test]
fn text_terminal_keeps_zero_width_multibyte_path_and_limit_completion_rows() {
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(vec![Terminal::Text(
            Regex::new(r"^|é").unwrap(),
            super::TextSource::Path,
            1,
        )]),
        vec![BatchFile::text_shared(
            Arc::new("untaped".to_owned()),
            Arc::from("éxé.ts"),
            Arc::from("text"),
            None,
        )],
        Arc::new(super::Catalog {
            terms: Vec::new(),
            ops: Vec::new(),
        }),
    );
    assert_eq!(
        batch.text_rows(0).unwrap(),
        vec![(Some((0, 0)), true), (None, false)]
    );
}

/// A text-only file has no tree: every node walk over it, the kind index's included, is empty.
#[test]
fn kind_walk_over_a_text_only_file_reads_no_nodes() {
    let kind = kind_id("call");
    let text_only = || {
        BatchFile::text_shared(
            Arc::new("print(1)\n".to_owned()),
            Arc::from("a.py"),
            Arc::from("python"),
            None,
        )
    };
    // A panic inside a SQLite table callback aborts the process: ask the kind index first.
    let file = text_only();
    let nodes = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        file.nodes_of_kind(u16::try_from(kind).unwrap()).to_vec()
    }));
    assert!(
        nodes.as_ref().is_ok_and(Vec::is_empty),
        "a text-only file's kind index is empty"
    );
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(Arc::new(Vec::new()), vec![text_only()], empty_catalog());
    for constraint in [format!(" AND r.kind = {kind}"), String::new()] {
        let sql = format!(
            "SELECT r.node FROM rerail_nodes r \
             WHERE r.batch = ?1 AND r.file_index = 0 AND r.axis = 'all'{constraint}"
        );
        let nodes = connection
            .connection()
            .prepare(&sql)
            .unwrap()
            .query_map([batch.token()], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(nodes, Vec::<i64>::new(), "{sql}");
    }
}

fn python_policy(forbid: Value) -> CompiledPolicyDsl {
    let document = json!({
        "format": "rerail.policy/v1",
        "id": "terminals",
        "revision": 1,
        "checks": [{
            "id": "check",
            "finding": { "anchor": "witness.span", "code": "X", "message": "x", "severity": "warning" },
            "forbid": forbid,
        }],
    });
    PolicyDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap()
}

#[test]
fn text_terminal_observes_the_owner_deadline_on_another_thread() {
    use crate::policy_dsl::transducer::sql_backend::{MatchRegistry, register_module};
    let policy =
        python_policy(json!({ "regex": { "source": "file.content", "pattern": "needle" } }));
    let plans = super::SqlPlans::compile(&[Arc::clone(&policy.inner)]);
    let terminal = plans.regex_terminal(0, 0).expect("a text terminal");
    let owner = crate::policy_dsl::budget::MatchBudget::expired_for_test();
    let registry = Arc::new(MatchRegistry::default());
    let token = registry.register_query_batch(
        Arc::clone(&plans.terminals),
        vec![BatchFile::text_shared(
            Arc::new("needle\n".to_owned()),
            Arc::from("a.py"),
            Arc::from("python"),
            None,
        )],
        Arc::new(()),
        Some(owner.deadline()),
    );
    // The table runs on a thread that never started the budget, as a Rayon worker would.
    let complete = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        register_module(&connection, registry).unwrap();
        connection
            .query_row(
                "SELECT min(complete) FROM rerail_matches WHERE batch = ?1 AND program_index = ?2",
                (token, terminal as i64),
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(
        complete, 0,
        "the spent owner deadline leaves the text terminal incomplete"
    );
    drop(owner);
}

#[test]
fn scalar_pattern_lookups_derive_no_capture_labels_per_node() {
    use crate::policy_dsl::transducer::sql_backend::pattern_label_derivations;
    let policy = python_policy(
        json!({ "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }),
    );
    // The first evaluation compiles the statement.
    crate::policy_dsl::test_support::evaluate_file(&policy, "python", "a.py", "print(0)\n");
    let source = "print(1)\nprint(2)\nlen(3)\n".repeat(20);
    let before = pattern_label_derivations();
    let result = crate::policy_dsl::test_support::evaluate_file(&policy, "python", "a.py", &source);
    assert_eq!(
        result["findings"].as_array().map(Vec::len),
        Some(40),
        "{result}"
    );
    assert_eq!(
        pattern_label_derivations() - before,
        0,
        "a pattern's capture labels are derived once, when the statement compiles"
    );
}

/// SPEC-S4: a three-valued `any` whose earlier branch is unknown and could bind captures does not
/// let a later matching branch substitute its own captures; it stays unknown (no match). An
/// unknown branch that binds nothing still lets a later branch decide.
#[test]
fn a_later_any_branch_does_not_stand_in_for_an_unknown_binding_branch() {
    use crate::policy_dsl::test_support::{compile_typescript_rule, evaluate_file, matched_texts};
    let texts = |rule: Value| {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "f(u);\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    };
    // An identifier that is unknown when it is `u`, true when it is `x`, false otherwise.
    let u_or_x = json!({ "kind": "identifier", "regex": "^(u|x)$", "testUnknown": "^u$" });
    let later = json!({ "has": { "stopBy": "end", "pattern": "$A", "kind": "identifier" } });
    let binding = json!({ "kind": "call_expression", "any": [
        { "has": { "stopBy": "end", "pattern": "$A", "all": [u_or_x.clone()] } },
        later.clone(),
    ] });
    assert_eq!(texts(binding), Vec::<String>::new());
    let capture_free = json!({ "kind": "call_expression", "any": [
        { "has": { "stopBy": "end", "all": [u_or_x] } },
        later,
    ] });
    assert_eq!(texts(capture_free), ["f(u) A=f"]);
}
