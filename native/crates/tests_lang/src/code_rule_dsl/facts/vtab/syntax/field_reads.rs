//! `field`'s reads by its field alone (`field($p, #f, $c)`, neither node bound): a walk of each document's tape.
use super::*;
use super::fixtures::documents;
use crate::code_rule_dsl::facts::vtab::ProbeCursor;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn temp(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rerail-field-reads-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// For every field the documents hold, and one they do not: `lookup` by the field and a cursor's read give exactly the
/// rows whose field column is that field (a scan of every row), in order, and the read's values are each row's.
#[test]
fn a_fields_rows_by_field_alone_are_the_rows_a_scan_finds() {
    let store = ContentStore::open(&temp("by-field")).unwrap();
    let (_, field) = sources(&store, documents().into_iter().take(4).collect()).unwrap().into_iter().find(|(name, _)| *name == "field").unwrap();
    let n = field.len();
    assert!(n > 100, "the documents have fielded nodes: {n}");
    let mut by: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for r in 0..n {
        by.entry(field.value(r, 1)).or_default().push(r);
    }
    assert!(by.len() > 5, "fields: {}", by.len());
    let absent = by.keys().last().unwrap() + 1;
    by.insert(absent, Vec::new());
    let columns: Vec<usize> = (0..field.columns().len()).collect();
    let mut cursor = ProbeCursor::default();
    for (&f, want) in &by {
        let got = field.lookup(1, &[f]).expect("field looks up by its field");
        assert_eq!(got.iter().copied().collect::<Vec<_>>(), *want, "field {f}: lookup");
        let mut read = Vec::new();
        assert!(field.lookup_read_at(1, f, &columns, &mut cursor, &mut |r, vals| read.push((r, vals.to_vec()))), "field {f}: read as found");
        let rows: Vec<(usize, Vec<i64>)> = want.iter().map(|&r| (r, columns.iter().map(|&c| field.value(r, c)).collect())).collect();
        assert_eq!(read, rows, "field {f}: a cursor's read");
    }
}
