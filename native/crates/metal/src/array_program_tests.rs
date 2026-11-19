//! The device's runs equal the interpreter's (`ArrayProgram::values_at`): each operation over
//! lengths around a tile's (256) and past a second level of tiles, then the refusals at the
//! same operation.

use super::{Job, run, run_batch};
use rerail_compiler::array::{ArrayProgram, Calls, Fun};

/// A program from its rows (`array N` dropped): the first is its shape.
fn program(rows: &str) -> ArrayProgram {
    let mut p = ArrayProgram::default();
    for row in rows.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let words: Vec<&str> = row.split_whitespace().collect();
        p.row(&words).unwrap_or_else(|e| panic!("{row}: {e}"));
    }
    p
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// `n` values in `lo..hi`.
    fn column(&mut self, n: usize, lo: i64, hi: i64) -> Vec<i64> {
        (0..n).map(|_| lo + (self.next() % (hi - lo) as u64) as i64).collect()
    }
}

/// Each function: a row of (x, x * 2) `x mod 3` times.
struct Rows;

impl Calls for Rows {
    fn call(&mut self, _: u16, _: &Fun, args: &[i64], env: &[&[i64]], out: &mut Vec<i64>) -> Result<(), &'static str> {
        let k = args[0].rem_euclid(3) + env.first().map_or(0, |e| e.len() as i64 % 2);
        for _ in 0..k {
            out.extend([args[0], args[0] * 2]);
        }
        Ok(())
    }
}

const LENGTHS: [usize; 9] = [0, 1, 2, 7, 255, 256, 257, 1000, 70_001];

/// `p` on the device and on the interpreter: the same outputs, or the same refusal.
fn same(p: &ArrayProgram, inputs: &[&[i64]], params: &[i64]) {
    let want = p.values_at(inputs, params, &mut Rows).map(|v| p.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect::<Vec<_>>());
    let got = run(p, inputs, params, &mut Rows).expect("a device").0;
    check(&want, &got);
}

fn check(want: &super::Outcome, got: &super::Outcome) {
    match (want, got) {
        (Ok(w), Ok(g)) => {
            for (k, (w, g)) in w.iter().zip(g).enumerate() {
                if w != g {
                    let at = w.iter().zip(g).position(|(a, b)| a != b);
                    panic!("output {k} (lengths {} and {}): first difference at {at:?}: {:?} and {:?}", w.len(), g.len(), at.map(|j| w[j]), at.and_then(|j| g.get(j)));
                }
            }
        }
        _ => assert_eq!(want, got),
    }
}

/// Programs over a batch of units of every length (and some refused): each unit's outcome
/// its own run's on the interpreter.
#[test]
fn a_batch_equals_each_unit_alone() {
    let p = program(
        "shape 2 1
        fun f0 host rows fields=2
        %0 = input 0
        %1 = input 1
        %2 = param 0
        %3 = map add %0 %2
        %4 = scan add excl %3 seg %1
        %5 = sort %0
        %6 = gather %0 %5
        %7 = search le %6 %3
        %8 = len %0
        %9 = const 0
        %10 = scatter max %0 %1 %8 %9
        %11 = compact %1
        %12 = expand %1
        %13 = concat %11 %12
        %14 = call f0 %0 env %13
        %15 = part %14 1
        %16 = gather %0 %13
        %17 = carry %1
        %18 = gather %17 %17
        %19 = sort %1 %0
        rounds %17 %18
        output %19
        output %4
        output %5
        output %7
        output %10
        output %13
        output %15
        output %16
        output %18
        ",
    );
    let mut r = Rng(23);
    let mut units: Vec<(Vec<i64>, Vec<i64>, i64)> = Vec::new();
    for (k, n) in [0usize, 1, 2, 7, 255, 256, 257, 1000, 3000, 1, 0, 40].into_iter().enumerate() {
        let x = r.column(n, -50, 50);
        // Parents before each node: a forest, its roots by pointer jumping; in unit 9 a node
        // past the end, refused by its gather.
        let mut parent: Vec<i64> = (0..n).map(|j| if j == 0 { 0 } else { (r.next() % j as u64) as i64 }).collect();
        if k == 11 {
            parent[20] = 40;
        }
        units.push((x, parent, k as i64 - 3));
    }
    let want: Vec<super::Outcome> = units
        .iter()
        .map(|(x, f, k)| p.values_at(&[x, f], &[*k], &mut Rows).map(|v| p.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect()))
        .collect();
    assert!(want.iter().any(Result::is_err) && want.iter().filter(|w| w.is_ok()).count() > 8, "{:?}", want.iter().map(|w| w.as_ref().err()).collect::<Vec<_>>());
    let mut hosts: Vec<Rows> = units.iter().map(|_| Rows).collect();
    let mut jobs: Vec<Job> = units.iter().zip(hosts.iter_mut()).map(|((x, f, k), h)| Job { inputs: vec![x, f], params: vec![*k], calls: h }).collect();
    let (got, run) = run_batch(&p, &mut jobs).expect("a device");
    for (u, (w, g)) in want.iter().zip(&got).enumerate() {
        eprintln!("unit {u}: {:?}", w.as_ref().err());
        check(w, g);
    }
    // One batch: the syncs of one unit's run, not of twelve.
    let (_, one) = super::run(&p, &[&units[8].0, &units[8].1], &[units[8].2], &mut Rows).expect("a device");
    assert!(run.syncs <= one.syncs + 2, "{} syncs a batch, {} a unit", run.syncs, one.syncs);
}

#[test]
fn maps_and_selects_equal_the_interpreter() {
    let mut r = Rng(7);
    for n in LENGTHS {
        let (x, y, m) = (r.column(n, -1 << 40, 1 << 40), r.column(n, -70, 70), r.column(n, 0, 2));
        for (op, name) in rerail_compiler::array::MapOp::ALL {
            let _ = op;
            same(&program(&format!("shape 3 1\n%0 = input 0\n%1 = input 1\n%2 = map {name} %0 %1\n%3 = param 0\n%4 = map {name} %2 %3\noutput %2\noutput %4\n")), &[&x, &y, &m], &[3]);
        }
        same(&program("shape 3 0\n%0 = input 0\n%1 = input 1\n%2 = input 2\n%3 = select %2 %0 %1\n%4 = const 9\n%5 = select %2 %4 %0\n%6 = iota %0\n%7 = len %0\noutput %3\noutput %5\noutput %6\noutput %7\n"), &[&x, &y, &m], &[]);
    }
}

#[test]
fn scans_equal_the_interpreter() {
    let mut r = Rng(11);
    for n in LENGTHS {
        let (x, f, g) = (r.column(n, -1000, 1000), r.column(n, 0, 2), r.column(n, 0, 1000).iter().map(|&v| i64::from(v == 0)).collect::<Vec<_>>());
        for how in ["add", "max", "min"] {
            for excl in ["", "excl "] {
                same(&program(&format!("shape 3 0\n%0 = input 0\n%1 = input 1\n%2 = input 2\n%3 = scan {how} {excl}%0\n%4 = scan {how} {excl}%0 seg %1\n%5 = scan {how} {excl}%0 seg %2\noutput %3\noutput %4\noutput %5\n")), &[&x, &f, &g], &[]);
            }
        }
    }
}

#[test]
fn sorts_gathers_and_searches_equal_the_interpreter() {
    let mut r = Rng(13);
    for n in LENGTHS {
        let (k, l, q) = (r.column(n, -1 << 50, 1 << 50), r.column(n, -3, 3), r.column(n, -4, 4));
        let p = program(
            "shape 3 0
            %0 = input 0
            %1 = input 1
            %2 = input 2
            %3 = sort %0
            %4 = sort %1 %0
            %5 = sort %1
            %6 = gather %1 %5
            %7 = search le %6 %2
            %8 = search lt %6 %2
            %9 = gather %0 %4
            output %3
            output %4
            output %5
            output %7
            output %8
            output %9
            ",
        );
        same(&p, &[&k, &l, &q], &[]);
    }
}

#[test]
fn scatters_compacts_expands_and_concats_equal_the_interpreter() {
    let mut r = Rng(17);
    for n in LENGTHS {
        let (s, i, c) = (r.column(n, -1000, 1000), r.column(n, -5, n as i64 + 5), r.column(n, 0, 4));
        // A permutation: a set scatter's indexes, each written once.
        let mut perm: Vec<i64> = (0..n as i64).collect();
        for j in (1..n).rev() {
            perm.swap(j, (r.next() % (j as u64 + 1)) as usize);
        }
        for how in ["add", "max", "min"] {
            same(&program(&format!("shape 3 1\n%0 = input 0\n%1 = input 1\n%2 = param 0\n%3 = const -3\n%4 = scatter {how} %0 %1 %2 %3\n%5 = const 1\n%6 = scatter {how} %5 %1 %2 %3\noutput %4\noutput %6\n")), &[&s, &i, &c], &[n as i64 + 2]);
        }
        same(&program("shape 3 0\n%0 = input 0\n%1 = input 2\n%2 = len %0\n%3 = const 0\n%4 = scatter set %0 %1 %2 %3\noutput %4\n"), &[&s, &c, &perm], &[]);
        same(&program("shape 3 0\n%0 = input 0\n%1 = input 1\n%2 = input 2\n%3 = map sub %2 %2\n%4 = compact %2\n%5 = compact %3\n%6 = expand %2\n%7 = concat %0 %6\n%8 = concat %5 %1\noutput %4\noutput %5\noutput %6\noutput %7\noutput %8\n"), &[&s, &i, &c], &[]);
    }
}

#[test]
fn calls_and_rounds_equal_the_interpreter() {
    let mut r = Rng(19);
    for n in LENGTHS {
        let x = r.column(n, -10, 10);
        // A forest: each node's parent before it; its roots by pointer jumping.
        let parent: Vec<i64> = (0..n).map(|j| if j == 0 || r.next() % 5 == 0 { j as i64 } else { (r.next() % j as u64) as i64 }).collect();
        same(
            &program(
                "shape 2 0
                fun f0 host rows fields=2
                %0 = input 0
                %1 = input 1
                %2 = call f0 %0
                %3 = part %2 0
                %4 = part %2 1
                %5 = call f0 %0 env %1
                %6 = carry %1
                %7 = gather %6 %6
                rounds %6 %7
                output %2
                output %3
                output %4
                output %5
                output %7
                ",
            ),
            &[&x, &parent],
            &[],
        );
    }
}

#[test]
fn refusals_are_at_the_interpreters_operation() {
    let x: Vec<i64> = (0..600).collect();
    let mut bad = x.clone();
    bad[400] = 600;
    same(&program("shape 2 0\n%0 = input 0\n%1 = input 1\n%2 = gather %0 %1\noutput %2\n"), &[&x, &bad], &[]);
    let twice: Vec<i64> = x.iter().map(|&j| j.min(500)).collect();
    same(&program("shape 2 0\n%0 = input 0\n%1 = input 1\n%2 = len %0\n%3 = const 0\n%4 = scatter set %0 %1 %2 %3\noutput %4\n"), &[&x, &twice], &[]);
    let mut unsorted = x.clone();
    unsorted.swap(3, 300);
    same(&program("shape 2 0\n%0 = input 0\n%1 = input 1\n%2 = search le %1 %0\noutput %2\n"), &[&x, &unsorted], &[]);
    let mut negative = vec![1i64; 600];
    negative[599] = -1;
    same(&program("shape 1 0\n%0 = input 0\n%1 = expand %0\n%2 = gather %0 %1\noutput %2\n"), &[&negative], &[]);
    // A device refusal before a host one: the earlier operation's.
    same(&program("shape 2 0\n%0 = input 0\n%1 = input 1\n%2 = gather %0 %1\n%3 = compact %2\n%4 = map add %0 %3\noutput %4\n"), &[&x, &bad], &[]);
}

/// Many small units, keys with many equals: each unit's sorts stable and its own.
#[test]
fn a_batch_of_many_units_sorts_each_stably() {
    let p = program(
        "shape 2 0
        %0 = input 0
        %1 = input 1
        %2 = sort %0
        %3 = sort %0 %1
        %4 = sort %1
        %5 = gather %0 %4
        %6 = scan add %5 seg %1
        %7 = compact %1
        %8 = gather %0 %7
        %9 = sort %8
        output %2
        output %3
        output %4
        output %6
        output %9
        ",
    );
    let mut r = Rng(29);
    for round in 0..4 {
        let units: Vec<(Vec<i64>, Vec<i64>)> = (0..60).map(|k| {
            let n = (r.next() % [5, 40, 300, 700][round]) as usize + usize::from(k % 7 == 0) * 257;
            (r.column(n, 0, 4), r.column(n, 0, 2))
        }).collect();
        let want: Vec<super::Outcome> = units.iter().map(|(x, f)| p.values_at(&[x, f], &[], &mut ()).map(|v| p.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect())).collect();
        let mut hosts: Vec<()> = vec![(); units.len()];
        let mut jobs: Vec<Job> = units.iter().zip(hosts.iter_mut()).map(|((x, f), h)| Job { inputs: vec![x, f], params: vec![], calls: h }).collect();
        let (got, _) = run_batch(&p, &mut jobs).expect("a device");
        for (u, (w, g)) in want.iter().zip(&got).enumerate() {
            if w != g {
                eprintln!("round {round} unit {u} of {} (length {})", units.len(), units[u].0.len());
            }
            check(w, g);
        }
    }
}
