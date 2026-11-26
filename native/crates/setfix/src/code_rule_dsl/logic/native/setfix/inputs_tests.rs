//! Decision 0254: connections on one database (a run's chunks evaluating at once) bind sources of their own, so the
//! inputs the engine loads are each connection's.
use rerail_blocks::vector::Vector;
use super::spec::hex;
use super::*;

/// A run begins by forgetting its connection's inputs (`run_native_inputs`) and loads them as its strata read them.
/// Keyed by the database's path, a chunk that began while another ran dropped and replaced that chunk's loaded input,
/// and the other's next stratum read the new chunk's rows (most of a run's findings were lost).
#[test]
fn a_connection_reads_its_own_input_while_another_on_its_database_loads_one() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let open = |rows: Vector<i64>| {
        let db = Connection::open(&path).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
        modules.bind(&db, "e", crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(rows)])).unwrap();
        (db, modules)
    };
    let (a, _a) = open(rerail_blocks::vector![1, 2]);
    let (b, _b) = open(rerail_blocks::vector![10]);
    let spec = format!("b:{}:{}:i", hex("main.e"), hex("c0"));
    let rows = |db: &Connection| {
        let shared = symbols(db).unwrap();
        let mut syms = Interner::for_loading(&shared);
        input(db, &spec, &mut syms).unwrap().len()
    };
    forget_inputs(&a);
    assert_eq!(rows(&a), 2);
    // b's run begins while a's goes on.
    forget_inputs(&b);
    assert_eq!(rows(&b), 1);
    assert_eq!(rows(&a), 2, "a's next stratum reads a's rows");
}

/// A relation held for the runs that read it (`columns_source`) gives each column's least and greatest value
/// (`col_bounds`), taken as its rows are written: a stratum's head key copied from it packs in the bits they span,
/// not 64. A column shorter than the rows reads 0 past its end, a column not held reads 0, and no rows bound nothing.
#[test]
fn a_held_relations_column_bounds_are_its_rows() {
    let held = crate::code_rule_dsl::logic::native::component::columns_source(
        vec!["a".into(), "b".into(), "c".into()],
        vec![Some(rerail_blocks::vector![5, -3, 9]), None, Some(rerail_blocks::vector![7])],
    );
    assert_eq!((0..3).map(|c| held.col_bounds(c)).collect::<Vec<_>>(), vec![Some([-3, 9]), Some([0, 0]), Some([0, 7])]);
    for c in 0..3 {
        let [lo, hi] = held.col_bounds(c).unwrap();
        assert!((0..held.len()).all(|r| (lo..=hi).contains(&held.value(r, c))));
    }
    let empty = crate::code_rule_dsl::logic::native::component::columns_source(vec!["a".into()], vec![Some(rerail_blocks::vector::Vector::new())]);
    assert_eq!(empty.col_bounds(0), Some([i64::MAX, i64::MIN]));
}
