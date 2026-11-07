use super::*;
use crate::code_rule_dsl::{
    language::{self, GrammarDialect},
    prefilter::tests::{open, put, temp},
};
use crate::content_store::ContentStore;

type Counts = (Vec<(i64, i64, i64)>, Vec<(i64, i64, i64, i64)>);

fn counts(store: &ContentStore) -> Counts {
    let kinds = store
        .query_rows("SELECT kind,n,size FROM kind_size ORDER BY 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    let edges = store
        .query_rows("SELECT pkind,ckind,field,n FROM edge_freq ORDER BY 1,2,3", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    (kinds, edges)
}

/// The counts of `docs`, from their nodes directly.
fn direct(docs: &[(GrammarDialect, &TapeDoc)]) -> Counts {
    let mut kinds = BTreeMap::<i64, (i64, i64)>::new();
    let mut edges = BTreeMap::<(i64, i64, i64), i64>::new();
    for (lang, doc) in docs {
        let base = i64::from(language::tape_tag(*lang).unwrap()) * i64::from(LANG_STRIDE);
        for pre in 0..doc.transducer_len() as u32 {
            let kind = base + i64::from(kind_code(doc, pre));
            let entry = kinds.entry(kind).or_default();
            entry.0 += 1;
            entry.1 += doc.transducer_children(pre).map(|child| 1 + doc.transducer_descendants(child).len() as i64).sum::<i64>();
            if let Some(parent) = doc.transducer_parent(pre) {
                let key = (base + i64::from(kind_code(doc, parent)), kind, i64::from(doc.transducer_field(pre)));
                *edges.entry(key).or_default() += 1;
            }
        }
    }
    (
        kinds.into_iter().map(|(kind, (n, size))| (kind, n, size)).collect(),
        edges.into_iter().map(|((p, c, f), n)| (p, c, f, n)).collect(),
    )
}

const SOURCES: [(&str, GrammarDialect); 5] = [
    ("function x(a) { return a + 1; }\nx(2);", GrammarDialect::TypeScript),
    ("const o = { k: f(1), g: [2, 3] };", GrammarDialect::TypeScript),
    ("def x(a):\n    return a + 1\nx(2)\n", GrammarDialect::Python),
    ("class C:\n    def m(self, b=4):\n        return self.b\n", GrammarDialect::Python),
    ("function f( {\n debugger;\n", GrammarDialect::TypeScript),
];

#[test]
fn kind_sizes_and_edges_equal_a_direct_count_and_follow_removal() {
    let store = open(&temp());
    let published = SOURCES.map(|(source, lang)| (put(&store, source, lang), lang));
    let docs = published.iter().map(|((_, doc), lang)| (*lang, doc)).collect::<Vec<_>>();
    let all = direct(&docs);
    assert!(all.1.iter().any(|edge| edge.2 != 0), "some edge hangs under a field");
    assert_eq!(counts(&store), all);

    // A collected tape counts down through the cascade; a republished one replaces itself.
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
    assert_eq!(counts(&store), (vec![], vec![]));
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
    assert_eq!(counts(&store), (vec![], vec![]));
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

const FACTS: [FactTable<'static>; 1] = [FactTable { name: "calls", doc: "d", local: &["n"], values: &["callee", "arity"] }];

fn fact_counts(db: &Connection) -> (Vec<(String, String, i64, i64, i64, i64)>, Vec<(String, String, String, i64, i64, i64)>) {
    let columns = db
        .prepare("SELECT tbl,col,rows,ndv,sq,sqd FROM fact_stats ORDER BY 1,2")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let values = db
        .prepare("SELECT tbl,col,CAST(val AS TEXT),n,docs,sqd FROM fact_value_stats ORDER BY 1,2,3")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    (columns, values)
}

#[test]
fn fact_statistics_merge_incrementally_to_the_one_shot_build() {
    let store = open(&temp());
    store
        .candidate_transaction(true, |db| {
            db.execute_batch("CREATE TEMP TABLE calls(d INTEGER, n INTEGER, callee TEXT, arity INTEGER)")?;
            let mut insert = db.prepare("INSERT INTO temp.calls VALUES (?1, ?2, ?3, ?4)")?;
            for d in 1..=9_i64 {
                for n in 0..(d % 4 + 1) {
                    insert.execute(params![d, n, ["f", "g", "h"][((d + n) % 3) as usize], (d * n) % 2])?;
                    insert.execute(params![d, n, "f", 0])?;
                }
            }
            let docs = (1..=9).collect::<Vec<i64>>();
            merge_facts(db, &FACTS, &docs, 1)?;
            let one = fact_counts(db);
            // T's definitions, directly: arity's corpus counts and their squares.
            let (rows, ndv, sq): (i64, i64, i64) = db.query_row(
                "SELECT sum(c), count(*), sum(c*c) FROM (SELECT count(*) c FROM temp.calls GROUP BY arity)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let arity = one.0.iter().find(|c| c.1 == "arity").unwrap();
            assert_eq!((arity.2, arity.3, arity.4), (rows, ndv, sq));
            let n = one.0.iter().find(|c| c.1 == "n").unwrap();
            assert_eq!(n.4, n.5, "a document-local column's squares are per document");

            db.execute_batch("DELETE FROM fact_stats; DELETE FROM fact_value_stats")?;
            for part in [&docs[3..6], &docs[..3], &docs[6..]] {
                merge_facts(db, &FACTS, part, 1)?;
            }
            merge_facts(db, &FACTS, &docs[3..5], -1)?;
            assert_ne!(fact_counts(db), one);
            merge_facts(db, &FACTS, &docs[3..5], 1)?;
            assert_eq!(fact_counts(db), one);
            merge_facts(db, &FACTS, &docs, -1)?;
            assert_eq!(fact_counts(db), (vec![], vec![]));
            Ok(())
        })
        .unwrap();
}
