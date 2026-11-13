use super::*;
use crate::code_rule_dsl::logic::native::component;

#[test]
fn same_relation_reuses_keys_and_lifetime_does_not_cross_runs() {
    let db = Connection::open_in_memory().unwrap();
    let source = component::columns_source(
        vec!["c0".into()],
        vec![Some(vec![1, 1, 2, (2_i64 << 32) | 1])],
    );
    let mut demands = Demands::default();
    let first = demands.read(&db, "keys", "c0", Some(source)).unwrap();
    assert_eq!(first.as_slice(), &[1, 2, (2_i64 << 32) | 1]);
    // There is no SQL table: a second consumer must reuse the same snapshot.
    let second = demands.read(&db, "keys", "c0", None).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let narrower = component::columns_source(vec!["c0".into()], vec![Some(vec![2])]);
    assert_eq!(
        demands
            .read(&db, "narrower", "c0", Some(narrower))
            .unwrap()
            .as_slice(),
        &[2]
    );
    demands.forget("keys");
    let empty = component::columns_source(vec!["c0".into()], vec![Some(vec![])]);
    assert!(
        demands
            .read(&db, "keys", "c0", Some(empty))
            .unwrap()
            .is_empty()
    );
    let mut next_run = Demands::default();
    let other = component::columns_source(vec!["c0".into()], vec![Some(vec![9])]);
    assert_eq!(
        next_run
            .read(&db, "keys", "c0", Some(other))
            .unwrap()
            .as_slice(),
        &[9]
    );
}

#[test]
fn sql_demands_are_read_once_and_cached_without_requiring_native_inputs() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE keys(c0 INTEGER); INSERT INTO keys VALUES(7),(7),(3)")
        .unwrap();
    let mut demands = Demands::default();
    let first = demands.read(&db, "keys", "c0", None).unwrap();
    db.execute_batch("DROP TABLE keys").unwrap();
    let second = demands.read(&db, "keys", "c0", None).unwrap();
    assert_eq!(first.as_slice(), &[3, 7]);
    assert!(Arc::ptr_eq(&first, &second));
}

#[cfg(test)]
mod integration_tests;
