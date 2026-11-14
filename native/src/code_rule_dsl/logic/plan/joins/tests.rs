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
    };
    let key: PlanKey = (5u128 << 70) | 9;
    assert_eq!(of_note_row(&note_row(key, &n)), Some((key, n)));
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
    let line = |stratum: &str, key: PlanKey, work: f64, est: f64| LineRun { stratum: stratum.into(), key, rule: key as u64, order: vec![(0, None), (1, None)], work, est };
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
    critical(&["s".into()], &[vec![]], &[LineRun { stratum: "s".into(), key: 200, rule: 1, order: steps(&order(&[0, 1])), work: 2e6, est: 1e5 }]);
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
    critical(&["s".into()], &[vec![]], &[LineRun { stratum: "s".into(), key: 300, rule: 1, order: steps(&order(&[0, 1, 2])), work: 1e6, est: 1e6 }]);
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
/// noted with its best order measured far over its price takes the other at its next plan.
#[test]
fn exact_statistics_and_the_memo_change_the_plan() {
    let _serial = SERIAL.lock();
    clear();
    let p = parse(".decl a(x: number, y: number)\n.decl b(y: number, z: number)\n.decl h(x: number, z: number)\nh(x, z) :- a(x, y), b(y, z).\n.output h\n").unwrap();
    let inputs = Inputs { facts: "main.".into(), stats: "main.".into(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: HashMap::new(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    for (rel, rows) in [("a", 50.0), ("b", 100.0)] {
        model.stats.card.insert(rel.into(), rows);
        for c in 0..2 {
            model.ndv.insert((rel.into(), c), rows);
            model.stats.fan.insert((rel.into(), c), 1.0);
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
    critical(&["h".into()], &[vec![]], &[LineRun { stratum: "h".into(), key, rule: b.observed_key(), order: steps(&best), work: 1e6, est: 100.0 }]);
    assert!(pending(&key), "a wider search is wanted");
    let ((order, _, _), _) = model.plan_rule_reused(&p, &b, None).unwrap();
    assert_eq!(lead(&order), "b", "corrected by what `a` first measured");
    let n = note(&key).unwrap();
    assert!(!n.wants_search && !n.proposals.is_empty(), "the line's work paid for a wider search, made inline: {n:?}");
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
    request(400, 1.0, Box::new(|| vec![(vec![(1, None), (0, None)], 5.0), (vec![(0, None), (1, None)], 3.0)]));
    assert_eq!(requests(), 1);
    assert!(!wants_request(&400), "one request a line");
    assert!(search_one());
    assert!(!search_one());
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
