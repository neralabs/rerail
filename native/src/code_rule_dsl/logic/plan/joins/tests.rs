use super::*;
use crate::code_rule_dsl::logic::plan::{KeepPlans, Model, Stats, atom_sig, body};
use crate::code_rule_dsl::logic::plan::Inputs;
use crate::code_rule_dsl::logic::rr::parse;
use std::collections::{BTreeSet, HashSet};

/// The memo is the process's: tests that write it take turns.
static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[test]
fn notes_and_exact_statistics_read_back_as_written() {
    let n = Note {
        rule: 7,
        runs: 3,
        slow: 1,
        critical: 0.25,
        est: 100.0,
        ran: 5e5,
        wants_search: true,
        tried: vec![Tried { order: vec![(0, None), (2, Some(Mode::Range)), (1, None)], ratio: 12.5, runs: 2 }],
        proposals: vec![(vec![(2, None), (0, None), (1, Some(Mode::Climb)), (3, None), (4, None)], 80.0)],
        exact: Vec::new(),
        raised: Vec::new(),
        walked: 2.5e5,
    };
    let key: PlanKey = (5u128 << 70) | 9;
    assert_eq!(of_note_row(&note_row(key, &n)), Some((key, n.clone())));
    // Decision 0243: a row stored before the walked work was kept ends at its proposals, and reads as priced.
    let row = note_row(key, &n);
    assert_eq!(of_note_row(&row[..row.len() - 1]), Some((key, Note { walked: 0.0, ..n })));
    let sig: Sig = ("rel".into(), vec![(0, None), (2, Some(Constant::Int(-4))), (3, Some(Constant::Str("a=b".into())))]);
    let (ints, rel, cols, c) = exact_row(key, 7, &sig, [40, 400]);
    assert_eq!(of_exact_row(&ints, &rel, &cols, c), Some((key, 7, sig, [40, 400])));
}

#[test]
fn random_orders_are_distinct_connected_and_start_where_asked() {
    // a(x,y), b(y,z), c(z,w), d(q): d shares nothing, so it comes last.
    let vars = [0b0011u128, 0b0110, 0b1100, 0b1_0000];
    let mut rng = Rng(12345);
    let orders = random_orders(&vars, Some(1), 20, &mut rng);
    assert!(orders.len() >= 2, "{orders:?}");
    let distinct: FxHashSet<&Vec<usize>> = orders.iter().collect();
    assert_eq!(distinct.len(), orders.len());
    for o in &orders {
        assert_eq!(o[0], 1);
        assert_eq!(o.len(), 4);
        assert_eq!(o[3], 3, "the unconnected join is placed once nothing connected is left: {o:?}");
        let mut bound = vars[o[0]];
        for &j in &o[1..3] {
            assert!(vars[j] & bound != 0, "connected: {o:?}");
            bound |= vars[j];
        }
    }
}

#[test]
fn the_critical_path_is_the_heaviest_chain_and_its_heavy_lines_are_noted() {
    let _serial = SERIAL.lock();
    clear();
    // s0 -> s1 -> s3 (work 10 + 200k + 10), s0 -> s2 -> s3 (10 + 50k + 10).
    let names: Vec<String> = ["s0", "s1", "s2", "s3"].map(String::from).to_vec();
    let producers = vec![vec![], vec![0], vec![0], vec![1, 2]];
    let line = |stratum: &str, key: PlanKey, work: f64, est: f64| LineRun { stratum: stratum.into(), key, rule: key as u64, order: vec![(0, None), (1, None)], work, est, ..Default::default() };
    let runs = vec![line("s0", 100, 10.0, 10.0), line("s1", 101, 200_000.0, 1_000.0), line("s2", 102, 50_000.0, 50_000.0), line("s3", 103, 10.0, 10.0)];
    let path = critical(&names, &producers, &runs);
    assert_eq!(path.strata, vec![0, 1, 3]);
    assert_eq!(path.work, 200_020.0);
    assert_eq!(path.lines.iter().map(|l| l.0).collect::<Vec<_>>(), vec![101], "s2 is off the path, s0 and s3 too light");
    let n = note(&101).expect("noted");
    assert!(n.critical > 0.99 && n.wants_search, "{n:?}");
    assert_eq!(n.tried.len(), 1);
    assert!((n.tried[0].ratio - 200.0).abs() < 1e-6, "measured over priced: {n:?}");
    assert!(note(&102).is_none() && note(&100).is_none());
    clear();
}

#[test]
fn a_choice_is_corrected_by_what_its_order_measured_and_rounds_choose_together() {
    let _serial = SERIAL.lock();
    clear();
    let order = |at: &[usize]| at.iter().map(|&a| Step { at: a, mode: None }).collect::<Vec<_>>();
    let candidates = vec![Candidate { order: order(&[0, 1]), est: 100.0 }, Candidate { order: order(&[1, 0]), est: 300.0 }];
    // Not noted: the search's best.
    let mut round = vec![Choice { key: 200, candidates: candidates.clone(), chosen: 9, why: "" }];
    choose(&mut round);
    assert_eq!(round[0].chosen, 0);
    // The best order measured 20 times its price: the other is cheaper, corrected.
    critical(&["s".into()], &[vec![]], &[LineRun { stratum: "s".into(), key: 200, rule: 1, order: steps(&order(&[0, 1])), work: 2e6, est: 1e5, ..Default::default() }]);
    let mut round = vec![Choice { key: 200, candidates: candidates.clone(), chosen: 0, why: "" }];
    choose(&mut round);
    assert_eq!(round[0].chosen, 1, "{}", round[0].why);
    // In a round, the choices are recorded and made together at its end.
    let open = Round::open();
    assert!(deferred(&Choice { key: 200, candidates: candidates.clone(), chosen: 0, why: "" }));
    assert!(deferred(&Choice { key: 201, candidates, chosen: 0, why: "" }));
    assert_eq!(Round::len(), 2);
    let made = open.finish();
    assert_eq!(made.iter().map(|c| c.chosen).collect::<Vec<_>>(), vec![1, 0]);
    assert!(!deferred(&made[0]), "no round is open after it finished");
    clear();
}

#[test]
fn a_noted_line_explores_untried_orders_within_the_margin() {
    let _serial = SERIAL.lock();
    clear();
    let order = |at: &[usize]| at.iter().map(|&a| Step { at: a, mode: None }).collect::<Vec<_>>();
    critical(&["s".into()], &[vec![]], &[LineRun { stratum: "s".into(), key: 300, rule: 1, order: steps(&order(&[0, 1, 2])), work: 1e6, est: 1e6, ..Default::default() }]);
    let candidates = vec![
        Candidate { order: order(&[0, 1, 2]), est: 100.0 },
        Candidate { order: order(&[1, 0, 2]), est: 150.0 },
        Candidate { order: order(&[2, 1, 0]), est: 10_000.0 },
    ];
    let mut explored = [0usize; 3];
    for _ in 0..400 {
        let mut round = vec![Choice { key: 300, candidates: candidates.clone(), chosen: 0, why: "" }];
        choose(&mut round);
        explored[round[0].chosen] += 1;
    }
    assert!(explored[1] > 0, "an untried order within the margin is tried now and then: {explored:?}");
    assert_eq!(explored[2], 0, "never one past the margin");
    assert!(explored[0] > explored[1], "mostly the best: {explored:?}");
    clear();
}

/// `h(x, z) :- a(x, y), b(y, z)`: priced from its statistics, `a` (50 rows) leads and each probes `b` by `y` for a
/// row. Exact statistics show `b` holds 1,000 rows a value of `a`'s: `b` leads once they are measured. And a line
/// noted with its best order measured far over its price takes the other at its next plan, its wider search left to
/// the background.
#[test]
fn walked_orders_are_costed_against_the_walked_order_that_ran() {
    let _serial = SERIAL.lock();
    clear();
    let order = |at: &[usize]| at.iter().map(|&a| Step { at: a, mode: None }).collect::<Vec<_>>();
    // The order that ran measured twice its price; the other is priced ten times higher.
    critical(&["s".into()], &[vec![]], &[LineRun { stratum: "s".into(), key: 210, rule: 1, order: steps(&order(&[0, 1])), work: 2e5, est: 1e5, ..Default::default() }]);
    let candidates = vec![Candidate { order: order(&[0, 1]), est: 100.0 }, Candidate { order: order(&[1, 0]), est: 1000.0 }];
    let mut round = vec![Choice { key: 210, candidates: candidates.clone(), chosen: 9, why: "" }];
    choose(&mut round);
    assert_eq!(round[0].chosen, 0, "priced: {}", round[0].why);
    // The background walked the other at a quarter of the one that ran: 50 in the run's units, against its 200.
    propose(210, vec![(steps(&order(&[0, 1])), 4e5), (steps(&order(&[1, 0])), 1e5)], Some(4e5));
    let mut round = vec![Choice { key: 210, candidates, chosen: 9, why: "" }];
    choose(&mut round);
    assert_eq!(round[0].chosen, 1, "walked: {}", round[0].why);
    clear();
}

#[test]
fn exact_statistics_and_the_memo_change_the_plan() {
    let _serial = SERIAL.lock();
    clear();
    let p = parse(".decl a(x: number, y: number)\n.decl b(y: number, z: number)\n.decl h(x: number, z: number)\nh(x, z) :- a(x, y), b(y, z).\n.output h\n").unwrap();
    let inputs = Inputs { facts: "main.".into(), stats: "main.".into(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    for (rel, rows) in [("a", 50.0), ("b", 100.0)] {
        model.stats.card.insert(rel, rows);
        for c in 0..2 {
            model.ndv.insert((rel, c), rows);
            model.stats.fan.insert((rel, c), 1.0);
        }
    }
    let r = &p.rules[0];
    let v = body::alone(r).unwrap();
    let b = body::nth(&v, 0);
    let lead = |order: &[Step]| r.body[order[0].at].atom().map(|a| a.rel.to_string()).unwrap();
    let ((order, work, _), _) = model.plan_inner(&p, &b, None, None, false).unwrap();
    assert_eq!(lead(&order), "a", "{work}");
    let bound: BTreeSet<String> = ["x", "y"].map(String::from).into();
    let atom_b = r.body[1].atom().unwrap();
    put_exact(1, b.observed_key(), vec![(atom_sig(atom_b, &bound), [50, 50_000])]);
    let ((order, work, _), _) = model.plan_inner(&p, &b, None, None, false).unwrap();
    assert_eq!(lead(&order), "b", "exact: {work}");
    clear();

    // The memo: the line's best order ran 10,000 times its price; its next plan takes the other.
    let _plans = KeepPlans::start(None);
    let key = model.reused_key(&b, None);
    let ((best, _, _), _) = model.plan_rule_reused(&p, &b, None).unwrap();
    assert_eq!(lead(&best), "a");
    critical(&["h".into()], &[vec![]], &[LineRun { stratum: "h".into(), key, rule: b.observed_key(), order: steps(&best), work: 1e6, est: 100.0, ..Default::default() }]);
    assert!(pending(&key), "a wider search is wanted");
    let ((order, _, _), _) = model.plan_rule_reused(&p, &b, None).unwrap();
    assert_eq!(lead(&order), "b", "corrected by what `a` first measured");
    let n = note(&key).unwrap();
    // Decision 0243: the wider search is the background's, asked for at the line's end; the plan makes none.
    assert!(n.wants_search && n.proposals.is_empty(), "no wider search at the plan: {n:?}");
    clear();
}

/// A wider search asked for is run by the upkeep job's step (here on the test's thread): its orders are the line's
/// proposals.
#[test]
fn a_requested_search_proposes_its_orders() {
    let _serial = SERIAL.lock();
    clear();
    {
        let mut m = MEMO.write();
        noted_mut(&mut m, 400, 1).wants_search = true;
    }
    request(400, 1.0, false, Box::new(|_| Searched { orders: vec![(vec![(1, None), (0, None)], 5.0), (vec![(0, None), (1, None)], 3.0)], walked: None }));
    assert_eq!(requests(), 1);
    assert!(!wants_request(&400), "one request a line");
    assert!(search_one(&|| false));
    assert!(!search_one(&|| false));
    let n = note(&400).unwrap();
    assert_eq!(n.proposals.iter().map(|p| p.1).collect::<Vec<_>>(), vec![3.0, 5.0]);
    assert!(!n.wants_search);
    clear();
}

/// Decision 0207: a line's orders are trialled once in a process, and once more after a flag; each trial's ratio is
/// kept as an order tried, as a run's is.
#[test]
fn trials_are_taken_once_and_their_ratios_kept() {
    let _serial = SERIAL.lock();
    clear();
    assert!(take_trial(500) && !take_trial(500), "once");
    flag_slow(500, 1, 10.0, 1e6);
    assert!(take_trial(500), "again after a flag");
    let order = vec![Step { at: 1, mode: None }, Step { at: 0, mode: None }];
    put_tried(500, 1, &order, 4.0);
    put_tried(500, 1, &order, 16.0);
    let n = note(&500).unwrap();
    let t = n.tried.iter().find(|t| t.order == steps(&order)).expect("tried");
    assert!((t.ratio - 8.0).abs() < 1e-9 && t.runs == 2, "a geometric mean over trials: {t:?}");
    clear();
}

/// Decision 0208: the path as the strata ran. s1 does the most work but ran as a job beside s2, which took longer:
/// the measured path is s0, s2, s3 (s3 waited on s2, ending last), not the predicted s0, s1, s3; the loop's thread
/// ran s0 then s2, so s2 also waited on s0 there. Its line is noted by its share of the path's seconds.
#[test]
fn the_measured_critical_path_follows_what_each_stratum_waited_for() {
    let _serial = SERIAL.lock();
    clear();
    let names: Vec<String> = ["s0", "s1", "s2", "s3"].map(String::from).to_vec();
    let producers = vec![vec![], vec![0], vec![0], vec![1, 2]];
    let timeline = vec![Some((0.0, 0.1, 0)), Some((0.1, 0.3, 1)), Some((0.12, 0.9, 0)), Some((0.95, 1.0, 0))];
    let m = measured_path(&producers, &timeline).unwrap();
    assert_eq!(m.strata, vec![0, 2, 3]);
    assert!((m.busy - (0.1 + 0.78 + 0.05)).abs() < 1e-9 && (m.waited - (0.02 + 0.05)).abs() < 1e-9 && (m.span - 1.0).abs() < 1e-9, "{m:?}");
    let line = |stratum: &str, key: PlanKey, work: f64, wall: f64| LineRun { stratum: stratum.into(), key, rule: key as u64, order: vec![(0, None), (1, None)], work, est: work, wall };
    let runs = vec![line("s1", 701, 500_000.0, 0.19), line("s2", 702, 20_000.0, 0.7), line("s3", 703, 20_000.0, 0.04)];
    let path = critical_timed(&names, &producers, &runs, Some(&timeline));
    assert_eq!(path.strata, vec![0, 1, 3], "predicted by work");
    let measured = path.measured.as_ref().unwrap();
    assert_eq!((measured.strata.clone(), measured.shared), (vec![0, 2, 3], 2));
    let noted: Vec<PlanKey> = path.lines.iter().map(|l| l.0).collect();
    assert_eq!(noted, vec![702, 703], "the lines on the measured path, by seconds: {:?}", path.lines);
    assert!(note(&701).is_none(), "s1 is off the measured path");
    assert!((path.lines[0].1 - 0.7 / 0.74).abs() < 1e-9);
    clear();
}

/// Decision 0209 §4: `h(a, b) :- r(f, a), r(f, b), r(f, c)` over a key of 100 values of one row and one of 300. A
/// fanout (sq / rows) prices the third atom as the second: 20.3 M rows. Its heavy values price each step by the
/// binding drawn through the atoms before it, so the third reads cube / sq a binding: Σ count³, 27.0 M, exact.
#[test]
fn heavy_values_price_a_skewed_three_way_self_join_by_its_third_moment() {
    let p = parse(".decl r(f: number, x: number)\n.decl h(a: number, b: number)\nh(a, b) :- r(f, a), r(f, b), r(f, c).\n.output h\n").unwrap();
    let inputs = Inputs { facts: "main.".into(), stats: "main.".into(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    let counts: Vec<(i64, u64)> = (0..100).map(|v| (v, 1)).chain([(1000, 300)]).collect();
    let h = crate::code_rule_dsl::logic::plan::table::HeavyCol::of_counts(counts);
    model.stats.card.insert("r", h.rows);
    model.ndv.insert(("r", 0), h.ndv);
    model.stats.fan.insert(("r", 0), h.sq / h.rows);
    model.ndv.insert(("r", 1), h.rows);
    model.stats.fan.insert(("r", 1), 1.0);
    let r = &p.rules[0];
    let v = body::alone(r).unwrap();
    let b = body::nth(&v, 0);
    let last = |m: &Model| *m.plan_rule_step_rows(&p, &b, None).unwrap().1.last().unwrap();
    let by_fan = last(&model);
    assert!((by_fan / (h.sq * h.sq / h.rows) - 1.0).abs() < 0.01, "{by_fan}");
    let cube = h.cube;
    model.stats.heavy.insert(("r", 0), std::sync::Arc::new(h));
    let by_heavy = last(&model);
    assert!((by_heavy / cube - 1.0).abs() < 0.01, "{by_heavy} against {cube}");
}

/// Decision 0232: an order's work per lead row, by a walk of sampled bindings (`Walker`), is the work of running the
/// order, exactly where the sample holds every binding: a filter, a chain, and the atom closing a triangle
/// (`e(n, j)`, over two variables that two other atoms bound) each measured at the row's key. Two functions of one
/// size and one count of calls, with unlike edges among their nodes, have unlike work.
#[test]
fn an_orders_work_by_a_walk_is_its_run_through_a_triangle() {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use crate::code_rule_dsl::logic::native::component::InRel;
    use crate::code_rule_dsl::logic::plan::{Step, Walker};
    use std::collections::HashMap;
    let p = parse(".decl r(f: number, x: number)\n.decl k(x: number, t: number)\n.decl e(a: number, b: number)\n.decl h(f: number, n: number)\nh(f, n) :- r(f, n), k(n, 1), r(f, m), k(m, 2), r(f, j), e(n, j), k(j, 3).\n.output h\n").unwrap();
    let rule = &p.rules[0];
    // Two functions of 30 nodes: 3 calls (kind 1), 2 returns (kind 2), the rest identifiers (kind 3). Function 1's
    // calls have an edge to every node; function 2's, one each.
    let (mut r, mut k, mut e): (Vec<[i64; 2]>, Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new(), Vec::new());
    for f in [1i64, 2] {
        let nodes: Vec<i64> = (0..30).map(|i| f * 1000 + i).collect();
        for (i, &n) in nodes.iter().enumerate() {
            r.push([f, n]);
            k.push([n, if i < 3 { 1 } else if i < 5 { 2 } else { 3 }]);
        }
        for &c in &nodes[..3] {
            if f == 1 { for &j in &nodes { e.push([c, j]) } } else { e.push([c, nodes[10]]) }
        }
    }
    let rel = |rows: &[[i64; 2]]| InRel::from_flat(rows.iter().flatten().copied().collect(), rows.len(), vec![b'i'; 2]);
    let rels: HashMap<&str, std::sync::Arc<InRel>> = [("r", rel(&r)), ("k", rel(&k)), ("e", rel(&e))].into_iter().collect();
    let order: Vec<Step> = [3, 2, 0, 1, 4, 5, 6].into_iter().map(|at| Step { at, mode: None }).collect();
    // The order run by hand: every binding enumerated, each step adding the bindings it reads.
    let run = |start: [i64; 2]| -> f64 {
        let mut binds: Vec<HashMap<String, i64>> = vec![[("m".to_owned(), start[0])].into_iter().collect()];
        let mut work = 0.0;
        for s in order.iter().skip(1) {
            work += binds.len() as f64;
            let Literal::Atom(a) = &rule.body[s.at] else { unreachable!() };
            let rows = match a.rel.as_ref() { "r" => &r, "k" => &k, _ => &e };
            let mut next = Vec::new();
            for b in &binds {
                for row in rows.iter() {
                    let mut nb = b.clone();
                    let ok = a.args.iter().zip(row).all(|(t, &v)| match t {
                        Term::Int(i) => *i == v,
                        Term::Var(x) => match nb.get(x.as_ref()) { Some(&w) => w == v, None => { nb.insert(x.to_string(), v); true } },
                        _ => true,
                    });
                    if ok { next.push(nb) }
                }
            }
            binds = next;
        }
        work
    };
    let walker = Walker::new(rule, 3, 1 << 16, &|n| rels.get(n).cloned(), &|_| None);
    for f in [1i64, 2] {
        let start = [f * 1000 + 3, 2];
        assert_eq!(walker.work(&order, &[start.to_vec()]), run(start), "function {f}");
    }
    let (w1, w2) = (walker.work(&order, &[vec![1003, 2]]), walker.work(&order, &[vec![2003, 2]]));
    // The same size and calls; the difference is the edges the triangle's atom passes: 90 in function 1, 3 in 2.
    assert_eq!((w1, w2), (215.0, 128.0), "the triangle's edges, measured at the key");
    // A narrow walk (16 bindings a step, of up to 90) is unbiased: its mean over walks is the work.
    let narrow = Walker::new(rule, 3, 16, &|n| rels.get(n).cloned(), &|_| None);
    let walks = 400;
    let est = (0..walks).map(|_| narrow.work(&order, &[vec![1003, 2]])).sum::<f64>() / walks as f64;
    assert!((est / w1 - 1.0).abs() < 0.05, "{est} against {w1}");
    // Decision 0236: narrower still (2 bindings a step, 32 before checks), the triangle's 90 edges pass the probes'
    // limit and are drawn by position (`InRel::probe_sample`); the mean is still the work.
    let thin = Walker::new(rule, 3, 2, &|n| rels.get(n).cloned(), &|_| None);
    let est = (0..2000).map(|_| thin.work(&order, &[vec![1003, 2]])).sum::<f64>() / 2000.0;
    assert!((est / w1 - 1.0).abs() < 0.05, "{est} against {w1}");
}

/// Decision 0236: a probe with more candidates than its limit reads that many, drawn by position, and estimates its
/// matches from their share; with fewer, it reads them all and is exact.
#[test]
fn a_sampled_probe_reads_its_limit_and_estimates_its_matches() {
    use crate::code_rule_dsl::logic::native::component::InRel;
    // Key 7: 10,000 rows, every fourth of kind 1. Key 8: 50 rows, all kind 1.
    let mut rows: Vec<i64> = Vec::new();
    for i in 0..10_000i64 { rows.extend([7, i, (i % 4 == 0) as i64]) }
    for i in 0..50i64 { rows.extend([8, i, 1]) }
    let rel = InRel::from_flat(rows, 10_050, vec![b'i'; 3]);
    let mut seed = 0x5eed_u64;
    let mut draw = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
    let mut seen = 0;
    assert_eq!(rel.probe_sample(&[0], &[8], 64, &mut draw, &mut |_| seen += 1), 50.0);
    assert_eq!(seen, 50, "all of a small probe's matches");
    let (mut sum, mut seen) = (0.0, 0);
    for _ in 0..200 { sum += rel.probe_sample(&[0], &[7], 64, &mut draw, &mut |_| seen += 1) }
    assert_eq!(seen, 200 * 64, "the limit's rows, not the key's");
    assert!((sum / 200.0 / 10_000.0 - 1.0).abs() < 0.01, "{}", sum / 200.0);
    // Two columns: the index's candidates are the key's rows of kind 1; a quarter of key 7's.
    let mut sum = 0.0;
    for _ in 0..200 { sum += rel.probe_sample(&[2, 0], &[1, 7], 64, &mut draw, &mut |row| assert_eq!((row(0), row(2)), (7, 1))) }
    assert!((sum / 200.0 / 2_500.0 - 1.0).abs() < 0.03, "{}", sum / 200.0);
}

/// Decision 0243: a background search walks its orders over the line's relations as its request kept them
/// (`WalkSnap`), each from its own lead: their walked work over the line is the work of running each from every one
/// of its lead's rows, exactly where the samples hold every row, and it ranks them where their prices misled.
#[test]
fn a_background_search_ranks_its_orders_by_their_walked_work_over_the_line() {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use crate::code_rule_dsl::logic::native::component::InRel;
    use crate::code_rule_dsl::logic::plan::{Step, WalkSnap};
    use std::collections::HashMap;
    let p = parse(".decl r(f: number, x: number)\n.decl k(x: number, t: number)\n.decl e(a: number, b: number)\n.decl h(f: number, n: number)\nh(f, n) :- r(f, n), k(n, 1), r(f, m), k(m, 2), r(f, j), e(n, j), k(j, 3).\n.output h\n").unwrap();
    let rule = &p.rules[0];
    // The triangle's functions (decision 0232's test): 30 nodes each, 3 calls, 2 returns; function 1's calls have an
    // edge to every node, function 2's one each.
    let (mut r, mut k, mut e): (Vec<[i64; 2]>, Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new(), Vec::new());
    for f in [1i64, 2] {
        let nodes: Vec<i64> = (0..30).map(|i| f * 1000 + i).collect();
        for (i, &n) in nodes.iter().enumerate() {
            r.push([f, n]);
            k.push([n, if i < 3 { 1 } else if i < 5 { 2 } else { 3 }]);
        }
        for &c in &nodes[..3] {
            if f == 1 { for &j in &nodes { e.push([c, j]) } } else { e.push([c, nodes[10]]) }
        }
    }
    let table = |rel: &str| -> &Vec<[i64; 2]> { match rel { "r" => &r, "k" => &k, _ => &e } };
    // An order run by hand from every row of its lead, each step adding the bindings it reads.
    let run = |order: &[usize]| -> f64 {
        let Literal::Atom(lead) = &rule.body[order[0]] else { unreachable!() };
        let bind = |b: &HashMap<String, i64>, a: &crate::code_rule_dsl::logic::front::Atom, row: &[i64; 2]| -> Option<HashMap<String, i64>> {
            let mut nb = b.clone();
            for (t, &v) in a.args.iter().zip(row) {
                match t {
                    Term::Int(i) if *i != v => return None,
                    Term::Var(x) => match nb.get(x.as_ref()) { Some(&w) if w != v => return None, Some(_) => {}, None => { nb.insert(x.to_string(), v); } },
                    _ => {}
                }
            }
            Some(nb)
        };
        let mut binds: Vec<HashMap<String, i64>> = table(&lead.rel).iter().filter_map(|row| bind(&HashMap::new(), lead, row)).collect();
        let mut work = 0.0;
        for &at in &order[1..] {
            work += binds.len() as f64;
            let Literal::Atom(a) = &rule.body[at] else { unreachable!() };
            binds = binds.iter().flat_map(|b| table(&a.rel).iter().filter_map(|row| bind(b, a, row)).collect::<Vec<_>>()).collect();
        }
        work
    };
    let rel = |rows: &[[i64; 2]]| InRel::from_flat(rows.iter().flatten().copied().collect(), rows.len(), vec![b'i'; 2]);
    let snap = WalkSnap { rels: [("r", rel(&r)), ("k", rel(&k)), ("e", rel(&e))].into_iter().map(|(n, x)| (n.to_owned(), x)).collect(), syms: HashMap::new(), delta: None, heavy: HashMap::new() };
    let steps = |o: &[usize]| -> Vec<Step> { o.iter().map(|&at| Step { at, mode: None }).collect() };
    // From the returns, and from every node; the second priced as if far cheaper, and the order that ran.
    let (good, poor) = ([3usize, 2, 0, 1, 4, 5, 6], [0usize, 5, 6, 4, 2, 3, 1]);
    // And the good order with its lead and first check swapped: from every node, the returns' check first.
    let checked = [2usize, 3, 0, 1, 4, 5, 6];
    let found = snap.walk(rule, None, &steps(&poor), vec![(steps(&poor), 1.0), (steps(&good), 100.0), (steps(&checked), 50.0)], &|| false);
    let of = |o: &[usize]| found.orders.iter().find(|w| w.0 == steps_of(o)).map(|w| w.1);
    assert_eq!(of(&checked), Some(run(&checked)), "from the lead's rows past its check, with the check's work");
    let (ran, best) = (found.walked.expect("walked"), found.orders.iter().min_by(|a, b| a.1.total_cmp(&b.1)).expect("orders"));
    assert_eq!(best.0, steps_of(&good), "the walks rank the orders");
    // The good order's samples hold every binding: exact. The poor one's expansions pass the walk's width, and are
    // sampled.
    assert_eq!(best.1, run(&good), "the good order's work over the line, from every row of its lead");
    assert!((ran / run(&poor) - 1.0).abs() < 0.05, "{ran} against {}", run(&poor));
    assert!(ran > 4.0 * best.1, "{ran} against {}", best.1);
}

#[test]
fn a_background_walk_splits_its_lead_rows_by_a_heavy_key() {
    use crate::code_rule_dsl::logic::native::component::InRel;
    use crate::code_rule_dsl::logic::plan::{Step, WalkSnap};
    use std::collections::HashMap;
    // From the calls: each call's function, then the function's nodes, then their kinds.
    let p = parse(".decl r(f: number, x: number)\n.decl k(x: number, t: number)\n.decl h(f: number, n: number)\nh(f, n) :- k(n, 1), r(f, n), r(f, m), k(m, 2).\n.output h\n").unwrap();
    let rule = &p.rules[0];
    // 1,000 functions of 30 nodes with 5 calls, and 2 of 2,000 nodes with 50: 2% of the calls, 56% of the work.
    let (mut r, mut k): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
    let mut add = |f: i64, nodes: i64, calls: i64| for i in 0..nodes {
        r.push([f, f * 10_000 + i]);
        k.push([f * 10_000 + i, if i < calls { 1 } else { 2 }]);
    };
    for f in 1..=1000 { add(f, 30, 5) }
    for f in [5001, 5002] { add(f, 2000, 50) }
    // Work by step after the lead: each call's function, each call's function's nodes, and their kinds.
    let truth = 5100.0 + 5100.0 + (1000.0 * 5.0 * 30.0 + 2.0 * 50.0 * 2000.0);
    let rel = |rows: &[[i64; 2]]| InRel::from_flat(rows.iter().flatten().copied().collect(), rows.len(), vec![b'i'; 2]);
    // The function column's heavy values: both large functions, and some of the rest (their rows tie).
    let heavy: Vec<i64> = [5001, 5002].into_iter().chain(1..=10).collect();
    let snap = WalkSnap { rels: [("r", rel(&r)), ("k", rel(&k))].into_iter().map(|(n, x)| (n.to_owned(), x)).collect(), syms: HashMap::new(), delta: None, heavy: [(("r".to_owned(), 0), heavy)].into_iter().collect() };
    let order: Vec<Step> = [0usize, 1, 2, 3].iter().map(|&at| Step { at, mode: None }).collect();
    let st = snap.starts(rule, &order, None).expect("measured");
    let (hot, n) = st.hot.as_ref().expect("split by the function's heavy values");
    // The heavy values' calls: the large functions' 100, and 5 for each of the 10 others.
    assert!((n / 150.0 - 1.0).abs() < 0.15, "{n} hot calls, {} drawn", hot.len());
    assert!(hot.len() >= 64, "{} hot rows to walk", hot.len());
    let found = snap.walk(rule, None, &order, vec![(order.clone(), 1.0)], &|| false);
    let walked = found.walked.expect("walked");
    let plain = WalkSnap { heavy: HashMap::new(), ..snap };
    let unsplit = plain.walk(rule, None, &order, vec![(order.clone(), 1.0)], &|| false).walked.expect("walked");
    // Split, 1.04 of the work; drawn whole, 1.76: the uniform draw's hot rows each stand for a large function.
    assert!((walked / truth - 1.0).abs() < 0.1, "{walked} against {truth}");
    assert!((unsplit / truth - 1.0).abs() > 3.0 * (walked / truth - 1.0).abs(), "unsplit {unsplit}, split {walked}, against {truth}");
}

fn steps_of(o: &[usize]) -> Vec<StoredStep> {
    o.iter().map(|&at| (at as u16, None)).collect()
}
