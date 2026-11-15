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

/// O168 and O169: a probe's whole plan (its delta is all its relation, in the first wave) is cut at its budget too,
/// and goes on as its whole plan. Trials over the rows it left run the whole body; the first to finish keeps its
/// rows for the wave, and its sample leaves the rows left. Handed to another whole order, the fixpoint is the
/// unlimited run's.
#[test]
fn a_whole_plan_is_cut_and_a_finished_trial_keeps_its_rows() {
    closure::semi_naive_here(true);
    let edges: Vec<String> = (0..300).flat_map(|i| [format!("({i}, {})", i + 1), format!("({i}, {})", (i * 7) % 300)]).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;\nCREATE TABLE node(c0); INSERT INTO node SELECT DISTINCT c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    let node = format!("t:b:{}:{}:i", hex("node"), hex("c0"));
    let delta = format!("pos d0 v0 v1\npos {input} v1 v2\npos {node} v2\nhead v0 v2");
    let whole = format!("pos c0 v0 v1\npos {input} v1 v2\npos {node} v2\nhead v0 v2");
    let other = format!("pos {input} v1 v2\npos c0 v0 v1\npos {node} v2\nhead v0 v2");
    let spec = format!("name wcut\nrel path ii\njoin 0 0 {} {}", hex(&delta), hex(&whole));
    let rows = |db: &Connection| -> Vec<(i64, i64)> { pairs(db, "SELECT DISTINCT c0, c1 FROM path ORDER BY c0, c1") };
    let unlimited = {
        let db = connection();
        setup(&db);
        open(&db, &spec).unwrap();
        assert_eq!(run(&db, "wcut", 0).unwrap().split_whitespace().last(), Some("1"));
        close(&db, "wcut").unwrap();
        rows(&db)
    };
    let db = connection();
    setup(&db);
    open(&db, &spec).unwrap();
    set_budgets(&db, "wcut", &[(0, Some(300))]).unwrap();
    assert_eq!(run(&db, "wcut", 1).unwrap().split_whitespace().last(), Some("5"));
    let cut = take_cut(&db, "wcut").unwrap().expect("a cut");
    assert!(cut.whole && cut.led && cut.left > 64, "{cut:?}");
    let other_spec = JoinSpec::parse(&other).unwrap();
    let own = trial(&db, "wcut", 0, None, None, 32, u64::MAX / 2).unwrap().expect("its whole body");
    let swapped = trial(&db, "wcut", 0, Some(&other_spec), None, 32, u64::MAX / 2).unwrap().expect("placed");
    assert_eq!(own.made, swapped.made, "{own:?} {swapped:?}");
    let kept = keep_trial_rows(&db, "wcut").unwrap();
    assert_eq!(kept, own.made, "the first finished trial's rows");
    assert_eq!(keep_trial_rows(&db, "wcut").unwrap(), 0, "kept once");
    reprobe_spec(&db, "wcut", 0, &Reprobe::Whole(std::sync::Arc::new(other_spec))).unwrap();
    assert_eq!(run(&db, "wcut", 0).unwrap().split_whitespace().last(), Some("1"));
    close(&db, "wcut").unwrap();
    assert_eq!(rows(&db), unlimited, "the fixpoint is the unlimited run's");
    closure::semi_naive_here(false);
}

/// Decision 0206 over a fused first step (an atom with the test after it run a chunk of rows at a time, `rowcode`):
/// a cut inside a chunk leaves the chunk's rows from the one it stopped at. Before, the chunk's loop went on past the
/// cut, moving the row it stopped at to the chunk's last, so the rows between were reported done and never run (a
/// fixpoint short of rows: typescript-eslint's no-shadow flagged valid code, its `count : { … } = 0` missing them).
#[test]
fn a_cut_inside_a_fused_chunk_leaves_the_rows_it_did_not_run() {
    let edges: Vec<String> = (0..300).flat_map(|i| [format!("({i}, {})", i + 1), format!("({i}, {})", (i * 7) % 300)]).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;\nCREATE TABLE node(c0); INSERT INTO node SELECT DISTINCT c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    closure::semi_naive_here(true);
    let node = format!("t:b:{}:{}:i", hex("node"), hex("c0"));
    // The delta's atom, then a test of its own columns: fused.
    let fused_first = format!("pos d0 v0 v1\nne v0 v1\npos {input} v1 v2\npos {node} v2\nhead v0 v2");
    let spec = format!("name cutf\nrel path ii\njoin 0 0 {}", hex(&fused_first));
    let rows = |db: &Connection| -> Vec<(i64, i64)> { pairs(db, "SELECT DISTINCT c0, c1 FROM path ORDER BY c0, c1") };
    let whole = {
        let db = connection();
        setup(&db);
        open(&db, &spec).unwrap();
        assert_eq!(run(&db, "cutf", 0).unwrap().split_whitespace().last(), Some("1"));
        close(&db, "cutf").unwrap();
        rows(&db)
    };
    let db = connection();
    setup(&db);
    open(&db, &spec).unwrap();
    assert!(matches!(&component(&db, "cutf").unwrap().lock().unwrap().probes[0].body, Body::Join(j) if j.fused[0].is_some()), "the first step is fused");
    set_budgets(&db, "cutf", &[(0, Some(300))]).unwrap();
    let report = run(&db, "cutf", 1).unwrap();
    assert_eq!(report.split_whitespace().last(), Some("5"), "cut: {report}");
    let cut = take_cut(&db, "cutf").unwrap().expect("a cut");
    assert!(cut.left > 1 && cut.done < 300, "a cut in the chunk leaves the rows after its row: {cut:?}");
    let (mut cuts, mut done, mut last) = (1, false, String::new());
    for _ in 0..10_000 {
        set_budgets(&db, "cutf", &[(0, Some(300))]).unwrap();
        last = run(&db, "cutf", 0).unwrap();
        match last.split_whitespace().last() {
            Some("1") => { done = true; break }
            Some("5") => { cuts += 1; take_cut(&db, "cutf").unwrap(); }
            _ => {}
        }
    }
    close(&db, "cutf").unwrap();
    assert!(done && cuts > 10, "to the fixpoint, cut often: {cuts} cuts, last {last}");
    assert_eq!(rows(&db), whole, "the fixpoint is the unlimited run's");
    closure::semi_naive_here(false);
}

/// A first step that scans many rows is walked in morsels on the pool (`run_join_in_each`'s `morsels`): the same
/// fixpoint and step counts as one walk's. Under a budget, each morsel stops at its share of it: the cut's rows done
/// and left are all the delta's, and the wave going on from the rows left reaches the same fixpoint.
#[test]
fn a_scan_walked_in_morsels_makes_one_walks_rows_and_cuts_within_its_budget() {
    if crate::threads::max() < 2 {
        return;
    }
    // Chains of three: 0 → 1 → 2, 3 → 4 → 5, …: 40,000 edges, each node reaching at most two others.
    let edges: Vec<String> = (0..60_000).filter(|i| i % 3 != 2).map(|i| format!("({i}, {})", i + 1)).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    closure::semi_naive_here(true);
    let spec = format!("name mors\nrel path ii\njoin 0 0 {}", hex(&format!("pos d0 v0 v1\npos {input} v1 v2\nhead v0 v2")));
    let rows = |db: &Connection| -> Vec<(i64, i64)> { pairs(db, "SELECT DISTINCT c0, c1 FROM path ORDER BY c0, c1") };
    let unlimited = |whole: bool| {
        let db = connection();
        setup(&db);
        open(&db, &spec).unwrap();
        let before = morsels_walked();
        let go = || run(&db, "mors", 0).unwrap();
        let report = if whole { walks_whole(go) } else { go() };
        assert_eq!(report.split_whitespace().last(), Some("1"), "{report}");
        let walked = morsels_walked() - before;
        let counts = step_counts(&db, "mors").unwrap();
        close(&db, "mors").unwrap();
        (rows(&db), counts, walked)
    };
    let (one, one_counts, none) = unlimited(true);
    let (many, many_counts, walked) = unlimited(false);
    assert_eq!(none, 0, "under walks_whole, no morsels");
    assert!(walked > 0, "the first wave's scan of 40,000 rows is walked in morsels");
    assert_eq!(many, one, "the same fixpoint");
    assert_eq!(one.len(), 60_000, "each chain's three pairs");
    assert_eq!(many_counts, one_counts, "the same step counts");
    // Under a budget: cut, the morsels' rows left read on, to the same fixpoint.
    let db = connection();
    setup(&db);
    open(&db, &spec).unwrap();
    set_budgets(&db, "mors", &[(0, Some(30_000))]).unwrap();
    let before = morsels_walked();
    let report = run(&db, "mors", 1).unwrap();
    assert_eq!(report.split_whitespace().last(), Some("5"), "cut: {report}");
    assert!(morsels_walked() > before, "the cut walk ran in morsels");
    let cut = take_cut(&db, "mors").unwrap().expect("a cut");
    assert!(cut.led && cut.done > 0 && cut.left > 0 && cut.done + cut.left == 40_000, "{cut:?}");
    set_budgets(&db, "mors", &[(0, None)]).unwrap();
    assert_eq!(run(&db, "mors", 0).unwrap().split_whitespace().last(), Some("1"));
    close(&db, "mors").unwrap();
    assert_eq!(rows(&db), one, "the fixpoint is the unlimited run's");
    closure::semi_naive_here(false);
}
