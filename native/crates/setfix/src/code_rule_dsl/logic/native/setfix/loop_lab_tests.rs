//! A lab of the join's own loop (decision 0640, "What follows"; t56 "What the per-file joins spend now"): what a
//! step costs a row it reads, over derived relations as `all`'s per-file strata read them (sealed rows, the chains
//! their waves kept, an order made at a first probe), against the reads alone done by hand.
//!
//! ```text
//! cargo nextest run --cargo-profile profiling --lib -p rerail-setfix --features test-hooks --no-capture \
//!     --run-ignored only -E 'test(/a_joins_candidates_cost/)'
//! ```
use super::spec::hex;
use super::*;

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db
}

/// A relation derived from `rows` through a run, as a per-file stratum's inputs are: its rows sealed, its wave's chains
/// kept.
fn derived(db: &Connection, name: &str, rows: &[[i64; 2]]) -> Arc<InRel> {
    let mut r = Rel::new("x", b"ii");
    for row in rows {
        r.push(row);
    }
    let input = format!("{name}_in");
    adopt(db, &input, Arc::new(InRel::moved(r)));
    let body = format!("pos r:{} v0 v1\nhead v0 v1", hex(&input));
    open(db, &format!("name {name}\nrel {name} ii empty\nnowrite\nseed 0 {}", hex(&body))).unwrap();
    run(db, name, 0).unwrap();
    close(db, name).unwrap();
    held(db, name).expect("the derived relation")
}

/// A spread of `0..m` (a multiplicative hash): keys that are not in order, as a join's probes mostly are not.
fn spread(i: i64, m: i64) -> i64 {
    ((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) as i64 % m
}

/// Runs `body` as a component's one join, profiled, and prints each step's CPU a row in and a row out.
fn steps(db: &Connection, name: &str, body: &str, what: &str) {
    let paths = || run_counts().index_paths.iter().map(|x| x.load(AtomicOrdering::Relaxed)).collect::<Vec<u64>>();
    let before = paths();
    runs::probe_reads::take();
    open(db, &format!("name {name}\nrel {name} ii empty\nnowrite\nseed 0 {}", hex(body))).unwrap();
    run(db, name, 0).unwrap();
    let counts = step_counts(db, name).unwrap();
    let cpu = step_cpu(db, name).unwrap();
    close(db, name).unwrap();
    let opened: Vec<String> = paths().iter().zip(&before).enumerate().filter(|(_, (a, b))| a > b).map(|(i, (a, b))| format!("path {i} x{}", a - b)).collect();
    println!("{what:<34} indexes opened: {}", opened.join(", "));
    let [probes, rows, marks] = runs::probe_reads::take();
    println!("{what:<34} a run's probe: {:.2} rows compared, {:.2} tags or fences read ({probes} probes)", rows as f64 / probes.max(1) as f64, marks as f64 / probes.max(1) as f64);
    for (w, (c, t)) in counts.iter().zip(&cpu).enumerate() {
        for (s, &(rows_in, rows_out)) in c.iter().enumerate() {
            let ns = t[s] * 1e9;
            println!("{what:<34} walk {w} step {s}: {rows_in:>9} in {rows_out:>9} out  {:>7.1} ns a row in  {:>7.1} ns a row out", ns / rows_in.max(1) as f64, ns / rows_out.max(1) as f64);
        }
        let head = t[c.len()] * 1e9;
        println!("{what:<34} walk {w} head's rows: {:>7.1} ns a row", head / c.last().map_or(1, |x| x.1.max(1)) as f64);
    }
}

#[test]
#[ignore = "a measurement: run alone, --no-capture"]
#[allow(non_snake_case)]
fn a_joins_candidates_cost() {
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    // `LAB_N` outer rows, `LAB_M` keys (a scale past the caches: a chunk's node relations hold about a million rows).
    let size = |v: &str, d: i64| std::env::var(v).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (n, m) = (size("LAB_N", 200_000), size("LAB_M", 50_000));
    let (N, M) = (n, m);
    let db = connection();
    // The outer rows: (a, b), b spread over 0..M.
    let outer: Vec<[i64; 2]> = (0..N).map(|a| [a, spread(a, M)]).collect();
    // Half of them held again (a membership of both columns, `kind / membership on (c0, c1)`).
    let mem: Vec<[i64; 2]> = outer.iter().filter(|r| r[0] % 2 == 0).copied().collect();
    // Each b once (a lookup's one row, `docof / lookup on (n)`), and eight times (`kind / lookup on (c0)`).
    let one: Vec<[i64; 2]> = (0..M).map(|b| [b, b + 7]).collect();
    let eight: Vec<[i64; 2]> = (0..M * 8).map(|i| [i % M, i]).collect();
    let (o, mm, on1, on8) = (derived(&db, "lab_outer", &outer), derived(&db, "lab_mem", &mem), derived(&db, "lab_one", &one), derived(&db, "lab_eight", &eight));
    let r = |t: &str| format!("r:{}", hex(t));
    let cases = [
        ("membership on (c0, c1)", format!("pos {} v0 v1\npos {} v0 v1\nhead v0 v1", r("lab_outer"), r("lab_mem"))),
        ("lookup on (c0), one row", format!("pos {} v0 v1\npos {} v1 v2\nhead v0 v2", r("lab_outer"), r("lab_one"))),
        ("lookup on (c0), eight rows", format!("pos {} v0 v1\npos {} v1 v2\nhead v0 v2", r("lab_outer"), r("lab_eight"))),
    ];
    // `LAB_ROUNDS` (a sample's): more rounds; `LAB_CASE`: one case alone.
    let rounds: usize = std::env::var("LAB_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let only: Option<usize> = std::env::var("LAB_CASE").ok().and_then(|v| v.parse().ok());
    let mut n = 0;
    for round in 0..rounds {
        let profiled = round < 2;
        crate::code_rule_dsl::logic::profile::set(profiled);
        // Counted in the first round alone: the counts are shared words, which slow every probe.
        runs::probe_reads::ON.store(round == 0, AtomicOrdering::Relaxed);
        if round < 3 {
            println!("-- round {round}{}", if profiled { "" } else { ", unprofiled: the whole run, wall and CPU, a row of the outer" });
        }
        for (c, (what, body)) in cases.iter().enumerate() {
            if only.is_some_and(|o| o != c) {
                continue;
            }
            n += 1;
            let name = format!("lab_{n}");
            if profiled {
                steps(&db, &name, body, what);
                continue;
            }
            let (t, c) = (crate::code_rule_dsl::logic::profile::now(), rerail_prof::process_cpu().as_secs_f64());
            open(&db, &format!("name {name}\nrel {name} ii empty\nnowrite\nseed 0 {}", hex(body))).unwrap();
            run(&db, &name, 0).unwrap();
            close(&db, &name).unwrap();
            if round < 3 {
                let (wall, cpu) = (crate::code_rule_dsl::logic::profile::now() - t, rerail_prof::process_cpu().as_secs_f64() - c);
                println!("{what:<34} {:>7.1} ns a row of the outer, {:>7.1} ns of CPU", wall * 1e9 / N as f64, cpu * 1e9 / N as f64);
            }
        }
    }
    crate::code_rule_dsl::logic::profile::set(false);
    // The floor: the same reads by hand, through the rows' own read (`InRel::at`, a sealed chunk's decode a value).
    let time = |what: &str, rows: usize, f: &dyn Fn() -> i64| {
        let t = crate::code_rule_dsl::logic::profile::now();
        let s = std::hint::black_box(f());
        println!("{what:<34} {:>7.1} ns a row (sum {s})", (crate::code_rule_dsl::logic::profile::now() - t) * 1e9 / rows as f64);
    };
    time("by hand: outer's rows read", o.len(), &|| (0..o.len()).map(|i| { let r = o.at(i); r.at(0) ^ r.at(1) }).fold(0, i64::wrapping_add));
    time("by hand: a row of mem read", o.len(), &|| (0..o.len()).map(|i| mm.at(i % mm.len()).at(1)).fold(0, i64::wrapping_add));
    time("by hand: a row of one read at b", o.len(), &|| (0..o.len()).map(|i| on1.at(spread(i as i64, M) as usize).at(1)).fold(0, i64::wrapping_add));
    time("by hand: a row of eight read", on8.len(), &|| (0..on8.len()).map(|i| on8.at(i).at(1)).fold(0, i64::wrapping_add));
    // The one-row lookup's step taken apart, on one thread: the outer's keys sorted (as the scan orders them for the
    // step), probing `lab_one` through each layer in turn.
    let mut keys: Vec<i64> = (0..o.len()).map(|i| o.at(i).at(1)).collect();
    keys.sort_unstable();
    let ix = on1.index(&[0]);
    let order = ix.runs(&on1).expect("an order of runs");
    let store = Store { rels: Vec::new(), syms: symbols(&db).unwrap() };
    // The one-row lookup's join, its second step's source: `lab_one` by its first column.
    let spec = JoinSpec::parse(&cases[1].1).unwrap().encode();
    let mut syms = Interner::for_loading(&store.syms);
    let mut j = join_rule_with(&db, &spec, &mut syms, false).unwrap();
    j.indexed();
    let src = match &j.steps[1] { Step::Atom { src, .. } => src.clone(), _ => panic!("a probe of lab_one") };
    let spec = [(0usize, Key::Slot(0))];
    let need = 0b11u64;
    time("parts: the order's search (find_at)", keys.len(), &|| {
        let mut hints = [0u32; runs::HINTS];
        keys.iter().map(|&k| order.find_at(&on1.rows, on1.k, &[k], None, &mut hints).len() as i64).sum()
    });
    time("parts: candidates", keys.len(), &|| {
        let (mut hints, mut last) = ([0u32; runs::HINTS], LastProbe::default());
        keys.iter().map(|&k| { let (_, c) = cands::candidates(&j, &src, &store, &[k], &spec, &mut hints, &mut last, need, None, false); i64::from(c.any(|_| false)) + 1 }).sum()
    });
    time("parts: candidates, ids walked", keys.len(), &|| {
        let (mut hints, mut last) = ([0u32; runs::HINTS], LastProbe::default());
        keys.iter().map(|&k| { let (_, c) = cands::candidates(&j, &src, &store, &[k], &spec, &mut hints, &mut last, need, None, false); let mut s = 0i64; c.any(|id| { s += i64::from(id); false }); s }).sum()
    });
    time("parts: candidates, rows read (each)", keys.len(), &|| {
        let (mut hints, mut last) = ([0u32; runs::HINTS], LastProbe::default());
        keys.iter().map(|&k| {
            let (rows, c) = cands::candidates(&j, &src, &store, &[k], &spec, &mut hints, &mut last, need, None, false);
            let mut s = 0i64;
            rows.each(c, need, &mut LastProbe::default(), |_, row| { if row.at(0) == k { s += row.at(1) } false });
            s
        }).sum()
    });
}
