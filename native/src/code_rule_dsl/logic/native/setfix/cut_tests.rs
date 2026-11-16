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

/// Decision 0206, a first step fused with its filters (`rowcode`): a cut inside a chunk stops the chunk at the row it
/// is at, so the rows after it are left and read by the wave going on. Before, the chunk's loop went on naming each
/// row it passed over as reached: the cut left one row, and the fixpoint lost what the rest derive.
#[test]
fn a_fused_first_step_cut_leaves_the_rows_after_its_row() {
    let edges: Vec<String> = (0..300).flat_map(|i| [format!("({i}, {})", i + 1), format!("({i}, {})", (i * 7) % 300)]).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;\nCREATE TABLE node(c0); INSERT INTO node SELECT DISTINCT c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    let node = format!("t:b:{}:{}:i", hex("node"), hex("c0"));
    closure::semi_naive_here(true);
    // `ne v0 v1` after the delta: the first step is fused with it into a row program.
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
    assert_eq!(run(&db, "cutf", 0).unwrap().split_whitespace().last(), Some("1"));
    close(&db, "cutf").unwrap();
    assert_eq!(rows(&db), whole, "the fixpoint is the unlimited run's");
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
    let (mut cuts, mut done, mut last) = (0, false, String::new());
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

/// Decision 0279 §7: a scan whose join only reads texts (here a split whose parts are looked up) walks in morsels, and
/// makes one walk's rows. Decision 0292 §1: one that makes atoms (the parts in its head, interned) walks in morsels too,
/// each deferring its new texts, interned at the merge in the morsels' order: one walk's rows, under the atoms one
/// walk gives them (each walk's connection has an arena of its own).
#[test]
fn a_join_that_reads_texts_or_makes_atoms_walks_in_morsels_and_makes_one_walks_atoms() {
    if crate::threads::max() < 2 {
        return;
    }
    let setup = |db: &Connection| {
        register_symbols(db).unwrap();
        let w: Vec<String> = (0..40_000).map(|i| format!("('m{i}.x{}')", i % 50)).collect();
        let p: Vec<String> = (0..40_000).filter(|i| i % 3 == 0).map(|i| format!("('m{i}', 'x{}')", i % 50)).collect();
        db.execute_batch(&format!("CREATE TABLE w(c0); INSERT INTO w VALUES {};\nCREATE TABLE p(c0, c1); INSERT INTO p VALUES {};\nCREATE TABLE res(c0, c1); CREATE TABLE one(c0);", w.join(", "), p.join(", "))).unwrap();
    };
    let (w, p) = (format!("t:b:{}:{}:s", hex("w"), hex("c0")), format!("t:b:{}:{}:ss", hex("p"), hex("c0, c1")));
    // `rel`: the table and its types; the rows read back as two texts (the second empty for one column).
    let walk = |join: &str, rel: &str, whole: bool| {
        let db = connection();
        setup(&db);
        open(&db, &format!("name txt\nrel {rel} empty\nseed 0 {}", hex(join))).unwrap();
        let before = morsels_walked();
        let go = || run(&db, "txt", 0).unwrap();
        if whole { walks_whole(go) } else { go() };
        let walked = morsels_walked() - before;
        let text = |c: &str| format!("CASE typeof({c}) WHEN 'integer' THEN sym_text({c}) ELSE {c} END");
        let (table, second) = if rel.starts_with("one ") { ("one", "''".to_owned()) } else { ("res", text("c1")) };
        let rows: Vec<(String, String)> = db.prepare(&format!("SELECT {}, {second} FROM {table} ORDER BY 1, 2", text("c0"))).unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        // Each text with the value it is stored under: the atom where it is one.
        let atoms: Vec<(String, String)> = db.prepare(&format!("SELECT {}, CAST(c0 AS TEXT) FROM {table} ORDER BY 1, 2", text("c0"))).unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        close(&db, "txt").unwrap();
        (rows, walked, atoms)
    };
    // The head reads the whole, not the parts: they are only keys of `p`.
    let looked_up = format!("pos {w} v0\ncat v0 v1 v2\npos {p} v1 v2\nhead v0");
    let (one, _, _) = walk(&looked_up, "one s", true);
    let (many, walked, _) = walk(&looked_up, "one s", false);
    assert!(walked > 0, "the split's parts are only looked up: its scan walks in morsels");
    assert_eq!(one.len(), 13_334, "every third name's parts are a row of p");
    assert_eq!(many, one, "the same rows");
    let interned = format!("pos {w} v0\ncat v0 v1 v2\nhead v1 v2");
    let (whole, none, whole_atoms) = walk(&interned, "res ss", true);
    let (made, walked, made_atoms) = walk(&interned, "res ss", false);
    assert_eq!(none, 0, "under walks_whole, no morsels");
    assert!(walked > 0, "parts in the head are deferred in each morsel: the walk is in morsels");
    assert_eq!(whole.len(), 40_000 * 2, "each name at its start and at its dot");
    assert_eq!(made, whole, "the same rows");
    assert!(whole_atoms.iter().any(|(t, v)| t != v), "the parts are atoms");
    assert_eq!(made_atoms, whole_atoms, "the same atom for each text: interned in one walk's order");
}

/// Q36: a profiled walk times each of its steps, and last its head's rows made (`StepClock`), scaled to its
/// thread's CPU; `step_cpu` takes them as `step_counts` takes the counts. Unprofiled, nothing is timed.
#[test]
fn a_profiled_walk_times_each_step_and_its_heads_rows() {
    let edges: Vec<String> = (0..300).flat_map(|i| [format!("({i}, {})", i + 1), format!("({i}, {})", (i * 7) % 300)]).collect();
    let setup = |db: &Connection| {
        db.execute_batch(&format!("CREATE TABLE edge(c0, c1); INSERT INTO edge VALUES {};\nCREATE TABLE path(c0, c1); INSERT INTO path SELECT c0, c1 FROM edge;\nCREATE TABLE node(c0); INSERT INTO node SELECT DISTINCT c1 FROM edge;", edges.join(", "))).unwrap();
    };
    let input = format!("t:b:{}:{}:ii", hex("edge"), hex("c0, c1"));
    closure::semi_naive_here(true);
    let node = format!("t:b:{}:{}:i", hex("node"), hex("c0"));
    let body = format!("pos d0 v0 v1\npos {input} v1 v2\npos {node} v2\nhead v0 v2");
    let spec = format!("name clock\nrel path ii\njoin 0 0 {}", hex(&body));
    for profiled in [false, true] {
        crate::code_rule_dsl::logic::profile::set(profiled);
        let db = connection();
        setup(&db);
        open(&db, &spec).unwrap();
        assert_eq!(run(&db, "clock", 0).unwrap().split_whitespace().last(), Some("1"));
        let counts = step_counts(&db, "clock").unwrap();
        let cpu = step_cpu(&db, "clock").unwrap();
        assert_eq!(cpu[0].len(), counts[0].len() + 1, "each step and the head's rows: {cpu:?}");
        assert!(cpu[0].iter().all(|c| c.is_finite() && *c >= 0.0), "{cpu:?}");
        if profiled {
            assert!(cpu[0].iter().sum::<f64>() > 0.0, "a walk of {counts:?} took no time: {cpu:?}");
            assert!(step_cpu(&db, "clock").unwrap()[0].iter().all(|&c| c == 0.0), "taken, the clocks start again");
        } else {
            assert!(cpu[0].iter().all(|&c| c == 0.0), "unprofiled, nothing is timed: {cpu:?}");
        }
        close(&db, "clock").unwrap();
    }
    crate::code_rule_dsl::logic::profile::set(false);
}

/// A literal that makes two steps (`$k = @sub(@strlen($d), 1)`) is one plan step: its counts are the bindings into
/// its first step and out of its last, its CPU both steps'; the head's computed terms' steps go to the head, and a
/// plan step that made no step passes its rows on. Zipped by place, the steps after it were their neighbours'.
#[test]
fn counts_and_cpu_are_reported_by_the_plan_step_that_made_the_steps() {
    // Plan steps: 0 (an atom), 1 (two steps), 2 (none), 3 (an atom); then a head's `@pkg_up` (one step).
    let from = [0, 1, 1, 3, 4];
    let counts = [(1, 10), (10, 10), (10, 8), (8, 3), (3, 3)];
    assert_eq!(plan_counts(&from, 4, &counts), vec![(1, 10), (10, 8), (8, 8), (8, 3)]);
    let cpu = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    assert_eq!(plan_cpu(&from, 4, &cpu), vec![1.0, 6.0, 0.0, 8.0, 48.0]);
    // A spec read from text: each step its own plan step.
    let from: Vec<usize> = (0..3).collect();
    assert_eq!(plan_counts(&from, 3, &counts[..3]), counts[..3].to_vec());
}
