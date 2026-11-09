use super::{TOP, register};
use rusqlite::Connection;

/// A connection to a new content store, whose `atoms` table holds the symbols.
fn connection() -> Connection {
    let path = crate::code_rule_dsl::prefilter::tests::temp();
    drop(crate::code_rule_dsl::prefilter::tests::open(&path));
    let db = Connection::open(path).unwrap();
    register(&db).unwrap();
    db
}

/// SQL views may decode or intern symbols while a component loads its input batch.
#[test]
fn computed_sql_inputs_do_not_reenter_a_held_symbol_lock() {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let db = connection();
        super::register_symbols(&db).unwrap();
        let atom: i64 = db.query_row("SELECT sym_atom('kept')", [], |r| r.get(0)).unwrap();
        db.execute_batch(&format!("CREATE TEMP VIEW computed AS SELECT sym_atom(sym_text({atom})) AS c0;
            CREATE TEMP VIEW recomputed AS SELECT sym_atom(sym_text({atom})) AS c0;
            CREATE TEMP TABLE result(c0 INTEGER PRIMARY KEY);")).unwrap();
        let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
        let join = |name: &str| format!("pos t:b:{}:{}:i v0\nhead v0", hex(name), hex("c0"));
        super::setfix::open(&db, &format!("name reentrant\nrel result i empty\nseed 0 {}", hex(&join("computed")))).unwrap();
        super::setfix::reprobe(&db, "reentrant", 0, "join", &hex(&join("recomputed"))).unwrap();
        super::setfix::run(&db, "reentrant", 0).unwrap();
        let actual: i64 = db.query_row("SELECT c0 FROM result", [], |r| r.get(0)).unwrap();
        assert_eq!(actual, atom);
        super::setfix::close(&db, "reentrant").unwrap();
        send.send(()).unwrap();
    });
    receive.recv_timeout(std::time::Duration::from_secs(5)).expect("computed SQL input did not finish; see the worker panic or timeout");
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

/// Symbol columns hold the store's atoms: text read from a table is interned, a rule's `cat` creates new atoms,
/// and the rows written back are atom ids that join with the `atoms` table.
#[test]
fn symbols_are_the_stores_atoms() {
    let db = connection();
    db.execute_batch(
        "CREATE TEMP TABLE member(c0, c1); INSERT INTO member VALUES ('pkg', 'mod'), ('mod', 'f');
         CREATE TEMP TABLE qual(c0, c1); INSERT INTO qual VALUES ('pkg', 'pkg');",
    )
    .unwrap();
    // qual(child, parent_qual "." child) :- qual(parent, parent_qual), member(parent, child)
    let spec = "name q\nrel qual ss\nrule 0 0 member ss key v0 ; ecols v2 ; pat v0 v1 ; head v2 cat:v1:v2";
    db.query_row("SELECT set_fixpoint_open(?1)", [spec], |r| r.get::<_, String>(0)).unwrap();
    db.query_row("SELECT set_fixpoint_run('q')", [], |r| r.get::<_, String>(0)).unwrap();
    db.query_row("SELECT set_fixpoint_close('q')", [], |r| r.get::<_, i64>(0)).unwrap();
    let names: Vec<(String, String)> = db
        .prepare("SELECT a.value, b.value FROM qual JOIN atoms a ON a.atom = qual.c0 JOIN atoms b ON b.atom = qual.c1 WHERE typeof(qual.c1) = 'integer' ORDER BY 2")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(names, [("mod".to_owned(), "pkg.mod".to_owned()), ("f".to_owned(), "pkg.mod.f".to_owned())]);
    let atoms: i64 = db.query_row("SELECT count(*) FROM atoms WHERE value IN ('pkg', 'mod', 'f', 'pkg.mod', 'pkg.mod.f')", [], |r| r.get(0)).unwrap();
    assert_eq!(atoms, 5);
}

/// The device work queue's file-local FIX (list step 17) solves the CPU's system: many components (as
/// files), with chains, cycles, transfers and conflicting seeds, and one long counting chain,
/// give the same values on both.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn the_device_fix_waves_reach_the_cpus_fixpoint() {
    let db = connection();
    let (mut seeds, mut copies, mut bins) = (Vec::new(), Vec::new(), Vec::new());
    // 300 components of 40 cells: a chain with a back edge (a cycle), a fork, and transfers.
    let mut state = 7_u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state >> 33
    };
    for component in 0..300_i64 {
        let base = component * 40;
        seeds.push(format!("({base}, {})", next() % 5));
        if component % 3 == 0 {
            seeds.push(format!("({}, {})", base + 20, next() % 5));
        }
        for at in 1..40 {
            copies.push(format!("({}, {})", base + at, base + at - 1));
        }
        copies.push(format!("({base}, {})", base + 39));
        for _ in 0..5 {
            let (cell, left, right) = (base + (next() % 40) as i64, base + (next() % 40) as i64, base + (next() % 40) as i64);
            let op = ["+", "-", "*", "/"][(next() % 4) as usize];
            bins.push(format!("({cell}, '{op}', {left}, {right})"));
        }
    }
    // One long component, the queue's case: a 20,000-cell chain counting up from 0
    // (`cell = previous + one`), each value distinct, so a stale read shows.
    let (chain, one) = (12_000_i64, 32_000_i64);
    seeds.push(format!("({chain}, 0)"));
    seeds.push(format!("({one}, 1)"));
    for at in 1..20_000 {
        bins.push(format!("({}, '+', {}, {one})", chain + at, chain + at - 1));
    }
    db.execute_batch(&format!(
        "CREATE TEMP TABLE d_seed(c0, c1); INSERT INTO d_seed VALUES {};
         CREATE TEMP TABLE d_copy(c0, c1); INSERT INTO d_copy VALUES {};
         CREATE TEMP TABLE d_bin(c0, c1, c2, c3); INSERT INTO d_bin VALUES {};
         CREATE TEMP TABLE cpu_out(c0 INTEGER PRIMARY KEY, c1); CREATE TEMP TABLE metal_out(c0 INTEGER PRIMARY KEY, c1);
         CREATE TEMP TABLE metal_warm(c0 INTEGER PRIMARY KEY, c1);",
        seeds.join(","),
        copies.join(","),
        bins.join(",")
    ))
    .unwrap();
    let on_cpu = db.query_row("SELECT lattice_fixpoint('d', 'cpu_out')", [], |r| r.get::<_, String>(0)).unwrap();
    let on_metal = db.query_row("SELECT lattice_fixpoint('d', 'metal_out', 'flat', NULL, 'metal')", [], |r| r.get::<_, String>(0)).unwrap();
    // Again, with the pipeline built.
    let warm = db.query_row("SELECT lattice_fixpoint('d', 'metal_warm', 'flat', NULL, 'metal')", [], |r| r.get::<_, String>(0)).unwrap();
    // rows, waves (on Metal: launches), visits, solve ms, total ms.
    eprintln!("lattice FIX: cpu {on_cpu}; metal {on_metal}; metal warm {warm}");
    let cpu = pairs(&db, "SELECT c0, c1 FROM cpu_out ORDER BY c0");
    assert_eq!(cpu.len(), 32_001, "every cell is reached");
    assert_eq!(cpu[31_999], (31_999, 19_999), "the chain counts");
    assert!(cpu.iter().any(|&(_, v)| v == TOP) && cpu.iter().any(|&(_, v)| v != TOP));
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM metal_out ORDER BY c0"), cpu);
    assert_eq!(pairs(&db, "SELECT c0, c1 FROM metal_warm ORDER BY c0"), cpu);
}

/// A request relation pauses the fixpoint; the rows the caller adds to a produced relation are the next wave's delta,
/// and the run goes on from there (decision 0065 §2a). Here each request `x` is answered `got(x, x + 1)`, up to 5.
#[test]
fn a_fixpoint_pauses_for_requests_and_resumes_with_their_answers() {
    let db = connection();
    db.execute_batch("CREATE TEMP TABLE want(c0); INSERT INTO want VALUES (1); CREATE TEMP TABLE req(c0); CREATE TEMP TABLE got(c0, c1);").unwrap();
    let spec = "name asks\nrel want i\nrel req i empty\nrel got ii empty\nrequest 1\nproduced 2\n\
                rule 1 0 - - key ; ecols ; pat v0 ; head v0\nrule 0 2 - - key ; ecols ; pat v0 v1 ; head v1";
    super::setfix::open(&db, spec).unwrap();
    let mut pauses = 0;
    loop {
        let report = super::setfix::run(&db, "asks", 0).unwrap();
        match report.rsplit(' ').next() {
            Some("2") => {
                pauses += 1;
                let mut answer = Vec::new();
                for (table, rows) in super::setfix::pending(&db, "asks").unwrap() {
                    assert_eq!(table, "req");
                    answer.extend(rows.iter().filter(|r| r[0] < 5).map(|r| vec![r[0], r[0] + 1]));
                }
                super::setfix::add(&db, "asks", "got", &answer).unwrap();
            }
            Some("1") => break,
            other => panic!("unexpected status {other:?}: {report}"),
        }
    }
    assert_eq!(pauses, 5, "one pause per request, as each answer makes the next");
    let ints = |sql: &str| -> Vec<i64> { db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
    assert_eq!(ints("SELECT c0 FROM want ORDER BY c0"), [1, 2, 3, 4, 5]);
    assert_eq!(ints("SELECT c0 FROM req ORDER BY c0"), [1, 2, 3, 4, 5]);
    // A produced relation is the caller's to write: the fixpoint writes none of its rows.
    assert_eq!(ints("SELECT count(*) FROM got"), [0]);
    super::setfix::close(&db, "asks").unwrap();
}
