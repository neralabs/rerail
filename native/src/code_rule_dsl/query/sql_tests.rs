use super::catalog::{CatalogBuilder, Term};
use super::{BatchCounters, QueryConnection, tables};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::code_rule_dsl::transducer::sql_backend::{BatchFile, Terminal};
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
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

/// A code_rule's statements are compiled once, by the code_rule compiler: stateless evaluation runs
/// them as compiled, however often it evaluates, `violation.each` and `select_when` sides included.
#[test]
#[ignore = "the SQL engine is restored but is not routed through product compilation"]
fn stateless_evaluation_runs_the_statements_the_compiler_built() {
    use crate::code_rule_dsl::model::{ClauseKind, Query};
    use crate::code_rule_dsl::test_support::{compile_document_with, evaluate_file, violation};
    fn matchers(query: &Query) -> usize {
        match query {
            Query::Ast { matchers, .. } => matchers.len(),
            Query::SelectWhen { select, exists } => matchers(select) + matchers(exists),
            Query::Regex { .. } => 0,
        }
    }
    let mut each = violation();
    each["each"] = json!("X");
    let calls = json!({ "language": "typescript", "rule": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "kind": "identifier", "pattern": "$X" } } } });
    let gate = json!({ "language": "typescript", "rule": { "pattern": "f($$$A)" } });
    let clauses = json!([
        { "id": "plain", "forbid": { "ast": calls.clone() }, "violation": violation() },
        { "id": "each", "forbid": { "ast": calls.clone() }, "violation": each.clone() },
        { "id": "gated", "forbid": { "select_when": { "exists": { "ast": gate }, "select": { "ast": calls } } }, "violation": each },
    ]);
    let before = super::compile::clauses_compiled();
    let code_rule = compile_document_with(
        json!({ "format": "rerail.policy/v1", "id": "once", "revision": 1, "mode": "steer", "clauses": clauses }),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let compiled = code_rule
        .inner
        .clauses
        .iter()
        .map(|clause| match &clause.kind {
            ClauseKind::Forbid(query) => matchers(query),
            _ => unreachable!("every clause is a forbid"),
        })
        .sum::<usize>();
    assert_eq!(super::compile::clauses_compiled() - before, compiled);
    for (language, path) in [
        ("typescript", "a.ts"),
        ("tsx", "a.tsx"),
        ("typescript", "b.ts"),
    ] {
        let result = evaluate_file(&code_rule, language, path, "f(a, b);\n");
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(5),
            "{result}"
        );
    }
    assert_eq!(super::compile::clauses_compiled() - before, compiled);
}

/// `precedes`/`follows` with `stopBy: end` start each candidate's walk at ast-grep's sibling
/// anchor (`next_all`/`prev_all`). Violation the anchor and taking the first sibling past it must
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
        crate::code_rule_dsl::tape::reset_anchor_siblings();
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
        let walked = tables::axis_walk_nodes() + crate::code_rule_dsl::tape::anchor_siblings();
        assert!(
            walked <= SIBLINGS * 9,
            "{axis}: the axis walk and its anchors visited {walked} nodes for {SIBLINGS} siblings"
        );
    }
}

/// A sibling walk through a node `IN` list reads only the listed siblings past its start, in walk
/// order and numbered from 0, however many siblings lie between them; listed nodes that are no
/// such sibling (the start's own children, a sibling behind the walk) are not rows.
#[test]
fn sibling_walk_through_a_node_list_reads_only_the_listed_siblings() {
    const SIBLINGS: u32 = 2_000;
    let file = python("listed.py", &"a();\n".repeat(SIBLINGS as usize));
    let statements = nodes_of_kinds(&file, &[kind_id("expression_statement")]);
    let (first, middle, last) = (statements[0], statements[1_000], statements[1_999]);
    let inner = middle + 1;
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(Arc::new(Vec::new()), vec![file], empty_catalog());
    let listed = format!(
        "[{last},{inner},{},{first},{}]",
        statements[10], statements[1_500]
    );
    for (axis, of, rows) in [
        ("prev_all", middle, vec![(statements[10], 0), (first, 1)]),
        ("next_all", middle, vec![(statements[1_500], 0), (last, 1)]),
        ("next_all", last, vec![]),
    ] {
        tables::reset_axis_walk_nodes();
        let found = connection
            .connection()
            .prepare(&format!(
                "SELECT s.node, s.pos FROM rerail_nodes s \
                 WHERE s.batch = ?1 AND s.file_index = 0 AND s.axis = '{axis}' AND s.of = ?2 \
                   AND s.node IN (SELECT value FROM json_each(?3)) ORDER BY s.pos"
            ))
            .unwrap()
            .query_map((batch.token(), of, &listed), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(found, rows, "{axis} from {of}");
        assert_eq!(tables::axis_walk_nodes(), rows.len(), "{axis} from {of}");
    }
}

/// ESLint no-dupe-keys counts each property's earlier siblings of the same key. The walk reads
/// only the siblings of that key (`Compiler::same_key`), so an object's work grows with its size,
/// not with its square, as a walk over every earlier sibling's does; both report the same keys,
/// duplicates of mixed kinds among keys without a static name included.
#[test]
fn same_key_sibling_counts_grow_linearly_and_report_as_full_walks() {
    use crate::code_rule_dsl::budget::MatchBudget;
    use crate::code_rule_dsl::test_support::{
        check_document, compile_document_with, evaluate_file, no_dupe_keys_ast,
    };
    use std::sync::atomic::AtomicBool;
    let compile = |full: bool| {
        super::compile::set_same_key_off(full);
        let clause = json!({
            "id": "dupe",
            "forbid": { "ast": no_dupe_keys_ast() },
            "violation": { "anchor": "capture.K", "code": "D", "message": "d" },
        });
        let compiled = compile_document_with(
            check_document(clause),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        );
        super::compile::set_same_key_off(false);
        compiled.unwrap_or_else(|error| panic!("{}", error.reason))
    };
    let (linear, full) = (compile(false), compile(true));
    // Violations, and the budget units their evaluation spent.
    let run = |code_rule: &CompiledCodeRuleDsl, source: &str| {
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(code_rule, "typescript", "keys.ts", source);
        let spent = budget.spent();
        drop(budget);
        assert_eq!(result["failures"], json!([]), "{result}");
        let starts = result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
            .collect::<Vec<_>>();
        (starts, spent)
    };
    let mut units = Vec::new();
    for keys in [100, 200, 400, 800] {
        let mut source = "export const o = {\n".to_owned();
        for key in 0..keys {
            source.push_str(&format!("  k{key}: {key},\n"));
        }
        let duplicates =
            "k7: 1, 'k9': 2, [\"k11\"]: 3, 1: 4, [x]: 5, ...y, '1': 6, [x]: 7, 0x1: 8 };\n";
        source.push_str(duplicates);
        let tail = source.len() - duplicates.len();
        let at = |needle: &str| (tail + source[tail..].find(needle).expect(needle)) as u64;
        let reported = vec![at("k7"), at("'k9'"), at("\"k11\""), at("'1'"), at("0x1")];
        let (found, spent) = run(&linear, &source);
        assert_eq!(found, reported, "{keys} keys");
        let (walked, walk_spent) = run(&full, &source);
        assert_eq!(
            walked, reported,
            "{keys} keys, every earlier sibling walked"
        );
        units.push((spent, walk_spent));
    }
    for pair in units.windows(2) {
        let ((before, _), (after, _)) = (pair[0], pair[1]);
        assert!(
            after <= before * 9 / 4,
            "twice the keys spends at most about twice the units: {units:?}"
        );
    }
    // Current Rerail memoizes the matcher oracle's sibling walk too. The restored SQL path must
    // remain linear; it no longer needs the historical oracle to remain quadratic.
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

/// A clause's root statement (`compile_clause`'s `sql`) reads only the nodes of its root kinds,
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

/// `violation.each` runs a clause's `root_sql` (`r.node = ?3`) once per enumeration step: each run
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

/// The statement's violations over `files`, as (file, start byte, end byte), with the batch's
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
fn module_level_getattr_rule() -> CompiledCodeRuleDsl {
    let document = json!({
        "format": "rerail.policy/v1",
        "id": "gate",
        "revision": 1,
        "mode": "steer",
        "clauses": [{
            "id": "module-level-getattr",
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
            "forbid": { "ast": { "language": "python", "rule": {
                "inside": { "kind": "module", "stopBy": "end" },
                "kind": "function_definition",
                "not": { "inside": { "kind": "class_definition", "stopBy": "end" } },
                "regex": "^def __getattr__",
            } } },
        }],
    });
    CodeRuleDslRuntime::compile(
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

/// The matcher's violations for one file as byte spans, and whether it reported any failure.
fn matcher_spans(
    code_rule: &CompiledCodeRuleDsl,
    path: &str,
    source: &str,
) -> (Vec<(i64, i64)>, bool) {
    let result =
        crate::code_rule_dsl::test_support::evaluate_file(code_rule, "python", path, source);
    let mut spans = BTreeSet::new();
    for violation in result["violations"].as_array().unwrap() {
        byte_spans(&violation["span"], &mut spans);
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
    let code_rule = module_level_getattr_rule();
    let files = FIXTURES
        .iter()
        .map(|(path, source)| python(path, source))
        .collect();
    let (rows, counters, plan) = run_module_level_getattr(files);
    let mut expected = Vec::new();
    for (file, (path, source)) in FIXTURES.iter().enumerate() {
        let (spans, failed) = matcher_spans(&code_rule, path, source);
        assert!(!failed);
        expected.extend(spans.into_iter().map(|(start, end)| (file, start, end)));
    }
    // The module-level definition and the one nested in a function; not the method.
    assert_eq!(rows, expected);
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), [0, 2]);
    // One terminal execution for the whole batch, however often SQLite read the table.
    assert_eq!(counters.terminal_executions, [1]);
    assert_eq!(counters.filters, 1, "{counters:?}\n{plan}");
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
    let code_rule = module_level_getattr_rule();
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
        let (spans, failed) = matcher_spans(&code_rule, path, source);
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
        eprintln!("sql violation {} {start}..{end}", sources[*file].0);
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
        batch.text_rows(0, 0).unwrap(),
        vec![(Some((0, 0)), true), (None, false)]
    );
}

#[test]
fn text_terminal_executes_once_for_every_file_in_a_batch() {
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(vec![Terminal::Text(
            Regex::new("secret").unwrap(),
            super::TextSource::Content,
            10,
        )]),
        ["secret one", "two secret"]
            .into_iter()
            .enumerate()
            .map(|(index, source)| {
                BatchFile::text_shared(
                    Arc::new(source.to_owned()),
                    Arc::from(format!("{index}.txt")),
                    Arc::from("text"),
                    None,
                )
            })
            .collect(),
        empty_catalog(),
    );

    assert_eq!(batch.text_rows(0, 0).unwrap(), vec![(Some((0, 6)), true)]);
    assert_eq!(batch.text_rows(0, 1).unwrap(), vec![(Some((4, 10)), true)]);
    assert_eq!(batch.counters().terminal_executions, [1]);
    let counters = batch.execution_counters();
    assert_eq!(counters.statements, 1);
    assert_eq!(counters.rows, 2);
    assert_eq!(counters.terminal_executions, 1);
    assert_eq!(counters.virtual_table_filters, 1);
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

fn python_rule(forbid: Value) -> CompiledCodeRuleDsl {
    let document = json!({
        "format": "rerail.policy/v1",
        "id": "terminals",
        "revision": 1,
        "mode": "steer",
        "clauses": [{
            "id": "clause",
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
            "forbid": forbid,
        }],
    });
    CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap()
}

#[test]
fn text_terminal_observes_the_owner_deadline_on_another_thread() {
    use crate::code_rule_dsl::transducer::sql_backend::{MatchRegistry, register_module};
    let code_rule =
        python_rule(json!({ "regex": { "source": "file.content", "pattern": "needle" } }));
    let plans = super::SqlPlans::compile(&[Arc::clone(&code_rule.inner)]);
    let terminal = plans.regex_terminal(0, 0).expect("a text terminal");
    let owner = crate::code_rule_dsl::budget::MatchBudget::expired_for_test();
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
    use crate::code_rule_dsl::transducer::sql_backend::pattern_label_derivations;
    let code_rule =
        python_rule(json!({ "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }));
    // The first evaluation compiles the statement.
    crate::code_rule_dsl::test_support::evaluate_file(&code_rule, "python", "a.py", "print(0)\n");
    let source = "print(1)\nprint(2)\nlen(3)\n".repeat(20);
    let before = pattern_label_derivations();
    let result =
        crate::code_rule_dsl::test_support::evaluate_file(&code_rule, "python", "a.py", &source);
    assert_eq!(
        result["violations"].as_array().map(Vec::len),
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
    use crate::code_rule_dsl::test_support::{
        compile_typescript_rule, evaluate_file, matched_texts,
    };
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

/// A Python pack code_rule, as the conformance runner registers it.
fn python_pack_rule(rule: &str) -> CompiledCodeRuleDsl {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python/candidates/rules")
        .join(rule);
    CodeRuleDslRuntime::compile(
        std::fs::read_to_string(&path).expect("pack code_rule"),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason))
}

/// A code_rule's result on one file, compiled and evaluated with the work-skipping shortcuts on and
/// off (`set_shortcuts_off`): they must agree, violations and failures alike.
fn agreeing(compile: &dyn Fn() -> CompiledCodeRuleDsl, path: &str, source: &str) -> Value {
    use crate::code_rule_dsl::test_support::evaluate_file;
    crate::code_rule_dsl::budget::set_shortcuts_off(true);
    let without = evaluate_file(&compile(), "python", path, source);
    crate::code_rule_dsl::budget::set_shortcuts_off(false);
    let with = evaluate_file(&compile(), "python", path, source);
    assert_eq!(with, without, "{path}: the shortcuts changed the result");
    with
}

/// The shortcuts that only skip work (the `binds` guards of `binds` and of the `all`/`any` that
/// need one, the environment-free utility memo shared across environments, the anchor-set
/// shortcuts) change no result of F811, F821 or F841: on every Ruff fixture, and on a module that
/// mixes unpacking, assignment chains, walruses, `with` targets, redefinitions and dotted imports.
#[test]
fn python_shortcuts_change_no_pack_result() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python/upstream/ruff/fixtures");
    let mut files = Vec::new();
    let mut pending = vec![fixtures];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("fixtures") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "py" || extension == "pyi")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(files.len() > 50, "{} fixtures", files.len());
    let mixed = "\
import os.path
import os
from typing import overload
import json as json
def f(items):
    a, b = c = (1, 2)
    d = e = f = [3, 4]
    (g, h) = i = j = items
    k = (l := 5)
    m, n = o = p = 6, 7
    with open('x') as (q, r), open('y') as s:
        pass
    t = 1
    t = 2
    u: int = 3
    v += 1
    for w in items:
        pass
    import json
    def os():
        pass
    return [x for x in items if (y := x)]
class C:
    import sys
    z = sys
";
    for rule in [
        "ruff-f811-redefined-while-unused.json",
        "ruff-f821-undefined-name.json",
        "ruff-f841-unused-variable.json",
    ] {
        let compile = || python_pack_rule(rule);
        for file in &files {
            let source = std::fs::read_to_string(file).expect("fixture");
            let path = if file.extension().is_some_and(|extension| extension == "pyi") {
                "fixture.pyi"
            } else {
                "fixture.py"
            };
            agreeing(&compile, path, &source);
        }
        let result = agreeing(&compile, "mixed.py", mixed);
        assert_eq!(result["failures"], json!([]), "{rule}: {result}");
    }
}

/// A guarded `any` whose branch is unknown where its `binds` holds stays unknown there: the guard
/// skips only nodes that name no binding, where the `binds` (so the branch) is no match. Under
/// `not`, an unknown wrongly read as no match would be reported.
#[test]
fn a_binds_guard_never_turns_an_unknown_branch_into_no_match() {
    use crate::code_rule_dsl::test_support::{check_document, compile_document_with};
    let compile = || {
        let clause = json!({
            "id": "guarded",
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "identifier",
                "pattern": "$N",
                "not": { "any": [
                    { "testUnknown": "^u", "binds": { "form": "assignment" } },
                    { "binds": { "form": "import" } },
                ] },
            } } },
            "violation": { "anchor": "capture.N", "code": "G", "message": "g" },
        });
        compile_document_with(
            check_document(clause),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason))
    };
    let source = "u1 = 1\nu2\nimport os\nos\nx = u1\n";
    let result = agreeing(&compile, "guarded.py", source);
    let mut starts = result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    // `u1`'s binding is unknown (no violation); `x`'s binding matches; the import binds its
    // `dotted_name`, not the `os` identifier inside it; that identifier and the references `u2`,
    // `os` and `u1` name no binding, so the `any` is no match there and `not` reports them.
    let at = |needle: &str, from: usize| (from + source[from..].find(needle).expect(needle)) as u64;
    let expected = vec![at("u2", 0), at("os", 10), at("os\n", 18), at("u1", 10)];
    assert_eq!(starts, expected, "{result}");
}

/// F841's work grows linearly with a function's assignments, each an unpacked chain whose
/// identifiers the environment-free utilities are asked about: doubling them at most about
/// doubles the budget units spent.
#[test]
fn f841_work_grows_linearly_with_assignments() {
    use crate::code_rule_dsl::budget::MatchBudget;
    use crate::code_rule_dsl::test_support::evaluate_file;
    use std::sync::atomic::AtomicBool;
    let code_rule = python_pack_rule("ruff-f841-unused-variable.json");
    let mut units = Vec::new();
    for assignments in [500, 1000, 2000] {
        let mut source = "def f(items):\n".to_owned();
        for index in 0..assignments {
            source.push_str(&format!(
                "    a{index}, b{index} = c{index} = d{index} = (items, {index}, (e{index}, items))\n"
            ));
        }
        source.push_str("    return a0, b0, c0, d0\n");
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(&code_rule, "python", "chains.py", &source);
        let spent = budget.spent();
        drop(budget);
        assert_eq!(result["failures"], json!([]), "{result}");
        // `c` and `d` of every assignment but the first are unused; `a` and `b` are unpacked.
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(2 * (assignments - 1)),
            "{assignments} assignments"
        );
        units.push(spent);
    }
    assert!(units[0] > 0, "the evaluation polled its budget: {units:?}");
    for pair in units.windows(2) {
        assert!(
            pair[1] * 4 <= pair[0] * 9,
            "units {units:?}: doubling the assignments more than doubled the work"
        );
    }
}

/// A TypeScript clause of `rule`, compiled with `value` tests lowered to the structural kernel or,
/// when `off`, left to `tq_pred` inside the SQL walks.
fn typescript_clause(rule: Value, off: bool) -> CompiledCodeRuleDsl {
    use crate::code_rule_dsl::test_support::{check_document, compile_document_with, violation};
    super::structure::set_value_lowering_off(off);
    let compiled = compile_document_with(
        check_document(json!({
            "id": "values",
            "forbid": { "ast": { "language": "typescript", "rule": rule } },
            "violation": violation(),
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    );
    super::structure::set_value_lowering_off(false);
    compiled.unwrap_or_else(|error| panic!("{}", error.reason))
}

/// `split-literal-separator`'s `has` to the end over a `string` whose value is `','` is answered
/// by the structural kernel, which remembers each node's answer: nested calls cost work linear in
/// their depth, where a walk over every call's subtree costs its square. Both report every call
/// that holds the separator.
#[test]
fn value_relations_grow_linearly_with_call_depth() {
    use crate::code_rule_dsl::budget::MatchBudget;
    use crate::code_rule_dsl::test_support::evaluate_file;
    use std::sync::atomic::AtomicBool;
    let rule = json!({
        "kind": "call_expression",
        "has": { "stopBy": "end", "kind": "string", "value": { "eq": "," } },
    });
    let (lowered, walked) = (
        typescript_clause(rule.clone(), false),
        typescript_clause(rule, true),
    );
    let run = |code_rule: &CompiledCodeRuleDsl, source: &str| {
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(code_rule, "typescript", "calls.ts", source);
        let spent = budget.spent();
        drop(budget);
        assert_eq!(result["failures"], json!([]), "{result}");
        (result["violations"].as_array().map(Vec::len), spent)
    };
    let mut units = Vec::new();
    for depth in [100, 200, 400] {
        let nested = |bottom: &str| {
            format!(
                "{}g({bottom}){}",
                "f('.', ".repeat(depth),
                ")".repeat(depth)
            )
        };
        let source = format!(
            "export const a = {};\nexport const b = {};\n",
            nested("','"),
            nested("'.'")
        );
        let (found, spent) = run(&lowered, &source);
        assert_eq!(found, Some(depth + 1), "depth {depth}");
        let (walked_found, walk_spent) = run(&walked, &source);
        assert_eq!(walked_found, found, "depth {depth}, every subtree walked");
        units.push((spent, walk_spent));
    }
    for pair in units.windows(2) {
        let ((before, _), (after, _)) = (pair[0], pair[1]);
        assert!(
            after <= before * 9 / 4,
            "twice the depth spends at most about twice the units: {units:?}"
        );
    }
    // Current Rerail memoizes the matcher oracle's subtree walk too. The restored SQL path must
    // remain linear; it no longer needs the historical oracle to remain quadratic.
}

/// `value` tests answered by the structural kernel give the violations `tq_pred` gives inside the
/// SQL walks, along every relation, under `not`, `any` and a utility, where values are known,
/// unknown (a parameter, a call) and followed through bindings.
#[test]
fn lowered_value_tests_match_predicate_calls() {
    use crate::code_rule_dsl::test_support::{evaluate_file, matched_texts};
    let source = "\
const a = 1, b = 'x', c = ',';
let u = g();
f(a, ',');
f(b, h(c, u));
s.split(',');
s.split(c);
q(u, [1, 2, u ? 3 : 4]);
if (x) { k(0); k(u + 3); }
function w(p: string) { return p.split(u, 2) + 1; }
const z = cond ? f(1, 'a') : f(u, 0);
m(m(m(',', p), u), 'b');
n(u + 1, n(1 + 1, [n(2 + 2)]), n(u * 2));
r([1, u, 2, 3], [4, 5, u, 6]);
";
    let rules = [
        json!({ "kind": "call_expression", "has": { "stopBy": "end", "kind": "string", "value": { "eq": "," } } }),
        json!({ "kind": "call_expression", "has": { "stopBy": "end", "value": { "eq": ",", "follow": true } } }),
        json!({ "kind": "call_expression", "not": { "has": { "stopBy": "end", "value": { "eq": 1, "follow": true } } } }),
        json!({ "kind": "call_expression", "not": { "has": { "stopBy": "end", "kind": "binary_expression", "value": { "eq": 5 } } } }),
        json!({ "kind": "identifier", "inside": { "stopBy": "end", "kind": "call_expression",
                "has": { "kind": "arguments", "has": { "value": { "truthy": true } } } } }),
        json!({ "kind": "identifier", "not": { "inside": { "kind": "arguments", "has": { "value": { "constant": true, "follow": true } } } } }),
        json!({ "kind": "number", "precedes": { "stopBy": "end", "value": { "type": "string", "follow": true } } }),
        json!({ "kind": "identifier", "not": { "follows": { "stopBy": "end", "not": { "value": { "constant": true, "follow": true } } } } }),
        json!({ "kind": "number", "follows": { "value": { "gt": 0 } } }),
        json!({ "kind": "number", "not": { "precedes": { "stopBy": "end", "value": { "eq": 9 } } } }),
        json!({ "kind": "number", "not": { "follows": { "stopBy": "end", "value": { "eq": 9 } } } }),
        json!({ "kind": "number", "not": { "inside": { "stopBy": "end", "kind": "call_expression", "has": { "kind": "arguments", "has": { "value": { "eq": 9 } } } } } }),
        json!({ "kind": "expression_statement", "any": [
            { "has": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "value": { "eq": 0 } } } } },
            { "has": { "stopBy": "end", "kind": "identifier", "value": { "gt": 2, "follow": true } } },
        ] }),
        json!({ "kind": "call_expression", "all": [
            { "has": { "stopBy": "end", "kind": "string" } },
            { "not": { "has": { "stopBy": "end", "kind": "identifier", "value": { "type": "string", "follow": true } } } },
        ] }),
    ];
    let mut reported = 0;
    for rule in rules {
        let found = |off: bool| {
            let result = evaluate_file(
                &typescript_clause(rule.clone(), off),
                "typescript",
                "values.ts",
                source,
            );
            assert_eq!(result["failures"], json!([]), "{rule}: {result}");
            matched_texts(&result, source)
        };
        let (lowered, called) = (found(false), found(true));
        assert_eq!(lowered, called, "{rule}");
        reported += lowered.len();
    }
    assert!(reported > 20, "the rules report violations: {reported}");
}
