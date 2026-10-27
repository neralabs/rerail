use super::{TOP, register};
use rusqlite::Connection;

fn connection() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    db
}

fn pairs(db: &Connection, sql: &str) -> Vec<(i64, i64)> {
    let mut statement = db.prepare(sql).unwrap();
    statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

/// Copies carry a value along their edges, a binary transfer combines two, and two different values meet at TOP.
#[test]
fn flat_lattice_fixpoint_joins_copies_and_transfers() {
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE p_seed(c0, c1); INSERT INTO p_seed VALUES (0, 2), (1, 3), (5, 7);
         CREATE TEMP TABLE p_copy(c0, c1); INSERT INTO p_copy VALUES (2, 0), (3, 2), (4, 0), (4, 1), (6, 6);
         CREATE TEMP TABLE p_bin(c0, c1, c2, c3); INSERT INTO p_bin VALUES (5, '+', 0, 1), (7, '*', 3, 1);",
    )
    .unwrap();
    let report: String = db.query_row("SELECT lattice_fixpoint('p')", [], |r| r.get(0)).unwrap();
    assert_eq!(report.split(' ').next(), Some("7"), "{report}");
    // 5 is seeded 7 and gets 2 + 3: TOP. 6 copies only itself and stays bottom (no row).
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM p_out ORDER BY c0"), [(0, 2), (1, 3), (2, 2), (3, 2), (4, TOP), (5, TOP), (7, 6)]);
    assert_eq!(db.query_row("SELECT flat_join(v) FROM (SELECT 4 v UNION ALL SELECT 4)", [], |r| r.get::<_, i64>(0)).unwrap(), 4);
}

/// An in-memory result answers point lookups through `lattice_result`.
#[test]
fn a_named_lattice_result_is_a_virtual_table() {
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE q_seed(c0, c1); INSERT INTO q_seed VALUES (1, 10);
         CREATE TEMP TABLE q_copy(c0, c1); INSERT INTO q_copy VALUES (3, 1);
         CREATE TEMP TABLE q_bin(c0, c1, c2, c3);",
    )
    .unwrap();
    db.query_row("SELECT lattice_fixpoint('q', '@q')", [], |r| r.get::<_, String>(0)).unwrap();
    db.execute_batch("CREATE VIRTUAL TABLE temp.qv USING lattice_result(q)").unwrap();
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM qv WHERE c0 = 3"), [(3, 10)]);
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM qv ORDER BY c0"), [(1, 10), (3, 10)]);
}

/// A linear rule (one recursive atom joined with an edge table) runs natively to the transitive closure, and the
/// new tuples are written to the relation's table at the fixpoint.
#[test]
fn set_fixpoint_closes_a_linear_rule_over_its_edges() {
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE edge(c0, c1); INSERT INTO edge VALUES (1, 2), (2, 3), (3, 4), (4, 2);
         CREATE TEMP TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;",
    )
    .unwrap();
    let spec = "name tc\nrel path ii\nrule 0 0 edge ii key v1 ; ecols v2 ; pat v0 v1 ; head v0 v2";
    db.query_row("SELECT set_fixpoint_open(?1)", [spec], |r| r.get::<_, String>(0)).unwrap();
    let report: String = db.query_row("SELECT set_fixpoint_run('tc')", [], |r| r.get(0)).unwrap();
    let fields: Vec<&str> = report.split(' ').collect();
    assert_eq!(fields.last(), Some(&"1"), "at the fixpoint: {report}");
    db.query_row("SELECT set_fixpoint_close('tc')", [], |r| r.get::<_, i64>(0)).unwrap();
    let closure = pairs(&db, "SELECT c0, c1 FROM path ORDER BY c0, c1");
    let mut expected = vec![(1, 2), (1, 3), (1, 4)];
    expected.extend([2, 3, 4].iter().flat_map(|&a| [(a, 2), (a, 3), (a, 4)]));
    assert_eq!(closure, expected);
}

/// A chain that keeps rising (a counter around a cycle) needs `widen`; with it, the cycle's nodes widen to the
/// lattice's top after k changes. Min is the same with the order reversed.
#[test]
fn unbounded_lattices_run_only_with_widen() {
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE c_seed(c0, c1); INSERT INTO c_seed VALUES (0, 1), (2, 1);
         CREATE TEMP TABLE c_copy(c0, c1); INSERT INTO c_copy VALUES (1, 0);
         CREATE TEMP TABLE c_bin(c0, c1, c2, c3); INSERT INTO c_bin VALUES (0, '+', 1, 2), (3, '-', 2, 1);",
    )
    .unwrap();
    let unbounded = db.query_row("SELECT lattice_fixpoint('c', NULL, 'max')", [], |r| r.get::<_, String>(0));
    assert!(unbounded.unwrap_err().to_string().contains("unbounded lattice"));
    db.query_row("SELECT lattice_fixpoint('c', NULL, 'max', 3)", [], |r| r.get::<_, String>(0)).unwrap();
    // 0 = max(1, 1 + 2's 1) rises with 1 around the cycle until it widens; 3 = 2 - 1 then absorbs TOP.
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM c_out ORDER BY c0"), [(0, TOP), (1, TOP), (2, 1), (3, TOP)]);
    db.execute_batch("UPDATE c_bin SET c1 = '-' WHERE c0 = 0").unwrap();
    db.query_row("SELECT lattice_fixpoint('c', NULL, 'min', 3)", [], |r| r.get::<_, String>(0)).unwrap();
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM c_out ORDER BY c0"), [(0, -TOP), (1, -TOP), (2, 1), (3, -TOP)]);
}

/// A finite lattice declared by the logic runs without `widen`.
#[test]
fn a_declared_lattice_replaces_the_join() {
    struct Or;
    impl super::Lattice for Or {
        fn lub(&self, a: i64, b: i64) -> i64 { a | b }
        fn top(&self) -> i64 { 0b111 }
        fn finite(&self) -> bool { true }
    }
    super::declare("bits", std::sync::Arc::new(Or));
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE b_seed(c0, c1); INSERT INTO b_seed VALUES (0, 1), (1, 2);
         CREATE TEMP TABLE b_copy(c0, c1); INSERT INTO b_copy VALUES (2, 0), (2, 1), (0, 2);
         CREATE TEMP TABLE b_bin(c0, c1, c2, c3);",
    )
    .unwrap();
    db.query_row("SELECT lattice_fixpoint('b', NULL, 'bits')", [], |r| r.get::<_, String>(0)).unwrap();
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM b_out ORDER BY c0"), [(0, 3), (1, 2), (2, 3)]);
}
