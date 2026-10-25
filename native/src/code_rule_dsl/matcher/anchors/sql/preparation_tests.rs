use super::*;
use crate::code_rule_dsl::language::GrammarDialect;

#[test]
fn streaming_structural_columns_match_scalar_decoding() {
    let store = store::Store::temporary().unwrap();
    for (dialect, source) in [
        (
            GrammarDialect::TypeScript,
            "function f(x) { if (x) { return obj.method(x); } }",
        ),
        (
            GrammarDialect::Python,
            "class A:\n def f(self, x):\n  return x + 1\n",
        ),
    ] {
        let doc = tests::document(source, dialect);
        let fid = store.put("input", &doc).unwrap();
        let mut q=store.db.prepare("SELECT p.pre,p.parent,p.end,p.kind,p.field,p.flags,nera_decode(?1,p.pre) FROM nera_nodes(?1,0,?2) p").unwrap();
        let mut rows = q
            .query(rusqlite::params![fid, doc.transducer_len() as i64])
            .unwrap();
        let mut count = 0;
        while let Some(row) = rows.next().unwrap() {
            let packed: Vec<u8> = row.get(6).unwrap();
            let words: Vec<_> = packed
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as i64)
                .collect();
            assert_eq!(row.get::<_, i64>(1).unwrap(), words[0] - 1);
            for (i, value) in words.iter().enumerate().skip(1) {
                assert_eq!(row.get::<_, i64>(i + 1).unwrap(), *value);
            }
            count += 1;
        }
        assert_eq!(count, doc.transducer_len());
        drop(rows);
        drop(q);
        store.readers.clear();
    }
}

#[test]
fn streaming_bounds_are_explicit_and_empty_ranges_are_empty() {
    let store = store::Store::temporary().unwrap();
    let doc = tests::document("f(x);", GrammarDialect::TypeScript);
    let fid = store.put("input", &doc).unwrap();
    let n: i64 = store
        .db
        .query_row("SELECT count(*) FROM nera_nodes(?1,2,2)", [fid], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
    assert!(
        store
            .db
            .query_row("SELECT pre FROM nera_nodes(?1,-1,2)", [fid], |r| r
                .get::<_, i64>(0))
            .is_err()
    );
}

#[test]
#[cfg(feature = "code-rule-profile")]
fn column_patterns_compile_once_for_each_vocabulary_batch() {
    let store = store::Store::temporary().unwrap();
    let before = store.readers.0.lock().counters.regex_compiles;
    let result:i64=store.db.query_row("SELECT sum(nera_term_matches(p.pattern,'apple',0)) FROM (SELECT '^a' AS pattern UNION ALL SELECT '^b') p CROSS JOIN generate_series(0,99,1)",[],|r|r.get(0)).unwrap();
    assert_eq!(result, 100);
    assert_eq!(
        store.readers.0.lock().counters.regex_compiles - before,
        2,
        "column values must not trigger one compilation per term"
    );
}

#[test]
#[cfg(feature = "code-rule-profile")]
fn universal_preparation_preserves_sparse_facts_and_decodes_only_missing_nodes() {
    let store = store::Store::temporary().unwrap();
    let doc = tests::document("target(value);", GrammarDialect::TypeScript);
    let fid = store.put("a.ts", &doc).unwrap();
    store.db.execute("INSERT INTO decoded_properties SELECT ?1,p.pre,p.parent,p.end,p.kind,p.field,p.flags FROM nera_nodes(?1,0,2) p",[fid]).unwrap();
    store.readers.clear();
    let before = store.readers.0.lock().counters;
    execute::prepare_structural(&store, &[fid]).unwrap();
    assert_eq!(
        store.readers.0.lock().counters.since(before).decoded_nodes,
        doc.transducer_len() as u64 - 2
    );
    let before = store.readers.0.lock().counters;
    execute::prepare_structural(&store, &[fid]).unwrap();
    assert_eq!(
        store.readers.0.lock().counters.since(before).decoded_nodes,
        0
    );
}

#[test]
#[cfg(feature = "code-rule-profile")]
fn full_frontier_hoists_universal_demand_and_templates_survive_file_edits() {
    use super::super::{Condition, Direction};
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };
    let doc = tests::document("target(value);", GrammarDialect::TypeScript);
    let lang =
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    let kind = doc.transducer_kind(
        (0..doc.transducer_len())
            .find(|&p| doc.transducer_text_str(p as u32) == "target")
            .unwrap() as u32,
    );
    let always = || AnchorPlan {
        root_kinds: None,
        condition: Condition::Always,
        exact: true,
    };
    let sparse = AnchorPlan {
        root_kinds: Some(BTreeSet::from([kind])),
        condition: Condition::Relation {
            inner: Box::new(always()),
            direction: Direction::Inside,
            recursive: false,
            field: None,
        },
        exact: true,
    };
    let mut plans = vec![Arc::new(sparse)];
    plans.extend((1..32).map(|_| {
        Arc::new(AnchorPlan {
            root_kinds: Some(BTreeSet::from([kind])),
            condition: Condition::Always,
            exact: true,
        })
    }));
    plans.push(Arc::new(AnchorPlan {
        root_kinds: None,
        condition: Condition::Relation {
            inner: Box::new(always()),
            direction: Direction::Has,
            recursive: false,
            field: None,
        },
        exact: true,
    }));
    let plans = BTreeMap::from([(lang, plans)]);
    let corpus = Corpus::default();
    corpus.put("a.ts", &doc, &|| false).unwrap();
    corpus.prepare(&plans).unwrap();
    assert_eq!(
        corpus.preparation_counts(),
        (doc.transducer_len() as u64, 1)
    );
    let statements = corpus.data_statements();
    corpus.prepare(&plans).unwrap();
    assert_eq!(
        corpus.data_statements(),
        statements,
        "unchanged preparation executes no SQL"
    );
    let changed = tests::document("target(other);\nsecond(value);", GrammarDialect::TypeScript);
    corpus.put("a.ts", &changed, &|| false).unwrap();
    corpus.prepare(&plans).unwrap();
    assert_eq!(
        corpus.preparation_counts(),
        ((doc.transducer_len() + changed.transducer_len()) as u64, 1)
    );
}

#[test]
fn candidate_selection_rejects_database_writes_and_rolls_back() {
    let store = store::Store::temporary().unwrap();
    let doc = tests::document("target(value);", GrammarDialect::TypeScript);
    store.put("a.ts", &doc).unwrap();
    let statement = lower::finish(
        Vec::new(),
        "DELETE FROM file RETURNING 0,fid,0".into(),
        Vec::new(),
        std::collections::BTreeSet::new(),
    );
    assert!(
        execute::rows(&store, &statement, |_, _, _| panic!(
            "write must not execute"
        ))
        .is_err()
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn file_scoped_kind_requests_do_not_walk_unrelated_postings() {
    use super::super::Condition;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for structural in [false, true] {
        let store = store::Store::temporary().unwrap();
        let dialect = GrammarDialect::TypeScript;
        let doc = tests::document("target(value);", dialect);
        let fid = store.put("requested.ts", &doc).unwrap();
        if structural {
            execute::prepare_structural(&store, &[fid]).unwrap();
        }
        let kind = dialect.kinds().kind_id("identifier");
        let plan = AnchorPlan {
            root_kinds: Some(std::collections::BTreeSet::from([kind])),
            condition: Condition::Always,
            exact: true,
        };
        let language = u32::from(language::tape_tag(dialect).unwrap());
        let statement = super::tests::planned_batch(&store, &[&plan], language, &[fid]);
        let ticks = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&ticks);
        store
            .db
            .progress_handler(
                100,
                Some(move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let select = || {
            ticks.store(0, Ordering::Relaxed);
            let mut rows = Vec::new();
            execute::rows_for(&store, &statement, &[fid], true, |_, file, pre| {
                rows.push((file, pre));
            })
            .unwrap();
            (rows, ticks.load(Ordering::Relaxed))
        };
        let (before, work) = select();
        assert_eq!(before.len(), 2);
        let decoded = store
            .db
            .query_row("SELECT count(*) FROM decoded_properties", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(
            decoded,
            if structural {
                doc.transducer_len() as i64
            } else {
                0
            }
        );
        store
            .put(
                "unrelated.ts",
                &tests::document(&"other(value);\n".repeat(2000), dialect),
            )
            .unwrap();
        let (after, expanded_work) = select();
        assert_eq!(before, after);
        assert!(
            expanded_work <= work * 2 + 4,
            "covered request work grew with unrelated postings: {work} -> {expanded_work}"
        );
    }
}

#[test]
fn file_scoped_kind_index_survives_edit_removal_and_legacy_upgrade() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let doc = tests::document(
        "target(value);\nfunction broken( {",
        GrammarDialect::TypeScript,
    );
    let store = store::Store::open(file.path()).unwrap();
    let fid = store.put("input.ts", &doc).unwrap();
    let expected = (0..doc.transducer_len())
        .map(|pre| {
            let kind = doc.transducer_kind(pre as u32);
            (fid, if kind == u16::MAX { 0 } else { kind }, pre as u32)
        })
        .collect::<Vec<_>>();
    let rows = |store: &store::Store| {
        let mut q=store.db.prepare("SELECT c.fid,c.code,p.pre FROM file_kind_counts c CROSS JOIN nera_postings(c.postings) p ORDER BY c.fid,p.pre").unwrap();
        q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    assert_eq!(rows(&store), expected);
    drop(store);
    // Reproduce the preceding internal format without a checked-in database fixture.
    let legacy = rusqlite::Connection::open(file.path()).unwrap();
    legacy
        .execute_batch("DROP TRIGGER value_insert;DROP TRIGGER value_delete;DROP TABLE value_frequency;DROP TABLE value_kind_frequency;ALTER TABLE vidx DROP COLUMN n;ALTER TABLE file_kind_counts DROP COLUMN postings; PRAGMA user_version=1")
        .unwrap();
    drop(legacy);
    let store = store::Store::open(file.path()).unwrap();
    assert_eq!(rows(&store), expected);
    assert_eq!(
        store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    drop(store);
    let store = store::Store::open(file.path()).unwrap();
    assert_eq!(rows(&store), expected);
    let changed = tests::document("other(next);", GrammarDialect::TypeScript);
    let next = store.put("input.ts", &changed).unwrap();
    assert!(rows(&store).iter().all(|(file, _, _)| *file == next));
    assert_eq!(rows(&store).len(), changed.transducer_len());
    store.remove("input.ts").unwrap();
    assert!(rows(&store).is_empty());
}

#[test]
fn failed_kind_index_upgrade_keeps_the_legacy_schema_atomic() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let store = store::Store::open(file.path()).unwrap();
    store
        .put(
            "input.ts",
            &tests::document("target(value);", GrammarDialect::TypeScript),
        )
        .unwrap();
    drop(store);
    let legacy = rusqlite::Connection::open(file.path()).unwrap();
    legacy.execute_batch("DROP TRIGGER value_insert;DROP TRIGGER value_delete;DROP TABLE value_frequency;DROP TABLE value_kind_frequency;ALTER TABLE vidx DROP COLUMN n;ALTER TABLE file_kind_counts DROP COLUMN postings; PRAGMA user_version=1; UPDATE file SET rec=substr(rec,1,length(rec)-1)").unwrap();
    drop(legacy);
    assert!(store::Store::open(file.path()).is_err());
    let legacy = rusqlite::Connection::open(file.path()).unwrap();
    assert_eq!(
        legacy
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        legacy
            .query_row(
                "SELECT count(*) FROM pragma_table_info('file_kind_counts') WHERE name='postings'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn kind_requests_preserve_mixed_coverage_error_and_missing_nodes() {
    use super::super::Condition;
    use std::collections::BTreeSet;
    let store = store::Store::temporary().unwrap();
    let dialect = GrammarDialect::TypeScript;
    let documents = [
        "target(value);",
        "function broken( {",
        "function f() {\na();\n",
    ]
    .map(|source| tests::document(source, dialect));
    let files = documents
        .iter()
        .enumerate()
        .map(|(i, doc)| store.put(&format!("{i}.ts"), doc).unwrap())
        .collect::<Vec<_>>();
    execute::prepare_structural(&store, &files[..1]).unwrap();
    let kinds = [
        BTreeSet::from([dialect.kinds().kind_id("identifier")]),
        BTreeSet::from([0]),
        BTreeSet::from([u16::MAX]),
        BTreeSet::from([0, u16::MAX]),
        BTreeSet::from([dialect.kinds().kind_id("}")]),
    ];
    let plans = kinds
        .iter()
        .map(|kinds| AnchorPlan {
            root_kinds: Some(kinds.clone()),
            condition: Condition::Always,
            exact: true,
        })
        .collect::<Vec<_>>();
    let language = u32::from(language::tape_tag(dialect).unwrap());
    let statement =
        super::tests::planned_batch(&store, &plans.iter().collect::<Vec<_>>(), language, &files);
    let mut expected = Vec::new();
    for (qid, kinds) in kinds.iter().enumerate() {
        for (fid, doc) in files.iter().zip(&documents) {
            for pre in 0..doc.transducer_len() {
                if kinds.contains(&doc.transducer_kind(pre as u32)) {
                    expected.push((qid, *fid, pre));
                }
            }
        }
    }
    assert!(expected.iter().any(|(qid, _, _)| *qid == 2));
    assert!(documents.iter().any(|doc| {
        (0..doc.transducer_len()).any(|pre| doc.transducer_flags(pre as u32) & 4 != 0)
    }));
    let select = || {
        let mut actual = Vec::new();
        execute::rows_for(&store, &statement, &files, false, |qid, fid, pre| {
            actual.push((qid, fid, pre));
        })
        .unwrap();
        actual
    };
    assert_eq!(select(), expected);
    execute::prepare_structural(&store, &files).unwrap();
    assert_eq!(
        select(),
        expected,
        "completion must preserve the FTS result"
    );
}

#[test]
fn file_scoped_values_do_not_scan_unrelated_files() {
    use super::super::{Condition, TextProbe};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = store::Store::temporary().unwrap();
    let dialect = GrammarDialect::TypeScript;
    let doc = tests::document("target(value);", dialect);
    let fid = store.put("requested.ts", &doc).unwrap();
    let regex = regex::Regex::new("^target$").unwrap();
    let plan = AnchorPlan {
        root_kinds: Some(std::collections::BTreeSet::from([dialect
            .kinds()
            .kind_id("identifier")])),
        condition: Condition::Text {
            probe: TextProbe::compile(regex.as_str()),
            regex,
        },
        exact: true,
    };
    let statement = super::tests::planned_batch(
        &store,
        &[&plan],
        u32::from(language::tape_tag(dialect).unwrap()),
        &[fid],
    );
    let ticks = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&ticks);
    store
        .db
        .progress_handler(
            100,
            Some(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let select = || {
        ticks.store(0, Ordering::Relaxed);
        let mut rows = Vec::new();
        execute::rows_for(&store, &statement, &[fid], false, |_, file, pre| {
            rows.push((file, pre))
        })
        .unwrap();
        (rows, ticks.load(Ordering::Relaxed))
    };
    let (expected, before) = select();
    assert_eq!(expected.len(), 1);
    for i in 0..200 {
        store.put(&format!("unrelated-{i}.ts"), &doc).unwrap();
    }
    let (actual, after) = select();
    assert_eq!(actual, expected);
    assert!(
        after <= before * 2 + 4,
        "file-scoped value work grew: {before} -> {after}"
    );
}

#[test]
fn file_scoped_structural_reads_do_not_scan_retained_corpus_rows() {
    use super::super::{Condition, Direction};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = store::Store::temporary().unwrap();
    let dialect = GrammarDialect::TypeScript;
    let doc = tests::document("target(value);", dialect);
    let fid = store.put("requested.ts", &doc).unwrap();
    let plan = AnchorPlan {
        root_kinds: None,
        condition: Condition::Relation {
            inner: Box::new(AnchorPlan {
                root_kinds: None,
                condition: Condition::Always,
                exact: true,
            }),
            direction: Direction::Inside,
            recursive: false,
            field: None,
        },
        exact: true,
    };
    let statement = super::tests::planned_batch(
        &store,
        &[&plan],
        u32::from(language::tape_tag(dialect).unwrap()),
        &[fid],
    );
    assert!(statement.universal);
    execute::prepare_structural(&store, &[fid]).unwrap();
    let ticks = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&ticks);
    store
        .db
        .progress_handler(
            100,
            Some(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let select = || {
        ticks.store(0, Ordering::Relaxed);
        let mut rows = Vec::new();
        execute::rows_for(&store, &statement, &[fid], true, |_, file, pre| {
            rows.push((file, pre))
        })
        .unwrap();
        (rows, ticks.load(Ordering::Relaxed))
    };
    let (expected, before) = select();
    assert_eq!(expected.len(), doc.transducer_len() - 1);
    let other = store
        .put(
            "unrelated.ts",
            &tests::document(&"other(value);\n".repeat(2000), dialect),
        )
        .unwrap();
    execute::prepare_structural(&store, &[other]).unwrap();
    let (actual, after) = select();
    assert_eq!(actual, expected);
    assert!(
        after <= before * 2 + 4,
        "scoped structural work grew: {before} -> {after}"
    );
}
