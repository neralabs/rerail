use super::*;
use crate::code_rule_dsl::{
    language::{self, GrammarDialect},
    fixtures::temp, prefilter::fixtures::{open, put},
};
use crate::content_store::ContentStore;

use crate::code_rule_dsl::search_index::kind_code;
use std::collections::BTreeMap;

type Counts = Vec<(i64, i64, i64)>;

/// The store's kind sizes, as the SQL route reads them (`kind_size`, a table over the kept counts).
fn counts(store: &ContentStore) -> Counts {
    store.query_rows("SELECT kind,n,size FROM kind_size ORDER BY 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap()
}

/// The counts of `docs`, from their nodes directly.
fn direct(docs: &[(GrammarDialect, &TapeDoc)]) -> Counts {
    let mut kinds = BTreeMap::<i64, (i64, i64)>::new();
    for (lang, doc) in docs {
        let base = i64::from(language::tape_tag(*lang).unwrap()) * i64::from(LANG_STRIDE);
        for pre in 0..doc.transducer_len() as u32 {
            let kind = base + i64::from(kind_code(doc, pre));
            let entry = kinds.entry(kind).or_default();
            entry.0 += 1;
            entry.1 += doc.transducer_children(pre).map(|child| 1 + doc.transducer_descendants(child).len() as i64).sum::<i64>();
        }
    }
    kinds.into_iter().map(|(kind, (n, size))| (kind, n, size)).collect()
}

const SOURCES: [(&str, GrammarDialect); 5] = [
    ("function x(a) { return a + 1; }\nx(2);", GrammarDialect::TypeScript),
    ("const o = { k: f(1), g: [2, 3] };", GrammarDialect::TypeScript),
    ("def x(a):\n    return a + 1\nx(2)\n", GrammarDialect::Python),
    ("class C:\n    def m(self, b=4):\n        return self.b\n", GrammarDialect::Python),
    ("function f( {\n debugger;\n", GrammarDialect::TypeScript),
];

/// The kind sizes the store counts (runs of deltas, `content_store::prefilter::kept`) equal a direct count of the
/// tapes it holds, through a collected tape, a republished one and every one removed.
#[test]
fn kind_sizes_equal_a_direct_count_and_follow_removal() {
    let store = open(&temp());
    let published = SOURCES.map(|(source, lang)| (put(&store, source, lang), lang));
    let docs = published.iter().map(|((_, doc), lang)| (*lang, doc)).collect::<Vec<_>>();
    let all = direct(&docs);
    assert_eq!(counts(&store), all);

    // A collected tape counts down with its rows; a republished one replaces itself.
    let (tape, _) = &published[1].0;
    store.delete_tape_for_test(*tape).unwrap();
    let rest = docs.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, d)| *d).collect::<Vec<_>>();
    assert_eq!(counts(&store), direct(&rest));
    let ((tape, doc), _) = &published[2];
    crate::code_rule_dsl::prefilter::publish(&store, &[*tape], std::slice::from_ref(doc)).unwrap();
    assert_eq!(counts(&store), direct(&rest));

    for ((tape, _), _) in &published {
        store.remove_prefilter(*tape).unwrap();
    }
    assert_eq!(counts(&store), Vec::new());
}

#[test]
fn incremental_publication_equals_one_shot() {
    // T's `check`: the documents in three batches, plus some removed and re-added.
    let one = open(&temp());
    let docs = SOURCES.map(|(source, lang)| put(&one, source, lang));
    let docs = docs.into_iter().map(|(_, doc)| doc).collect::<Vec<_>>();
    let expected = counts(&one);

    let store = open(&temp());
    let stored = SOURCES.map(|(source, lang)| put(&store, source, lang).0);
    for tape in &stored {
        store.remove_prefilter(*tape).unwrap();
    }
    assert_eq!(counts(&store), Vec::new());
    use crate::code_rule_dsl::prefilter::publish;
    for range in [2..4, 0..2, 4..5] {
        publish(&store, &stored[range.clone()], &docs[range]).unwrap();
    }
    for tape in &stored[1..3] {
        store.remove_prefilter(*tape).unwrap();
    }
    publish(&store, &stored[1..3], &docs[1..3]).unwrap();
    assert_eq!(counts(&store), expected);
}
