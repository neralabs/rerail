use super::*;
use crate::code_rule_dsl::{
    language::GrammarDialect,
    tape::{Grammar, TAPE_FORMAT_VERSION, TapeImage},
};
use crate::content_store::prefilter::TOO_BIG;
use ast_grep_core::tree_sitter::StrDoc;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(in crate::code_rule_dsl) fn document(source: &str, lang: GrammarDialect) -> (TapeDoc, Vec<u8>) {
    let native = StrDoc::new(source, lang);
    let grammar = Grammar::new(lang).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
    let bytes = image.to_bytes().unwrap();
    (TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(), bytes)
}

/// A fresh store path: the clock alone is not unique across parallel tests (macOS reads it in
/// microseconds), so a counter disambiguates.
pub(in crate::code_rule_dsl) fn temp() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("rerail-prefilter-{}-{nonce}-{n}.sqlite", std::process::id()))
}

/// Stores `source` and its tape, publishes the tape's prefilter rows, and returns both.
pub(in crate::code_rule_dsl) fn put(store: &ContentStore, source: &str, lang: GrammarDialect) -> (TapeId, TapeDoc) {
    let (doc, bytes) = document(source, lang);
    let grammar = i64::from(language::tape_tag(lang).unwrap());
    let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
    let tape = store.put_tape(source_id, grammar, 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
    publish(store, &[tape], std::slice::from_ref(&doc)).unwrap();
    (tape, doc)
}

fn count(store: &ContentStore, sql: &str) -> i64 {
    store.query_rows(sql, [], |r| r.get(0)).unwrap()[0]
}

pub(in crate::code_rule_dsl) fn open(path: &Path) -> ContentStore {
    ContentStore::open(path).unwrap()
}

/// A rule's compiled matcher, outside any rule document.
pub(in crate::code_rule_dsl) fn matcher(rule: &serde_json::Value, lang: GrammarDialect) -> crate::code_rule_dsl::matcher::AstMatcher {
    let ids = std::collections::BTreeMap::new();
    let utils = crate::code_rule_dsl::compile::UtilScope {
        table: std::sync::Weak::new(),
        ids: &ids,
        options: None,
        python: None,
        transform: None,
        conflicts: None,
        captures: Default::default(),
        calls: None,
        documents: None,
        side: crate::code_rule_dsl::compile::documents::Side::Host,
    };
    crate::code_rule_dsl::compile::compile_ast_rule(rule, lang, "test/rule", &mut Vec::new(), &utils).unwrap()
}

#[test]
fn terms_are_case_exact_grammar_agnostic_and_die_with_their_tape() {
    let path = temp();
    let store = open(&path);
    let lang = GrammarDialect::TypeScript;
    let (tape, doc) = put(&store, "const Target = target(42); // spaced residue\n", lang);
    let base = u32::from(language::tape_tag(lang).unwrap()) * LANG_STRIDE;
    let rows = |store: &ContentStore| {
        store
            .query_rows(
                "SELECT v.term,v.kind,p.pre FROM term_postings v,nera_postings(v.postings) p WHERE v.tape_id=?1 ORDER BY p.pre",
                [tape.row()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, u32>(2)?)),
            )
            .unwrap()
    };
    let indexed = rows(&store);
    for term in ["Target", "target", "const", "42", "// spaced residue"] {
        assert!(indexed.iter().any(|(t, _, _)| t == term), "missing value {term}");
    }
    assert!(!indexed.iter().any(|(t, _, _)| t == "TARGET"));
    for (text, kind, pre) in indexed {
        assert_eq!(text, doc.transducer_text_str(pre));
        assert_eq!(kind, base + u32::from(kind_code(&doc, pre)));
    }
    assert_eq!(count(&store, "SELECT count(*) FROM term_freq WHERE term IN ('Target','target')"), 2);

    // Another connection (a reopened store) decodes postings too.
    drop(store);
    let store = open(&path);
    assert!(!rows(&store).is_empty());
    assert_eq!(store.prefilter_version(tape).unwrap(), Some(crate::content_store::prefilter::PREFILTER_VERSION));

    // An edit is a new tape; the old one's rows go with it, and the counts follow.
    let (replacement, _) = put(&store, "const replacement = 7;", lang);
    store.delete_tape_for_test(tape).unwrap();
    assert_eq!(count(&store, "SELECT count(*) FROM term_freq WHERE term IN ('Target','target')"), 0);
    assert_eq!(store.prefilter_version(tape).unwrap(), None);
    store.remove_prefilter(replacement).unwrap();
    for table in ["term_postings", "term_freq", "term_coverage", "tape_kind_counts", "kind_freq"] {
        assert_eq!(count(&store, &format!("SELECT count(*) FROM {table}")), 0, "{table}");
    }
}

#[test]
fn term_coverage_bounds_residue_across_the_store_and_restores_after_removal() {
    let store = open(&temp());
    let lang = GrammarDialect::TypeScript;
    let source = "// spaced residue\n".repeat(2200);
    let (first, _) = put(&store, &source, lang);
    let (second, _) = put(&store, &format!("{source}\n"), lang);
    let kind = u32::from(language::tape_tag(lang).unwrap()) * LANG_STRIDE + u32::from(lang.kinds().kind_id("comment"));
    let bad: Vec<u8> = store
        .query_rows("SELECT bad FROM term_coverage WHERE tape_id=?1 AND kind=?2", rusqlite::params![second.row(), kind], |r| r.get(0))
        .unwrap();
    assert_ne!(bad[0] & TOO_BIG, 0, "the bound applies to the store, not each tape");
    let n: Vec<i64> = store
        .query_rows("SELECT count(*) FROM term_postings WHERE tape_id=?1 AND kind=?2", rusqlite::params![second.row(), kind], |r| r.get(0))
        .unwrap();
    assert_eq!(n[0], 0, "overflow remains on the coverage fallback path");
    store.remove_prefilter(first).unwrap();
    store.remove_prefilter(second).unwrap();
    put(&store, "// new residue\n", lang);
    assert_eq!(count(&store, "SELECT count(*) FROM term_freq WHERE term='// new residue'"), 1);
}

#[test]
fn posting_decoder_streams_zero_large_deltas_and_rejects_corruption() {
    let store = open(&temp());
    let sql = "SELECT pre FROM nera_postings(?1)";
    let rows = store.query_rows(sql, [vec![0u8, 127, 128, 1, 255, 255, 3]], |r| r.get::<_, u32>(0)).unwrap();
    assert_eq!(rows, [0, 127, 255, 65790]);
    assert!(store.query_rows(sql, [vec![128u8]], |r| r.get::<_, u32>(0)).is_err());
}

#[test]
fn term_postings_and_uncovered_ordinals_partition_every_tape_exactly() {
    let store = open(&temp());
    let long_comment = format!("// {}\n", "large free text ".repeat(40000));
    for (name, source, lang) in [
        ("a.ts", "const café = \"two words\"; target(café);", GrammarDialect::TypeScript),
        ("b.py", "def target(value):\n    return value + 42\n", GrammarDialect::Python),
        ("broken.ts", "function f( {\n debugger;\n", GrammarDialect::TypeScript),
        ("long.ts", long_comment.as_str(), GrammarDialect::TypeScript),
        ("empty.ts", "", GrammarDialect::TypeScript),
    ] {
        let (tape, doc) = put(&store, source, lang);
        let ordinals = store
            .query_rows(
                "SELECT p.pre FROM term_postings v,nera_postings(v.postings) p WHERE v.tape_id=?1 \
                 UNION ALL SELECT p.pre FROM term_coverage c,nera_postings(c.missing) p WHERE c.tape_id=?1 ORDER BY 1",
                [tape.row()],
                |r| r.get::<_, u32>(0),
            )
            .unwrap();
        assert_eq!(ordinals, (0..doc.transducer_len() as u32).collect::<Vec<_>>(), "coverage gap or overlap in {name}");
    }
}

#[test]
fn kind_positions_are_every_preorder_of_the_kind_and_feed_kind_frequencies() {
    assert_eq!([0, 35, 36, 1024, 65535].map(kind_token), ["k0", "kz", "k10", "ksg", "k1ekf"]);
    let store = open(&temp());
    let mut totals = std::collections::BTreeMap::<u32, i64>::new();
    for (source, lang) in [
        ("function x(a) { return a + 1; }\nx(2);", GrammarDialect::TypeScript),
        ("def x(a):\n    return a + 1\nx(2)\n", GrammarDialect::Python),
        ("function f( {\n debugger;\n", GrammarDialect::TypeScript),
    ] {
        let (tape, doc) = put(&store, source, lang);
        let base = u32::from(language::tape_tag(lang).unwrap()) * LANG_STRIDE;
        let mut seen = 0;
        for code in 0..LANG_STRIDE as u16 {
            let positions = store.kind_positions(tape, base + u32::from(code)).unwrap();
            assert!(positions.iter().all(|&pre| kind_code(&doc, pre) == code));
            seen += positions.len();
            *totals.entry(base + u32::from(code)).or_default() += positions.len() as i64;
        }
        assert_eq!(seen, doc.transducer_len(), "every node is in exactly one kind's postings");
    }
    let stored = store.query_rows("SELECT kind,n FROM kind_freq ORDER BY kind", [], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, i64>(1)?))).unwrap();
    assert_eq!(stored, totals.into_iter().filter(|(_, n)| *n > 0).collect::<Vec<_>>());
}
