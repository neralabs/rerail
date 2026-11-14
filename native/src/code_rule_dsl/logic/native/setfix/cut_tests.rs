//! Decision 0206: a probe's work budget, its cut at a delta row, and the wave going on from there.
use super::spec::hex;
use super::*;

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db
}

fn pairs(db: &Connection, sql: &str) -> Vec<(i64, i64)> {
    db.prepare(sql).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

/// Decision 0206: a probe that spends its budget is cut at the delta row it is at, and its wave goes on from there
/// with the probe's body as it is then (here another order), over only the rows it left: the fixpoint is the
/// unlimited run's, and the cut row is the only one read twice.
#[test]
fn a_probe_cut_at_its_budget_hands_its_rows_left_to_another_order() {
    let edges: Vec<String> = (0..300).flat_map(|i| [format!("({i}, {})", i + 1), format!("({i}, {})", (i * 7) % 300)]).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;\nCREATE TABLE node(c0); INSERT INTO node SELECT DISTINCT c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    // Semi-naive, not as a closure (decision 0098 §1): the probe's waves are what is cut.
    closure::semi_naive_here(true);
    let node = format!("t:b:{}:{}:i", hex("node"), hex("c0"));
    let delta_first = format!("pos d0 v0 v1\npos {input} v1 v2\npos {node} v2\nhead v0 v2");
    let edge_first = format!("pos {input} v1 v2\npos d0 v0 v1\npos {node} v2\nhead v0 v2");
    let spec = format!("name cut\nrel path ii\njoin 0 0 {}", hex(&delta_first));
    let rows = |db: &Connection| -> Vec<(i64, i64)> { pairs(db, "SELECT DISTINCT c0, c1 FROM path ORDER BY c0, c1") };
    let whole = {
        let db = connection();
        setup(&db);
        open(&db, &spec).unwrap();
        assert_eq!(run(&db, "cut", 0).unwrap().split_whitespace().last(), Some("1"));
        close(&db, "cut").unwrap();
        rows(&db)
    };
    let db = connection();
    setup(&db);
    open(&db, &spec).unwrap();
    let component = set_budgets;
    component(&db, "cut", &[(0, Some(300))]).unwrap();
    let report = run(&db, "cut", 1).unwrap();
    assert_eq!(report.split_whitespace().last(), Some("5"), "cut: {report}");
    let cut = take_cut(&db, "cut").unwrap().expect("a cut");
    let n0: i64 = db.query_row("SELECT COUNT(*) FROM (SELECT DISTINCT c0, c1 FROM path)", [], |r| r.get(0)).unwrap();
    assert!(cut.led && cut.done > 0 && cut.left > 0 && (cut.done + cut.left) as i64 == n0, "{cut:?} of {n0}");
    let first: i64 = step_counts(&db, "cut").unwrap()[0][0].1;
    // The same order again over its rows left, to the wave's end: each delta row read once, the cut row twice.
    let report = run(&db, "cut", 1).unwrap();
    assert_eq!(report.split_whitespace().last(), Some("0"), "the wave finished: {report}");
    let then: i64 = step_counts(&db, "cut").unwrap()[0][0].1;
    assert_eq!(first + then, n0 + 1, "{cut:?}: {first} + {then}");
    // A cut again in the next wave, then another order over the rows left, to the fixpoint.
    component(&db, "cut", &[(0, Some(200))]).unwrap();
    let report = run(&db, "cut", 0).unwrap();
    assert_eq!(report.split_whitespace().last(), Some("5"), "cut: {report}");
    assert!(take_cut(&db, "cut").unwrap().is_some_and(|c| c.left > 0));
    // Decision 0207: trials over a sample of the rows the cut left. Both orders make the same rows from it, at their
    // own work; a budget under that work cuts the trial. Nothing is added, and the probe's counts are as they were.
    let other_spec = JoinSpec::parse(&edge_first).unwrap();
    // Read (and so reset) the counts first: trials leave them at nothing.
    step_counts(&db, "cut").unwrap();
    let own = trial(&db, "cut", 0, None, None, 64, u64::MAX / 2).unwrap().expect("a trial of its own body");
    let swapped = trial(&db, "cut", 0, Some(&other_spec), None, 64, u64::MAX / 2).unwrap().expect("the delta is placed in the other order");
    assert!(!own.cut && !swapped.cut && own.rows == swapped.rows && own.rows <= 64 && own.rows > 0, "{own:?} {swapped:?}");
    assert_eq!(own.made, swapped.made, "the same rows from the same sample: {own:?} {swapped:?}");
    assert_ne!(own.work, swapped.work);
    let short = trial(&db, "cut", 0, Some(&other_spec), None, 64, swapped.work / 2).unwrap().unwrap();
    assert!(short.cut && short.work <= swapped.work / 2 + 1, "{short:?}");
    assert!(step_counts(&db, "cut").unwrap()[0].iter().all(|&(a, b)| a == 0 && b == 0), "the probe's counts are as they were");
    let other = Reprobe::Join(std::sync::Arc::new(JoinSpec::parse(&edge_first).unwrap()));
    reprobe_spec(&db, "cut", 0, &other).unwrap();
    assert_eq!(run(&db, "cut", 0).unwrap().split_whitespace().last(), Some("1"));
    close(&db, "cut").unwrap();
    assert_eq!(rows(&db), whole, "the fixpoint is the unlimited run's");
    closure::semi_naive_here(false);
}
