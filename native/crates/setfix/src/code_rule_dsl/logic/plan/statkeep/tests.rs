use super::RelStats;
use crate::code_rule_dsl::facts::vtab::syntax;
use crate::code_rule_dsl::logic::native::component::columns_source;

/// Document `d`'s rows of `r(n, kind, name)`: `n` its nodes, `kind` few-valued, `name` many values shared
/// across documents (Zipf-like), so each column takes another of the keeper's paths.
fn doc(d: i64, rows: i64, salt: i64) -> rerail_blocks::vector::Vector<i64> {
    (0..rows).flat_map(|i| [d << 32 | i, (i * 7 + salt) % 9, ((i * i + salt) % 997) / (1 + i % 5)]).collect()
}

fn measured(docs: &[rerail_blocks::vector::Vector<i64>]) -> Vec<(String, i64, i64, i64, i64)> {
    let mut cols = vec![rerail_blocks::vector::Vector::new(); 3];
    for d in docs {
        for row in d.as_chunks::<3>().0 {
            for (c, v) in row.iter().enumerate() {
                cols[c].push(*v);
            }
        }
    }
    syntax::measured(columns_source(vec!["n".into(), "kind".into(), "name".into()], cols.into_iter().map(Some).collect()).as_ref())
}

/// A base relation's documents counted where their rows lie (`documents_read`, decision 0458), at some of its
/// columns, are counted as their flat rows are; an edit's patch (the old document out, the new in) is a count
/// from scratch of the documents now.
#[test]
fn documents_read_where_they_lie_count_as_their_flat_rows() {
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=20).map(|d| doc(d, 30 + d * 7, d % 3)).collect();
    let all: rerail_blocks::vector::Vector<i64> = docs.iter().flat_map(|d| d.iter().copied()).collect();
    let n = all.len() / 3;
    fn read(rows: &[i64]) -> impl Fn(usize, usize) -> i64 + '_ {
        move |r, c| rows[r * 3 + c]
    }
    let from = [0usize, 2];
    let names = || vec!["n".to_owned(), "name".to_owned()];
    let (mut flat, mut placed) = (RelStats::new(names()), RelStats::new(names()));
    flat.documents_at(&all, 3, &from, 1);
    placed.documents_read(n, 3, &read(&all), &from, 1);
    assert_eq!(placed, flat);
    let edited = doc(5, 90, 1);
    placed.documents_read(docs[4].len() / 3, 3, &read(&docs[4]), &from, -1);
    placed.documents_read(edited.len() / 3, 3, &read(&edited), &from, 1);
    let mut now = docs.clone();
    now[4] = edited;
    let now = now.concat();
    let mut fresh = RelStats::new(names());
    fresh.documents_read(now.len() / 3, 3, &read(&now), &from, 1);
    assert_eq!(placed, fresh);
    // Read by the source's walk, a column at a time (`documents_scanned`): the same counts, in and out.
    let source = |rows: &[i64]| crate::code_rule_dsl::facts::vtab::Relation::from_rows(&["n", "k", "name"], &mut rows.as_chunks::<3>().0.iter().map(|r| r.iter().copied().collect()));
    let mut scanned = RelStats::new(names());
    scanned.documents_scanned(&source(&all), &from, 1);
    assert_eq!(scanned, flat);
    scanned.documents_scanned(&source(&docs[4]), &from, -1);
    scanned.documents_scanned(&source(&doc(5, 90, 1)), &from, 1);
    assert_eq!(scanned, fresh);
    // Counted in windows that end where a document does (a large relation's, bounding the count's lists): the
    // same counts, at a window shorter than a document and at one of several.
    for window in [7, 100, 1000] {
        let mut windowed = RelStats::new(names());
        windowed.documents_scanned_in(&source(&all), &from, 1, window);
        assert_eq!(windowed, flat, "window {window}");
    }
}

/// A column's watched values (the constants the rules test) are counted exactly as documents come and go and as
/// rows are patched, past the column's exact map, and read back from the blob: an edit's run takes them as kept.
#[test]
fn watched_values_are_counted_exactly_past_the_exact_map() {
    let names = vec!["n".to_owned(), "kind".into(), "name".into()];
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=40).map(|d| doc(d, 50 + d * 13, d % 3)).collect();
    let mut s = RelStats::new(names);
    s.documents(&docs.iter().flat_map(|d| d.iter().copied()).collect::<Vec<_>>(), 3, 1);
    assert!(s.exact(2).is_none(), "the name column is many-valued");
    let watch = [0i64, 5, 12, 99_999];
    let count = |rows: &[rerail_blocks::vector::Vector<i64>], v: i64| rows.iter().flat_map(|d| d.as_chunks::<3>().0).filter(|r| r[2] == v).count() as i64;
    s.watch(2, watch.iter().map(|&v| (v, count(&docs, v))));
    // Documents out and in, and a delta of rows that are not whole documents.
    s.documents(&docs[7], 3, -1);
    let new = doc(41, 400, 2);
    s.documents(&new, 3, 1);
    let added = doc(42, 30, 1);
    let removed: rerail_blocks::vector::Vector<i64> = docs[3][..3 * 20].iter().copied().collect();
    s.delta(&added, &removed, 3);
    let mut now = docs.clone();
    now[7].clear();
    now[3].drain(..3 * 20);
    now.push(new);
    now.push(added);
    let mut bytes = Vec::new();
    s.encode(&mut bytes);
    let back = RelStats::decode(&bytes, &mut 0).expect("reads back");
    for v in watch {
        assert_eq!(s.watched(2, v), Some(count(&now, v)), "value {v}");
        assert_eq!(back.watched(2, v), Some(count(&now, v)), "value {v}, read back");
    }
    assert_eq!(s.watched(2, 7), None, "a value not watched");
}

/// Each chunk's documents counted where its rows lie (`documents_scanned_range`, a range of whole documents of a
/// source holding others too) and the chunks' counts merged (`merge`) are the count of all their documents: the
/// statistics the run's stats phase takes from its chunks (decision 0458) are those it counted off the batch.
#[test]
fn chunks_counts_merged_are_the_count_of_all_their_documents() {
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=24).map(|d| doc(d, 20 + d * 5, d % 4)).collect();
    let names = || vec!["n".to_owned(), "kind".to_owned(), "name".to_owned()];
    let from = [0usize, 1, 2];
    let source = |rows: &[i64]| crate::code_rule_dsl::facts::vtab::Relation::from_rows(&["n", "kind", "name"], &mut rows.as_chunks::<3>().0.iter().map(|r| r.iter().copied().collect()));
    let mut whole = RelStats::new(names());
    whole.documents_scanned(&source(&docs.concat()), &from, 1);
    // Three chunks, each of every third document, its rows counted where they lie in a source of all of them.
    let all = source(&docs.concat());
    let starts: Vec<usize> = docs.iter().scan(0, |at, d| Some(std::mem::replace(at, *at + d.len() / 3))).collect();
    let mut merged: Option<RelStats> = None;
    for chunk in 0..3 {
        let mut k = RelStats::new(names());
        for (i, d) in docs.iter().enumerate().filter(|(i, _)| i % 3 == chunk) {
            k.documents_scanned_range(&all, starts[i], starts[i] + d.len() / 3, &from, 1);
        }
        match &mut merged {
            None => merged = Some(k),
            Some(m) => assert!(m.merge(&k)),
        }
    }
    let merged = merged.expect("three chunks");
    assert_eq!(merged.statistics(&|_| true), whole.statistics(&|_| true));
    assert_eq!(merged.rows(), whole.rows());
    // Columns that differ are not merged.
    let mut other = RelStats::new(vec!["n".to_owned()]);
    assert!(!other.merge(&merged));
}


/// Kept by delta, the statistics are those of the documents counted in, whatever came and went before: node
/// and few-valued columns exactly as measured, a many-valued one within the planner's margin. The blob reads
/// back the same.
#[test]
fn statistics_kept_by_document_are_those_of_the_documents_counted_in() {
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=40).map(|d| doc(d, 50 + d * 13, d % 3)).collect();
    let mut kept = RelStats::new(vec!["n".into(), "kind".into(), "name".into()]);
    for d in &docs {
        kept.documents(d, 3, 1);
    }
    // Edits: a document changes, one leaves, one comes back changed again.
    let edited = doc(7, 400, 2);
    kept.documents(&docs[6], 3, -1);
    kept.documents(&edited, 3, 1);
    kept.documents(&docs[11], 3, -1);
    let mut now = docs.clone();
    now[6] = edited;
    now.remove(11);
    let (got, want) = (kept.statistics(&|_| true), measured(&now));
    assert_eq!(got[0], want[0], "a node column is exact");
    assert_eq!((got[1].1, got[1].2, got[1].3), (want[1].1, want[1].2, want[1].3), "a few-valued column is exact");
    let ratio = |a: i64, b: i64| a.max(b) as f64 / a.min(b).max(1) as f64;
    assert!(ratio(got[2].2, want[2].2) < 2.0 && ratio(got[2].3, want[2].3) < 2.0, "{:?} against {:?}", got[2], want[2]);
    assert_eq!(got[2].4, want[2].4, "sqd is exact");
    // Counted from scratch, the same counters: history does not matter.
    let mut fresh = RelStats::new(kept.columns().to_vec());
    for d in &now {
        fresh.documents(d, 3, 1);
    }
    assert_eq!(fresh, kept);
    let mut blob = Vec::new();
    kept.encode(&mut blob);
    let mut at = 0;
    assert_eq!(RelStats::decode(&blob, &mut at), Some(kept));
    assert_eq!(at, blob.len());
}

/// Patched by a delta of rows (not whole documents), the counts stay those of the rows: exact while few-valued.
/// A relation counted whole and then patched by rows that are not whole documents (a maintained relation's
/// delta) keeps its within-document sum by its pairs' sketch, within the planner's margin, where an unpaired one
/// reads it as `sq`; its rows and its few-valued column stay exact.
#[test]
fn a_paired_relation_patched_by_rows_keeps_its_sum_within_documents() {
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=40).map(|d| doc(d, 50 + d * 13, d % 3)).collect();
    let names = vec!["n".to_owned(), "kind".into(), "name".into()];
    let (mut paired, mut plain) = (RelStats::paired(names.clone()), RelStats::new(names));
    let all: rerail_blocks::vector::Vector<i64> = docs.iter().flat_map(|d| d.iter().copied()).collect();
    paired.documents(&all, 3, 1);
    plain.documents(&all, 3, 1);
    // Rows out of some documents, and others into them: not whole documents.
    let removed: rerail_blocks::vector::Vector<i64> = docs[3][..30].iter().chain(&docs[20][..60]).copied().collect();
    let added: rerail_blocks::vector::Vector<i64> = doc(9, 70, 1)[3 * 50..].iter().chain(&doc(41, 30, 0)).copied().collect();
    paired.delta(&added, &removed, 3);
    plain.delta(&added, &removed, 3);
    let mut now = docs.clone();
    now[3].drain(..30);
    now[20].drain(..60);
    now.push(added);
    let (got, flat, want) = (paired.statistics(&|_| true), plain.statistics(&|_| true), measured(&now));
    let ratio = |a: i64, b: i64| a.max(b) as f64 / a.min(b).max(1) as f64;
    for c in 0..3 {
        assert_eq!(got[c].1, want[c].1, "rows are exact");
        assert!(ratio(got[c].4, want[c].4) < 1.5, "column {c}: sqd {:?} against {:?}", got[c], want[c]);
        assert!(ratio(got[c].2, want[c].2) < 2.0 && ratio(got[c].3, want[c].3) < 2.0, "column {c}: {:?} against {:?}", got[c], want[c]);
    }
    assert_eq!((got[1].2, got[1].3), (want[1].2, want[1].3), "a few-valued column is exact");
    assert!(flat[1].4 == flat[1].3 && flat[1].4 > want[1].4, "unpaired, sqd reads as sq");
    let mut blob = Vec::new();
    paired.encode(&mut blob);
    assert_eq!(RelStats::decode(&blob, &mut 0), Some(paired));
}

#[test]
fn statistics_patched_by_a_delta_of_rows_count_the_rows() {
    let rows: rerail_blocks::vector::Vector<i64> = (0..300).flat_map(|i| [i % 10, i % 4]).collect();
    let mut kept = RelStats::new(vec!["a".into(), "b".into()]);
    kept.delta(&rows, &[], 2);
    let (plus, minus): (rerail_blocks::vector::Vector<i64>, rerail_blocks::vector::Vector<i64>) = ((0..30).flat_map(|i| [i % 10, 9]).collect(), rerail_blocks::vector::Vector::from(&rows[..40][..]));
    kept.delta(&plus, &minus, 2);
    let mut now: rerail_blocks::vector::Vector<i64> = rerail_blocks::vector::Vector::from(&rows[40..][..]);
    now.extend(&plus);
    let mut fresh = RelStats::new(kept.columns().to_vec());
    fresh.delta(&now, &[], 2);
    assert_eq!(fresh, kept);
    let got = kept.statistics(&|_| true);
    assert_eq!(got[1].1, 310);
    assert_eq!(got[1].2, 5, "b holds 0..4 and 9");
}

/// The one-pass count gives the counters the four passes it replaced gave: rows naming their document by its
/// key or by its nodes, documents whose rows come apart, values crossing documents, with and without `at`.
#[test]
fn one_pass_counts_as_the_four_passes_did() {
    use rustc_hash::FxHashMap;
    // The old count, kept here as the reference.
    fn old(stats: &mut RelStats, flat: &[i64], width: usize, sign: i64, at: Option<Option<usize>>) {
        let of = |row: &[i64]| match at {
            None => (super::doc32(row[0]), super::document_of(row[0])),
            Some(at) => {
                let d = at.map_or(-1, |c| row[c] >> 32);
                (d, d)
            }
        };
        let names = |v: i64, row: &[i64]| match at {
            None => super::names_doc(v, row[0]),
            Some(at) => at.is_some_and(|c| v >> 32 == row[c] >> 32),
        };
        stats.rows += sign * (flat.len() / width) as i64;
        for (c, col) in stats.cols_mut().iter_mut().enumerate().take(width) {
            let (mut counts, mut values, mut crossing, mut per_doc) = (FxHashMap::<(i64, i64), i64>::default(), FxHashMap::<i64, i64>::default(), FxHashMap::<i64, bool>::default(), FxHashMap::<(i64, i64), ()>::default());
            for row in flat.chunks_exact(width) {
                let (v, (d32, dof)) = (row[c], of(row));
                *counts.entry((d32, v)).or_default() += 1;
                *values.entry(v).or_default() += 1;
                *crossing.entry(dof).or_default() |= !names(v, row);
                per_doc.insert((dof, v), ());
            }
            // The document-key rows' group (`doc32` 0) by value, apart (`Column::zero`).
            col.sqd += sign * counts.iter().filter(|((d, _), _)| *d != 0).map(|(_, k)| k * k).sum::<i64>();
            let zeros: Vec<(i64, i64)> = counts.iter().filter(|((d, _), _)| *d == 0).map(|(&(_, v), &k)| (v, k)).collect();
            RelStats::zeros(col, &zeros, sign);
            col.local_ndv += sign * per_doc.len() as i64;
            col.crossing += sign * crossing.values().filter(|x| **x).count() as i64;
            // A paired column's sketch of (document, value) counts: rows naming a document by its nodes by
            // that document, rows whose first column is a document key summed by value.
            if let Some(p) = &mut col.pairs {
                let (mut large, mut small) = (FxHashMap::<(i64, i64), i64>::default(), FxHashMap::<i64, i64>::default());
                for row in flat.chunks_exact(width) {
                    let (v, (d32, dof)) = (row[c], of(row));
                    if d32 == 0 { *small.entry(v).or_default() += 1 } else { *large.entry((dof, v)).or_default() += 1 }
                }
                for ((d, v), k) in large {
                    p.add(super::pair(d, v), sign * k);
                }
                for (v, k) in small {
                    p.add(super::pair(0, v), sign * k);
                }
            }
            RelStats::values(col, &values.into_iter().collect::<Vec<_>>(), sign);
        }
    }
    let mut flat = rerail_blocks::vector::Vector::new();
    for i in 0..600i64 {
        let d = 1 + i % 7;
        let first = if i % 5 == 0 { d } else { d << 32 | i };
        let other = if i % 11 == 0 { (d + 1) << 32 | i } else { d << 32 | (i / 3) };
        flat.extend([first, i % 4, other]);
    }
    // One document's rows alone, in order (no sort), and its rows reversed (sorted).
    let one: rerail_blocks::vector::Vector<i64> = flat.as_chunks::<3>().0.iter().filter(|r| r[0] == 3 || r[0] >> 32 == 3).flatten().copied().collect();
    let reversed: rerail_blocks::vector::Vector<i64> = one.as_chunks::<3>().0.iter().rev().flatten().copied().collect();
    // Many values in one column: past the exact map's bound.
    let wide: rerail_blocks::vector::Vector<i64> = (0..700i64).flat_map(|i| [5 << 32 | i, i % 3, i]).collect();
    let names = || vec!["x".to_owned(), "k".into(), "y".into()];
    for new in [RelStats::new as fn(Vec<String>) -> RelStats, RelStats::paired] {
        for rows in [&flat, &one, &reversed, &wide] {
            for at in [None, Some(None), Some(Some(0)), Some(Some(2))] {
                let (mut a, mut b) = (new(names()), new(names()));
                a.rows_of(rows, 3, 1, at, None);
                old(&mut b, rows, 3, 1, at);
                assert_eq!(a, b, "at {at:?}");
                let half = &rows[..rows.len() / 6 * 3];
                a.rows_of(half, 3, -1, at, None);
                old(&mut b, half, 3, -1, at);
                assert_eq!(a, b, "at {at:?}, counted out");
            }
        }
    }
}

/// Statistics read as kept (`RelStats::kept`, t33 K4) are decoded statistics' in every answer: their summary answers
/// `statistics` undecoded; the exact maps and watched values given apart (a symbol column's, as the keeper keeps them)
/// answer `exact` and `watched` undecoded, and are taken out again for writing as they were given; written back
/// undecoded, they are the bytes they were read from; patched by a document, they are decoded statistics patched alike.
#[test]
fn statistics_read_as_kept_answer_and_patch_as_decoded_ones() {
    let names = vec!["n".to_owned(), "kind".into(), "name".into()];
    let docs: Vec<rerail_blocks::vector::Vector<i64>> = (1..=30).map(|d| doc(d, 40 + d * 9, d % 3)).collect();
    let mut s = RelStats::new(names);
    s.documents(&docs.iter().flat_map(|d| d.iter().copied()).collect::<rerail_blocks::vector::Vector<i64>>(), 3, 1);
    let count = |rows: &[rerail_blocks::vector::Vector<i64>], v: i64| rows.iter().flat_map(|d| d.as_chunks::<3>().0).filter(|r| r[2] == v).count() as i64;
    s.watch(2, [3, 8].into_iter().map(|v| (v, count(&docs, v))));
    let summary = s.summary();
    // As the keeper writes them: the kind column's exact map and the name column's watched values taken out.
    let mut written = s.clone();
    let exact = written.take_exact(1).expect("kind is few-valued");
    let watched = written.take_watched(2);
    let mut bytes = Vec::new();
    written.encode(&mut bytes);
    let read = || {
        let mut k = RelStats::kept(bytes.clone().into(), Some(summary.clone())).expect("reads as kept");
        k.put_exact(1, exact.clone());
        k.watch(2, watched.clone());
        k
    };
    let k = read();
    assert_eq!(k.statistics(&|_| true), s.statistics(&|_| true), "the summary answers as the decoded statistics do");
    assert_eq!(k.statistics(&|c| c != "name"), s.statistics(&|c| c != "name"), "and with a column not held");
    let mut a: Vec<(i64, i64)> = k.exact(1).expect("given").collect();
    let mut b: Vec<(i64, i64)> = s.exact(1).expect("counted").collect();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "the exact map given apart");
    assert_eq!(k.exact(2).is_some(), s.exact(2).is_some(), "the bytes' own exact map, or none");
    for v in [3, 8, 5] {
        assert_eq!(k.watched(2, v), s.watched(2, v), "watched value {v}");
    }
    // Written back undecoded, its maps taken out as the keeper takes them: the bytes it was read from.
    let mut again = read();
    assert_eq!(again.take_exact(1), Some(exact.clone()));
    assert_eq!(again.take_watched(2), watched);
    let mut rewritten = Vec::new();
    again.encode(&mut rewritten);
    assert_eq!(rewritten, bytes, "written back as read");
    // Patched: decoded, its maps applied, and patched as decoded statistics are.
    let (mut kept, mut decoded) = (read(), s.clone());
    for x in [&mut kept, &mut decoded] {
        x.documents(&docs[4], 3, -1);
        x.documents(&doc(31, 70, 2), 3, 1);
    }
    assert_eq!(kept, decoded, "patched alike");
    assert_eq!(kept.statistics(&|_| true), decoded.statistics(&|_| true));
}
