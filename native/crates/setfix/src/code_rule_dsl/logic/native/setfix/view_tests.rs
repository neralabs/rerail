//! A bound input is read in place by the name its statement writes (O-pse-alone-strict).
use super::*;

/// A relation named like a SQLite keyword is quoted in every statement (`sql::ident`, O106): `plan` is read as
/// `main."plan"`. `view` found no bound input of that name and read it through SQL, which strict mode refuses inside
/// the waves (0104): the data-layer's plan-scope rules alone stopped at `SELECT c0 FROM main."plan"`.
#[test]
fn a_bound_input_named_like_a_sql_keyword_is_read_in_place() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(vec![4, 7])]);
    modules.bind(&db, "plan", source).unwrap();
    let table = format!("main.{}", crate::code_rule_dsl::logic::sql::ident("plan"));
    assert_eq!(table, "main.\"plan\"");
    let all = view(&db, &table, "c0").expect("read in place");
    assert_eq!(all.src.len(), 2);
    let some = view(&db, &format!("{table} WHERE c0 = 7"), "c0").expect("read in place, filtered");
    assert_eq!(some.ids(), Some(&[1u32][..]));
}

/// `unquote` reads back what `ident` wrote, for a keyword, a name that is no plain identifier, and a plain one.
#[test]
fn unquote_reads_back_an_identifier() {
    use crate::code_rule_dsl::logic::sql::{ident, unquote};
    for name in ["plan", "values", "a\"b", "has space", "p47_plan"] {
        assert_eq!(unquote(&ident(name)), name, "{}", ident(name));
    }
}

/// A relation the file-local part derives and hands on under a base input's name is bound as an export, its columns
/// positional (`c0`, `c1`), while statements name its columns as the input's (`plan::sql_col`): `value` (decision
/// 0292 §3: `@rr_node_value` read in the chunk) was read as `SELECT n, v FROM main.value`, and `view` found no column
/// `n` in the bound export and read it through SQL, which strict mode refuses inside the waves (0104).
#[test]
fn an_export_named_like_an_input_is_read_in_place_by_the_inputs_column_names() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(vec![4, 7]), Some(vec![40, 70])]);
    modules.bind(&db, "value", source).unwrap();
    let (n, v) = (crate::code_rule_dsl::logic::sql::select::sql_col("value", 0), crate::code_rule_dsl::logic::sql::select::sql_col("value", 1));
    assert_eq!((n.as_str(), v.as_str()), ("n", "v"), "the statement names `value`'s columns as the input's");
    let all = view(&db, "main.value", &format!("{n}, {v}")).expect("read in place");
    assert_eq!(all.cols, [0, 1]);
    let some = view(&db, &format!("main.value WHERE {n} = 7"), &v).expect("read in place, filtered");
    assert_eq!((some.cols.as_slice(), some.ids()), (&[1usize][..], Some(&[1u32][..])));
}

/// Its table is named by the base names when first bound (`vtab::named`), so a later binding of the same shape is
/// compared by them too: the kept inputs' `value` (`c0`, `c1`) bound again over the chunks' failed "native binding
/// schema changed" on an edit. A source of another shape is still refused.
#[test]
fn an_export_named_like_an_input_is_bound_again_with_the_same_columns() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = |rows: Vec<i64>| crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(rows.clone()), Some(rows)]);
    modules.bind(&db, "value", source(vec![4])).unwrap();
    modules.bind(&db, "value", source(vec![4, 7])).expect("bound again, the same columns");
    assert_eq!(crate::code_rule_dsl::facts::vtab::bound_source(&db, "value").unwrap().len(), 2);
    let other = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(vec![1])]);
    assert!(modules.bind(&db, "value", other).is_err(), "another shape is refused");
}

/// A view filtered by a constant finds its rows on its first scan, not when it is opened: its probes check the filter
/// themselves (`View::find`), so a batch's relation (decision 0494) is not made whole to open a rule that only
/// probes it. ep2's edit: `parsed(s, "annotation", r)` made every document's rows to open its rules.
#[test]
fn a_filtered_view_finds_its_rows_on_its_first_scan() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(vec![4, 7, 7]), Some(vec![1, 2, 3])]);
    modules.bind(&db, "pairs", source).unwrap();
    let some = view(&db, "main.pairs WHERE c0 = 7", "c1").expect("read in place, filtered");
    assert!(some.ids.as_ref().is_some_and(|ids| ids.get().is_none()), "no row is found to open it");
    assert!(!some.known_empty(), "nor is it known empty");
    assert_eq!(some.find(0, 3), vec![2], "a probe checks the filter itself");
    assert_eq!(some.find(0, 1), Vec::<u32>::new(), "row 0 fails the filter");
    assert!(some.ids.as_ref().is_some_and(|ids| ids.get().is_none()), "a probe finds no view row");
    assert_eq!((some.ids(), some.len()), (Some(&[1u32, 2][..]), 2), "the scan's rows");
}

/// A held relation that views every row of a source (an input, a changed input's version, a kept relation read in
/// place) is estimated as its source estimates it: an index's choice of its lead column (`InRel::lead`) measured
/// every column of every row for one column's distinct values (ep2's edit: 15% of its CPU).
#[test]
fn a_whole_views_estimate_is_its_sources() {
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["a".into(), "b".into()], vec![Some(vec![1, 1, 2, 2, 3, 3]), Some(vec![5, 6, 7, 8, 9, 10])]);
    let held = InRel::over_source(Arc::clone(&source), vec![b'i', b'i']);
    let viewed = held.source(vec!["a".into(), "b".into()]);
    assert_eq!(viewed.estimated_matches(0, false), source.estimated_matches(0, false), "the source's estimate");
    assert_eq!(viewed.estimated_matches(1, false), source.estimated_matches(1, false));
    assert!(held.measured.get().is_none(), "no statistic of it is measured for an estimate");
    // Statistics given (a kept relation's, measured by an earlier run) answer it, as they answer `statistics`.
    held.give_measured(&[("a".into(), 6, 3, 12, 12), ("b".into(), 6, 6, 6, 6)]);
    assert_eq!((viewed.estimated_matches(0, false), viewed.estimated_matches(1, false)), (2, 1));
    assert_eq!(viewed.statistics().map(|s| s[0].2), Some(3), "the statistics given, not measured");
}

/// The memory profile counts the rows the run holds (`held_by_name`): a view over a source holds none of the run's,
/// and its source is not asked its length, which a source made on lookup answers by making every row (the sampler's
/// thread made `within`'s rows, 87 MB on the M3's 300 files, for this count alone).
#[test]
fn the_memory_profile_asks_no_view_its_length() {
    use crate::code_rule_dsl::facts::vtab::Source;
    struct Unasked;
    impl Source for Unasked {
        fn columns(&self) -> Vec<String> {
            vec!["n".into(), "a".into()]
        }
        fn len(&self) -> usize {
            panic!("a view's length asked for the memory profile")
        }
        fn estimated_matches(&self, _column: usize, _batch: bool) -> usize {
            1
        }
        fn indexed(&self, _column: usize) -> bool {
            true
        }
        fn lookup(&self, _column: usize, _values: &[i64]) -> Option<Vec<usize>> {
            Some(Vec::new())
        }
        fn value(&self, _row: usize, _column: usize) -> i64 {
            0
        }
        fn made_on_lookup(&self) -> bool {
            true
        }
    }
    let rel = InRel::over_source(Arc::new(Unasked), vec![b'i', b'i']);
    assert_eq!(rows_held(&rel), 0, "a view holds no rows of the run's");
}
