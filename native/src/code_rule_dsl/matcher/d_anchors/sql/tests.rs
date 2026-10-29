use super::*;
use crate::code_rule_dsl::{
    language::GrammarDialect,
    tape::{Grammar, TapeImage},
};
use ast_grep_core::{source::SgNode, tree_sitter::StrDoc};
use std::{collections::BTreeSet, sync::Arc};

pub(super) fn document(source: &str, lang: GrammarDialect) -> TapeDoc {
    let native = StrDoc::new(source, lang);
    let grammar = Grammar::new(lang).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

pub(super) fn planned_batch(
    store: &store::Store,
    plans: &[&AnchorPlan],
    lang: u32,
    files: &[i64],
) -> lower::Statement {
    let statistics =
        super::statistics::Statistics::read(&store.db, plans.iter().map(|p| (lang, *p))).unwrap();
    lower::batch_files_planned(
        plans,
        lang,
        files,
        &statistics,
        &mut lower::Fragments::default(),
    )
}

#[test]
fn value_index_is_case_exact_grammar_agnostic_and_survives_edits_and_reopen() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = store::Store::open(file.path()).unwrap();
    let lang = GrammarDialect::TypeScript;
    let doc = document("const Target = target(42); // spaced residue\n", lang);
    let fid = store.put("a.ts", &doc).unwrap();
    let rows = |store: &store::Store| {
        let mut q = store.db.prepare("SELECT v.term,v.kind,p.pre FROM vidx v,nera_postings(v.postings) p WHERE v.fid=?1 ORDER BY p.pre").unwrap();
        q.query_map([fid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let indexed = rows(&store);
    for term in ["Target", "target", "const", "42", "// spaced residue"] {
        assert!(
            indexed.iter().any(|(t, _, _)| t == term),
            "missing value {term}"
        );
    }
    assert!(!indexed.iter().any(|(t, _, _)| t == "TARGET"));
    for (text, kind, pre) in indexed {
        assert_eq!(text, doc.transducer_text_str(pre));
        assert_eq!(kind, doc.transducer_kind(pre));
    }
    let vlex_count: i64 = store
        .db
        .query_row(
            "SELECT count(*) FROM vlex WHERE term IN ('Target','target')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(vlex_count, 2);
    drop(store);
    let store = store::Store::open(file.path()).unwrap();
    assert!(!rows(&store).is_empty());
    store
        .put("a.ts", &document("const replacement = 7;", lang))
        .unwrap();
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT count(*) FROM vlex WHERE term IN ('Target','target')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    store.remove("a.ts").unwrap();
    for table in ["vidx", "vlex", "vcov"] {
        assert_eq!(
            store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn value_coverage_bounds_residue_across_publications_and_restores_after_removal() {
    let store = store::Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let source = "// spaced residue\n".repeat(2200);
    let doc = document(&source, lang);
    store.put("a.ts", &doc).unwrap();
    let second = store.put("b.ts", &doc).unwrap();
    let kind = lang.kinds().kind_id("comment");
    let bad: u8 = store
        .db
        .query_row(
            "SELECT bad FROM vcov WHERE fid=?1 AND kind=?2",
            rusqlite::params![second, kind],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(bad & 4, 0, "the bound applies to the corpus, not each file");
    let n: i64 = store
        .db
        .query_row(
            "SELECT count(*) FROM vidx WHERE fid=?1 AND kind=?2",
            rusqlite::params![second, kind],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "overflow remains on the coverage fallback path");
    store.remove("a.ts").unwrap();
    store
        .put("b.ts", &document("// new residue\n", lang))
        .unwrap();
    let n: i64 = store
        .db
        .query_row(
            "SELECT count(*) FROM vlex WHERE term='// new residue'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn posting_decoder_streams_zero_large_deltas_and_rejects_corruption() {
    let store = store::Store::temporary().unwrap();
    let mut q = store
        .db
        .prepare("SELECT pre FROM nera_postings(?1)")
        .unwrap();
    let rows = q
        .query_map([vec![0u8, 127, 128, 1, 255, 255, 3]], |r| {
            r.get::<_, u32>(0)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows, [0, 127, 255, 65790]);
    assert!(
        q.query_map([vec![128u8]], |r| r.get::<_, u32>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .is_err()
    );
}

#[test]
fn vocabulary_paths_preserve_regex_literals_and_negation_without_source_reads() {
    use crate::code_rule_dsl::matcher::anchors::tests::compile_rule;
    let store = store::Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let source = "target(value); Target(value);\n".repeat(100);
    let doc = document(&source, lang);
    store.put("a.ts", &doc).unwrap();
    let rules = [
        serde_json::json!({"kind":"identifier","regex":"^(?:target)$"}),
        serde_json::json!({"kind":"identifier","not":{"regex":"^(?:target)$"}}),
        serde_json::json!({"pattern":"target($VALUE)"}),
        serde_json::json!({"kind":"identifier","regex":"(?i)^target$"}),
    ];
    let matchers: Vec<_> = rules
        .iter()
        .map(|r| compile_rule(r, lang, "vocabulary", &mut Vec::new()).unwrap())
        .collect();
    let plans: Vec<_> = matchers.iter().map(AnchorPlan::compile).collect();
    let statement = lower::batch(
        &store,
        &plans.iter().collect::<Vec<_>>(),
        u32::from(language::tape_tag(lang).unwrap()),
        None,
        None,
    );
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    let mut counts = [0usize; 4];
    execute::rows(&store, &statement, |q, _, _| counts[q] += 1).unwrap();
    assert_eq!(counts, [100, 300, 100, 200]);
    #[cfg(feature = "code-rule-profile")]
    assert_eq!(
        store.readers.0.lock().counters.since(before).texts,
        0,
        "complete value kinds never decode source spans"
    );
}

#[test]
fn value_postings_and_uncovered_ordinals_partition_every_file_exactly() {
    let store = store::Store::temporary().unwrap();
    let long_comment = format!("// {}\n", "large free text ".repeat(40000));
    for (path, source, lang) in [
        (
            "a.ts",
            "const café = \"two words\"; target(café);",
            GrammarDialect::TypeScript,
        ),
        (
            "b.py",
            "def target(value):\n    return value + 42\n",
            GrammarDialect::Python,
        ),
        (
            "broken.ts",
            "function f( {\n debugger;\n",
            GrammarDialect::TypeScript,
        ),
        ("long.ts", long_comment.as_str(), GrammarDialect::TypeScript),
        ("empty.ts", "", GrammarDialect::TypeScript),
    ] {
        let doc = document(source, lang);
        let fid = store.put(path, &doc).unwrap();
        let mut q=store.db.prepare("SELECT p.pre FROM vidx v,nera_postings(v.postings) p WHERE v.fid=?1 UNION ALL SELECT p.pre FROM vcov c,nera_postings(c.missing) p WHERE c.fid=?1 ORDER BY 1").unwrap();
        let ordinals = q
            .query_map([fid], |r| r.get::<_, u32>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            ordinals,
            (0..doc.transducer_len() as u32).collect::<Vec<_>>(),
            "coverage gap or overlap in {path}"
        );
    }
}

#[test]
fn bare_global_kind_tokens_preserve_every_preorder_and_span() {
    assert_eq!(
        [0, 35, 36, 1024, 65535].map(store::kind_token),
        ["k0", "kz", "k10", "ksg", "k1ekf"]
    );
    let store = store::Store::temporary().unwrap();
    for (path, source, lang) in [
        (
            "a.ts",
            "function x(a) { return a + 1; }\nx(2);",
            GrammarDialect::TypeScript,
        ),
        (
            "b.py",
            "def x(a):\n    return a + 1\nx(2)\n",
            GrammarDialect::Python,
        ),
        (
            "damaged.ts",
            "function f( {\n debugger;\n",
            GrammarDialect::TypeScript,
        ),
    ] {
        let doc = document(source, lang);
        let fid = store.put(path, &doc).unwrap();
        let lang = u32::from(language::tape_tag(lang).unwrap());
        let mut statement = store
            .db
            .prepare("SELECT term,offset FROM ftsk_inst WHERE doc=?1 ORDER BY offset")
            .unwrap();
        let rows = statement
            .query_map([fid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)? as usize))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), doc.transducer_len());
        for (pre, (term, offset)) in rows.iter().enumerate() {
            let node = doc.node(pre as u32);
            let kind = if node.is_error() { 0 } else { node.kind_id() };
            assert_eq!(*term, store::kind_token(lang * 1024 + u32::from(kind)));
            assert_eq!(*offset, pre);
            let (parent, size): (i64, usize) = store
                .db
                .query_row(
                    "SELECT nera_parent(fid,?2),nera_end(fid,?2) FROM file WHERE fid=?1",
                    rusqlite::params![fid, pre as i64],
                    |row| Ok((row.get(0)?, row.get::<_, u32>(1)? as usize)),
                )
                .unwrap();
            assert_eq!(parent, node.parent().map_or(-1, |p| p.node_id() as i64));
            assert!(size > pre);
            let equal: bool = store
                .db
                .query_row(
                    "SELECT nera_equal(?2,fid,?3) FROM file WHERE fid=?1",
                    rusqlite::params![fid, &source[node.range()], pre as i64],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(equal);
        }
    }
}

fn files(store: &store::Store, lang: GrammarDialect, kind: u16, glob: Option<&str>) -> Vec<i64> {
    let plan = AnchorPlan {
        root_kinds: Some(BTreeSet::from([kind])),
        condition: super::super::Condition::Always,
        exact: true,
    };
    let statement = lower::batch(
        store,
        &[&plan],
        u32::from(language::tape_tag(lang).unwrap()),
        None,
        glob,
    );
    let mut files = Vec::new();
    execute::rows(store, &statement, |_, file, _| files.push(file)).unwrap();
    files.sort_unstable();
    files.dedup();
    files
}

#[test]
fn sql_filters_files_and_survives_replace_delete_and_reopen() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = store::Store::open(file.path()).unwrap();
    let lang = GrammarDialect::TypeScript;
    let call = lang.kinds().kind_id("call_expression");
    let a = store.put("src/a.ts", &document("target();", lang)).unwrap();
    store
        .put("test/a.ts", &document("target();", lang))
        .unwrap();
    store
        .put("src/b.ts", &document("const x = 1;", lang))
        .unwrap();
    assert_eq!(files(&store, lang, call, Some("src/*")), [a]);
    assert_eq!(files(&store, lang, call, None).len(), 2);
    store
        .put("src/a.ts", &document("const x = 1;", lang))
        .unwrap();
    assert!(files(&store, lang, call, Some("src/*")).is_empty());
    store.remove("test/a.ts").unwrap();
    drop(store);
    let store = store::Store::open(file.path()).unwrap();
    assert!(files(&store, lang, call, None).is_empty());
    let tokens: i64 = store
        .db
        .query_row("SELECT count(*) FROM ftsk_inst", [], |r| r.get(0))
        .unwrap();
    let nodes: i64 = store
        .db
        .query_row("SELECT sum(nodes) FROM file", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tokens, nodes);
    assert_eq!(
        store
            .db
            .query_row("SELECT sum(n) FROM kfreq", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        nodes
    );
}

#[test]
fn database_grows_on_disk_with_bounded_cache() {
    let store = store::Store::temporary().unwrap();
    let doc = document(&"target(value);\n".repeat(1500), GrammarDialect::TypeScript);
    for n in 0..32 {
        store.put(&format!("src/{n}.ts"), &doc).unwrap();
    }
    let pages: i64 = store
        .db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let width: i64 = store
        .db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let resident = store.resident_bytes().unwrap();
    eprintln!(
        "SQL index: {} bytes on disk, {resident} SQLite cache/schema/statement bytes",
        pages * width
    );
    assert!(pages * width > 2 * 1024 * 1024);
    assert!(resident < 2 * 1024 * 1024);
    assert_eq!(
        store
            .db
            .query_row("PRAGMA temp_store", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .db
            .query_row("PRAGMA mmap_size", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        store
            .db
            .query_row("PRAGMA cache_spill", [], |r| r.get::<_, i64>(0))
            .unwrap()
            > 0
    );
}

#[test]
fn parallel_connections_do_not_share_a_global_page_cache_mutex() {
    let store = store::Store::temporary().unwrap();
    let enabled: bool = store
        .db
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_MEMORY_MANAGEMENT')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !enabled,
        "this SQLite option forces all worker page caches into one mutex-protected LRU group"
    );
}

#[test]
fn incremental_reads_cover_overflows_without_full_columns_or_stale_cursors() {
    let store = store::Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let source = "other(value);\n".repeat(12000) + "target(value);\n";
    let doc = document(&source, lang);
    let fid = store.put("a.ts", &doc).unwrap();
    assert!(doc.transducer_len() > 65535);
    let target = (0..doc.transducer_len())
        .find(|&pre| &source[doc.node(pre as u32).range()] == "target")
        .unwrap();
    let equal: bool = store
        .db
        .query_row(
            "SELECT nera_equal('target',?1,?2)",
            rusqlite::params![fid, target as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert!(equal);
    assert!(
        store.readers.0.lock().read_bytes < 256,
        "one lookup must not load complete BLOB columns"
    );
    let end: u32 = store
        .db
        .query_row("SELECT nera_end(?1,0)", [fid], |r| r.get(0))
        .unwrap();
    assert_eq!(end as usize, doc.transducer_len());
    let last_statement = doc.node(target as u32).parent().unwrap().parent().unwrap();
    assert_eq!(last_statement.parent().unwrap().node_id(), 0);
    assert!(last_statement.node_id() > 65535);
    let parent: i64 = store
        .db
        .query_row(
            "SELECT nera_parent(?1,?2)",
            rusqlite::params![fid, last_statement.node_id() as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent, 0);
    let replacement = document("updated();", lang);
    let fid = store.put("a.ts", &replacement).unwrap();
    let pre = (0..replacement.transducer_len())
        .find(|&pre| &replacement.get_source()[replacement.node(pre as u32).range()] == "updated")
        .unwrap();
    let equal: bool = store
        .db
        .query_row(
            "SELECT nera_equal('updated',?1,?2)",
            rusqlite::params![fid, pre as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert!(equal);
    // Drop closes active BLOB cursors before SQLite destroys its function closures.
    drop(store);
}

#[test]
fn kind_and_path_file_filtering_does_not_read_tape_columns() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let store = store::Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let fid = store.put("src/a.ts", &document("target();", lang)).unwrap();
    store
        .db
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "file",
                column_name: "src" | "rec" | "spans" | "checkpoints" | "flags" | "parb" | "sizb",
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    assert_eq!(
        files(
            &store,
            lang,
            lang.kinds().kind_id("call_expression"),
            Some("src/*")
        ),
        [fid]
    );
}

#[test]
fn interrupted_sql_construction_keeps_all_candidates() {
    let root = AstGrep::doc(document(
        &"other(value);\n".repeat(500),
        GrammarDialect::TypeScript,
    ));
    let mut polls = 0;
    let index = FileAnchorIndex::build_at(
        &root,
        &[],
        usize::MAX,
        || {
            polls += 1;
            true
        },
        "cancelled.ts",
    );
    assert!(!index.complete());
    assert!(matches!(
        index.select(&AnchorPlan::unrestricted()),
        Selection::All
    ));
    assert!(polls > 0);
}

#[test]
fn refused_oversized_index_still_charges_node_exhaustion() {
    let root = AstGrep::doc(document("safe();", GrammarDialect::TypeScript));
    let index = FileAnchorIndex::build(&root, &[], 1, || false);
    assert!(!index.complete());
    assert_eq!(index.visited_nodes(), 2);
    assert!(matches!(
        index.select(&AnchorPlan::unrestricted()),
        Selection::All
    ));
}

#[test]
fn shared_sql_dependencies_do_not_expand_into_exponential_table_references() {
    let store = store::Store::temporary().unwrap();
    let fid = store
        .put("a.ts", &document("f();", GrammarDialect::TypeScript))
        .unwrap();
    let mut ctes = vec!["c0(fid,pre) AS MATERIALIZED (SELECT fid,0 FROM file)".to_owned()];
    for n in 1..=20 {
        ctes.push(format!(
            "c{n}(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM c{} UNION SELECT fid,pre FROM c{})",
            n - 1,
            n - 1
        ));
    }
    let statement = lower::finish(
        ctes,
        "SELECT 0 AS qid,fid,pre FROM c20".to_owned(),
        Vec::new(),
        BTreeSet::new(),
    );
    assert!(!statement.stages.is_empty());
    for _ in 0..2 {
        let mut rows = Vec::new();
        execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
        assert_eq!(rows, [(0, fid, 0)]);
        let tables: u32 = store
            .db
            .query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE type='table' AND name NOT IN ('request_file','structural_demand','source_batch_definition','source_batch_demand','source_batch_stats','source_batch_match')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tables, 0,
            "candidate stages must be released while fixed request schemas remain"
        );
    }
}

#[test]
fn pattern_range_probes_use_indexes_and_actual_literal_selectivity() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let compiled = compiled(
        serde_json::json!({"language":"typescript","rule":{"pattern":"obj.target($ARG)"}}),
    );
    let plan = &grammar_matchers(&compiled)[0].anchors;
    let source = "obj.other(value);\n".repeat(2000) + "obj.target(needle);\n";
    let root = AstGrep::doc(document(&source, GrammarDialect::TypeScript));
    let store = store::Store::temporary().unwrap();
    let fid = store.put("a.ts", root.root().get_doc()).unwrap();
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&steps);
    store
        .db
        .progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let statement = planned_batch(
        &store,
        &[plan],
        u32::from(language::tape_tag(GrammarDialect::TypeScript).unwrap()),
        &[fid],
    );
    let mut roots = Vec::new();
    execute::rows(&store, &statement, |_, _, pre| roots.push(pre)).unwrap();
    assert_eq!(roots.len(), 1);
    let steps = steps.load(Ordering::Relaxed);
    assert!(
        steps < 2_000_000,
        "subtree probes must not rescan each posting for every root: {steps} VM steps"
    );
}

#[test]
fn descendant_relations_use_subtree_ranges_instead_of_ancestor_closures() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let source = "f(".repeat(200) + "target()" + &")".repeat(200);
    for (rule, expected) in [
        (
            serde_json::json!({"kind":"identifier","inside":{"kind":"call_expression","stopBy":"end"}}),
            201,
        ),
        (
            serde_json::json!({"kind":"identifier","inside":{"kind":"call_expression","field":"function","stopBy":"end"}}),
            201,
        ),
        (
            serde_json::json!({"kind":"call_expression","has":{"kind":"identifier","regex":"^target$","stopBy":"end"}}),
            201,
        ),
        (
            serde_json::json!({"kind":"call_expression","has":{"kind":"identifier","field":"function","regex":"^target$","stopBy":"end"}}),
            1,
        ),
    ] {
        let compiled = compiled(serde_json::json!({"rule":rule}));
        let plan = &grammar_matchers(&compiled)[0].anchors;
        let root = AstGrep::doc(document(&source, GrammarDialect::TypeScript));
        let store = store::Store::temporary().unwrap();
        let fid = store.put("a.ts", root.root().get_doc()).unwrap();
        let steps = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&steps);
        store
            .db
            .progress_handler(
                1,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let statement = planned_batch(
            &store,
            &[plan],
            u32::from(language::tape_tag(GrammarDialect::TypeScript).unwrap()),
            &[fid],
        );
        let mut roots = Vec::new();
        execute::rows(&store, &statement, |_, _, pre| roots.push(pre)).unwrap();
        assert_eq!(roots.len(), expected, "{rule}");
        assert!(
            steps.load(Ordering::Relaxed) < 1_000_000,
            "descendant intervals must not expand each ancestor path: {rule}"
        );
    }
}

#[test]
fn corpus_publishes_once_across_partitions_and_updates_only_changed_files() {
    let corpus = Arc::new(Corpus::default());
    let a = document("target(value);", GrammarDialect::TypeScript);
    let b = document("other(value);", GrammarDialect::TypeScript);
    let first = corpus.put("a.ts", &a, &|| false).unwrap();
    corpus.put("b.ts", &b, &|| false).unwrap();
    let changes = {
        let connection = corpus.connection().unwrap();
        connection.db.total_changes()
    };
    // Restored documents and source Arcs differ, but content identity is unchanged.
    let restored = document("target(value);", GrammarDialect::TypeScript);
    for _ in 0..10 {
        assert_eq!(
            corpus
                .put("a.ts", &restored, &|| panic!(
                    "unchanged index must not rebuild"
                ))
                .unwrap(),
            first
        );
        let connection = corpus.connection().unwrap();
        assert_eq!(connection.db.total_changes(), changes);
        assert_eq!(
            connection
                .db
                .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    let before = corpus.connection().unwrap();
    let old: Vec<i64> = files(
        &before,
        GrammarDialect::TypeScript,
        a.node(0).kind_id(),
        None,
    );
    assert_eq!(old.len(), 2);
    let current = corpus.put("a.ts", &b, &|| false).unwrap();
    corpus.remove("b.ts").unwrap();
    // A concurrent reader keeps a coherent snapshot. Membership selects the current version.
    assert_eq!(
        before
            .db
            .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(before);
    let after = corpus.connection().unwrap();
    assert_eq!(
        after
            .db
            .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3,
        "physical versions remain in the owner after logical removal"
    );
    let stored: Vec<u8> = after
        .db
        .query_row("SELECT src FROM file WHERE fid=?1", [current], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, b"other(value);");
    assert!(
        corpus
            .prepared_file(
                "b.ts",
                u32::from(language::tape_tag(GrammarDialect::TypeScript).unwrap()),
                b.index_source_digest()
            )
            .is_none()
    );
    assert_eq!(
        corpus.census().1,
        3,
        "the owner retains immutable versions independently of logical membership"
    );
}

#[test]
fn corpus_connections_share_postings_and_isolate_temporary_tables() {
    let corpus = Arc::new(Corpus::default());
    let doc = document("target(value);", GrammarDialect::TypeScript);
    corpus.put("a.ts", &doc, &|| false).unwrap();
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|threads| {
        for _ in 0..4 {
            let corpus = &corpus;
            let barrier = &barrier;
            threads.spawn(move || {
                let connection = corpus.connection().unwrap();
                connection
                    .db
                    .execute_batch(
                        "CREATE TEMP TABLE independent(value); INSERT INTO independent VALUES(1)",
                    )
                    .unwrap();
                barrier.wait();
                assert_eq!(
                    connection
                        .db
                        .query_row("SELECT count(*) FROM independent", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
                assert_eq!(
                    connection
                        .db
                        .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    1
                );
            });
        }
    });
    let connection = corpus.connection().unwrap();
    assert_eq!(
        connection
            .db
            .query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE name='independent'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn corpus_database_outlives_its_owner_until_the_last_reader_finishes() {
    let corpus = Arc::new(Corpus::default());
    corpus
        .put(
            "a.ts",
            &document("target(value);", GrammarDialect::TypeScript),
            &|| false,
        )
        .unwrap();
    let reader = corpus.connection().unwrap();
    let path = std::path::PathBuf::from(reader.db.path().unwrap());
    drop(corpus);
    assert!(path.exists());
    assert_eq!(
        reader
            .db
            .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(reader);
    assert!(
        !path.exists(),
        "a finished workspace must release its owned database"
    );
}

#[test]
fn candidate_queries_batch_files_and_rules_and_never_run_during_matching() {
    use crate::code_rule_dsl::matcher::anchors::tests::compile_rule;
    fn run(count: usize) -> usize {
        let lang = GrammarDialect::TypeScript;
        let corpus = Arc::new(Corpus::default());
        let doc = document("target(value); other();", lang);
        let matcher = compile_rule(
            &serde_json::json!({"pattern":"target($VALUE)"}),
            lang,
            "batch",
            &mut Vec::new(),
        )
        .unwrap();
        let positive = Arc::new(AnchorPlan::compile(&matcher));
        let clean_matcher = compile_rule(
            &serde_json::json!({"pattern":"absent($VALUE)"}),
            lang,
            "clean",
            &mut Vec::new(),
        )
        .unwrap();
        let clean = Arc::new(AnchorPlan::compile(&clean_matcher));
        let plans = std::collections::BTreeMap::from([(
            u32::from(language::tape_tag(lang).unwrap()),
            vec![Arc::clone(&positive), Arc::clone(&clean)],
        )]);
        for n in 0..count {
            corpus.put(&format!("{n}.ts"), &doc, &|| false).unwrap();
        }
        corpus.prepare(&plans).unwrap();
        let queries = corpus.batch_queries();
        let statements = corpus.data_statements();
        assert_eq!(queries, 1, "one corpus batch for both rule plans");
        let _scope = CorpusSlot::install(Arc::clone(&corpus));
        let root = AstGrep::doc(doc.clone());
        for _ in 0..3 {
            for n in 0..count {
                let view =
                    FileAnchorIndex::build_at(&root, &[], usize::MAX, || false, &format!("{n}.ts"));
                assert_eq!(view.selected(&positive).unwrap().len(), 1);
                assert!(view.selected(&clean).unwrap().is_empty());
            }
            corpus.prepare(&plans).unwrap();
            assert_eq!(
                corpus.data_statements(),
                statements,
                "consumption executes no SQL data statements"
            );
            assert_eq!(
                corpus.batch_queries(),
                queries,
                "partitions must issue no SQL candidate queries"
            );
        }
        // A changed source invalidates this file only, and the next frontier is still batched.
        corpus
            .put("0.ts", &document("absent(value);", lang), &|| false)
            .unwrap();
        corpus.prepare(&plans).unwrap();
        assert_eq!(corpus.batch_queries(), queries + 1);
        statements
    }
    assert_eq!(run(1), run(64));
}

#[test]
fn one_policy_frontier_shares_producers_beyond_compound_select_limits() {
    use crate::code_rule_dsl::matcher::anchors::tests::compile_rule;
    let lang = GrammarDialect::TypeScript;
    let corpus = Arc::new(Corpus::default());
    let doc = document("target(value); other();", lang);
    corpus.put("a.ts", &doc, &|| false).unwrap();
    let matcher = compile_rule(
        &serde_json::json!({"pattern":"target($VALUE)"}),
        lang,
        "frontier",
        &mut Vec::new(),
    )
    .unwrap();
    let plans: Vec<_> = (0..535)
        .map(|_| Arc::new(AnchorPlan::compile(&matcher)))
        .collect();
    corpus
        .prepare(&std::collections::BTreeMap::from([(
            u32::from(language::tape_tag(lang).unwrap()),
            plans.clone(),
        )]))
        .unwrap();
    assert_eq!(
        corpus.batch_queries(),
        1,
        "one shared producer catalog for the complete policy frontier"
    );
    let _scope = CorpusSlot::install(Arc::clone(&corpus));
    let root = AstGrep::doc(doc);
    let view = FileAnchorIndex::build_at(&root, &[], usize::MAX, || false, "a.ts");
    for plan in plans {
        assert_eq!(view.selected(&plan).unwrap().len(), 1);
    }
}

#[test]
fn failed_batch_never_publishes_empty_selections_and_can_retry() {
    use crate::code_rule_dsl::{budget::MatchBudget, matcher::anchors::tests::compile_rule};
    use std::sync::atomic::AtomicBool;
    let lang = GrammarDialect::TypeScript;
    let corpus = Arc::new(Corpus::default());
    let doc = document("target(value);", lang);
    corpus.put("a.ts", &doc, &|| false).unwrap();
    let matcher = compile_rule(
        &serde_json::json!({"pattern":"target($VALUE)"}),
        lang,
        "batch",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = Arc::new(AnchorPlan::compile(&matcher));
    let plans = std::collections::BTreeMap::from([(
        u32::from(language::tape_tag(lang).unwrap()),
        vec![Arc::clone(&plan)],
    )]);
    {
        let _budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), 0);
        assert!(corpus.prepare(&plans).is_err());
    }
    let _scope = CorpusSlot::install(Arc::clone(&corpus));
    let root = AstGrep::doc(doc);
    let view = FileAnchorIndex::build_at(&root, &[], usize::MAX, || false, "a.ts");
    assert!(
        view.selected(&plan).is_none(),
        "failure must retain authoritative matching"
    );
    corpus.prepare(&plans).unwrap();
    let view = FileAnchorIndex::build_at(&root, &[], usize::MAX, || false, "a.ts");
    assert_eq!(view.selected(&plan).unwrap().len(), 1);
}

#[test]
fn batch_count_tracks_policy_frontiers_and_reuses_unchanged_files() {
    use crate::code_rule_dsl::matcher::anchors::tests::compile_rule;
    let lang = GrammarDialect::TypeScript;
    let corpus = Corpus::default();
    let doc = document("target(value);", lang);
    let matcher = compile_rule(
        &serde_json::json!({"pattern":"target($VALUE)"}),
        lang,
        "batch",
        &mut Vec::new(),
    )
    .unwrap();
    let plans = std::collections::BTreeMap::from([(
        u32::from(language::tape_tag(lang).unwrap()),
        (0..65)
            .map(|_| Arc::new(AnchorPlan::compile(&matcher)))
            .collect(),
    )]);
    for n in 0..64 {
        corpus.put(&format!("{n}.ts"), &doc, &|| false).unwrap();
    }
    corpus.prepare(&plans).unwrap();
    assert_eq!(corpus.batch_queries(), 1);
    corpus.prepare(&plans).unwrap();
    assert_eq!(corpus.batch_queries(), 1);
}

#[test]
fn batch_materializes_only_dependencies_of_live_outputs() {
    let store = store::Store::temporary().unwrap();
    let statement = lower::finish(
        vec![
            "unused(fid,pre) AS MATERIALIZED (SELECT bad,bad FROM absent_table)".to_owned(),
            "live(fid,pre) AS MATERIALIZED (SELECT 1,2)".to_owned(),
        ],
        "SELECT 0 AS qid,fid,pre FROM live".to_owned(),
        Vec::new(),
        BTreeSet::from(["unused".to_owned()]),
    );
    assert!(
        statement.stages.is_empty(),
        "discarded rule branches must never construct tables"
    );
    let mut rows = Vec::new();
    execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 2)]);
}
