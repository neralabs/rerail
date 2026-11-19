//! Decision 0254: connections on one database (a run's chunks evaluating at once) bind sources of their own, so the
//! inputs the engine loads are each connection's.
use super::spec::hex;
use super::*;

/// A run begins by forgetting its connection's inputs (`run_native_inputs`) and loads them as its strata read them.
/// Keyed by the database's path, a chunk that began while another ran dropped and replaced that chunk's loaded input,
/// and the other's next stratum read the new chunk's rows (most of a run's findings were lost).
#[test]
fn a_connection_reads_its_own_input_while_another_on_its_database_loads_one() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let open = |rows: Vec<i64>| {
        let db = Connection::open(&path).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
        modules.bind(&db, "e", crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(rows)])).unwrap();
        (db, modules)
    };
    let (a, _a) = open(vec![1, 2]);
    let (b, _b) = open(vec![10]);
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
