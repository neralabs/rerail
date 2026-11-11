//! Tier 2's monoids and the Metal plan verifier, pinned (docs/parser-generators/17-tier-2-tdd-plan.md):
//! the monoids' laws and references (phase 2), and the plan verifier (phase 4).

use super::block_monoid::{Dyck, First, Last, StateFn};

/// A small xorshift: the tests' random values, the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// --- Phase 2: the monoids -----------------------------------------------------------------

/// A random function from 2 state bits to 2 state bits and 2 halo bits, as S1's (doc 16 §4.2).
fn random_fn(r: &mut Rng) -> StateFn {
    let table = r.next() & 0xffff;
    StateFn::from(2, 4, |x| (table >> (4 * x)) & 15)
}

/// The same function on every full state.
fn same(a: StateFn, b: StateFn) -> bool {
    (0..16).all(|x| a.apply(x) == b.apply(x))
}

#[test]
fn state_fn_laws() {
    let mut r = Rng(1);
    let id = StateFn::identity(2, 4);
    for _ in 0..500 {
        let (a, b, c) = (random_fn(&mut r), random_fn(&mut r), random_fn(&mut r));
        assert!(same(id.then(a), a) && same(a.then(id), a), "identity: {a:?}");
        assert!(same(a.then(b).then(c), a.then(b.then(c))), "associative: {a:?} {b:?} {c:?}");
    }
    assert_eq!(id.apply(0b1110), 0b1110, "the identity keeps the halos coming in");
}

/// Blocks' functions composed in any grouping give the state a walk over the blocks gives, at
/// each block's start: the state bits, and the halos of the block before.
#[test]
fn state_fn_composes_like_a_walk() {
    let mut r = Rng(2);
    for _ in 0..200 {
        let n = 1 + r.below(40) as usize;
        let fs: Vec<StateFn> = (0..n).map(|_| random_fn(&mut r)).collect();
        let start = r.below(4);
        let mut walk = vec![start];
        for f in &fs {
            let x = *walk.last().expect("a state");
            walk.push(f.apply(x));
        }
        let mut prefix = StateFn::identity(2, 4);
        for (i, f) in fs.iter().enumerate() {
            assert_eq!(prefix.apply(start), walk[i], "block {i}");
            prefix = prefix.then(*f);
        }
        assert_eq!(prefix.apply(start), walk[n]);
        // Combined as a tree, as a SIMD group does.
        let mut level = fs.clone();
        while level.len() > 1 {
            level = level.chunks(2).map(|p| if p.len() == 2 { p[0].then(p[1]) } else { p[0] }).collect();
        }
        assert_eq!(level[0].apply(start), walk[n]);
    }
}

/// An event of the stack: brackets of two kinds, keys and commas.
#[derive(Clone, Copy, Debug)]
enum Event {
    Open(u8),
    Close(u8),
    Key,
    Comma,
}

fn random_events(r: &mut Rng, n: usize) -> Vec<Event> {
    let mut depth = 0i32;
    (0..n)
        .map(|_| match r.below(10) {
            0..=2 if depth < 15 => {
                depth += 1;
                Event::Open(r.below(2) as u8)
            }
            3..=5 => {
                depth -= 1;
                Event::Close(r.below(2) as u8)
            }
            6 | 7 => Event::Key,
            _ => Event::Comma,
        })
        .collect()
}

fn summary(events: &[Event]) -> Dyck {
    events.iter().fold(Dyck::ID, |d, e| {
        d.then(match *e {
            Event::Open(k) => Dyck::open(k),
            Event::Close(k) => Dyck::close(k),
            Event::Key => Dyck::key(),
            Event::Comma => Dyck::comma(),
        })
    })
}

#[test]
fn dyck_laws() {
    let mut r = Rng(3);
    for _ in 0..500 {
        // Few enough events that no summary passes 13 levels: past that, a file fails anyway.
        let n = [r.below(5) as usize, r.below(5) as usize, r.below(5) as usize];
        let (a, b, c) = (summary(&random_events(&mut r, n[0])), summary(&random_events(&mut r, n[1])), summary(&random_events(&mut r, n[2])));
        assert_eq!(Dyck::ID.then(a), a);
        assert_eq!(a.then(Dyck::ID), a);
        assert_eq!(a.then(b).then(c), a.then(b.then(c)), "{a:?} {b:?} {c:?}");
    }
}

/// The stack a walk gives: its kinds (innermost last), whether the innermost object's pair is
/// open, and whether a close met the wrong kind, an empty stack, or the stack went past 13.
fn walk(events: &[Event]) -> (Vec<u8>, bool, bool) {
    let (mut stack, mut pair, mut error) = (Vec::new(), false, false);
    for e in events {
        match *e {
            Event::Open(k) => {
                stack.push(k);
                pair = false;
                error |= stack.len() > 13;
            }
            Event::Close(k) => {
                error |= stack.pop() != Some(k);
                pair = true;
            }
            Event::Key => pair = true,
            Event::Comma => pair = false,
        }
    }
    (stack, pair, error)
}

/// Summaries of random pieces, combined, equal the whole's summary; and the whole's, from an
/// empty stack, equals a walk: its depth and kinds, its pair bit, its error. A file fails when a
/// close meets the wrong kind or no open, or when it nests past 13 anywhere: the summary's
/// `peak` sees a nesting inside a piece that the piece's final opens do not.
#[test]
fn dyck_equals_a_stack() {
    let mut r = Rng(4);
    let mut errors = 0;
    for _ in 0..2000 {
        let n = r.below(60) as usize;
        let events = random_events(&mut r, n);
        let whole = summary(&events);
        let mut combined = Dyck::ID;
        let mut at = 0;
        while at < events.len() {
            let to = (at + 1 + r.below(8) as usize).min(events.len());
            combined = combined.then(summary(&events[at..to]));
            at = to;
        }
        let (stack, pair, error) = walk(&events);
        assert_eq!(whole.fails(13), error, "{events:?}: {whole:?}");
        assert_eq!(combined.fails(13), error, "{events:?}: {combined:?}");
        if !error {
            assert_eq!(combined, whole, "{events:?}");
            assert_eq!(usize::from(whole.opens()), stack.len(), "{events:?}");
            let kinds: Vec<u8> = (0..whole.opens()).rev().map(|i| whole.open_kind(i)).collect();
            assert_eq!(kinds, stack, "innermost first in the summary, last in the walk");
            if !events.is_empty() {
                assert_eq!(whole.pair(), pair, "{events:?}");
            }
        } else {
            errors += 1;
        }
    }
    assert!(errors > 100 && errors < 1900, "both kinds of case: {errors} errors");
}

/// `last` gives the latest value present before each place, and `first` the earliest after.
#[test]
fn last_and_first_equal_a_walk() {
    let mut r = Rng(5);
    for _ in 0..300 {
        let n = r.below(30) as usize;
        let xs: Vec<Option<u64>> = (0..n).map(|_| if r.below(3) == 0 { Some(r.below(100)) } else { None }).collect();
        let mut last = Last(None);
        for (i, x) in xs.iter().enumerate() {
            assert_eq!(last.0, xs[..i].iter().rev().find_map(|x| *x));
            last = last.then(Last(*x));
        }
        let mut first = First(None);
        for (i, x) in xs.iter().enumerate().rev() {
            assert_eq!(first.0, xs[i + 1..].iter().find_map(|x| *x));
            first = First(*x).then(first);
        }
        // Associative, as every monoid.
        let (a, b, c) = (Last(xs.first().copied().flatten()), Last(xs.get(1).copied().flatten()), Last(xs.get(2).copied().flatten()));
        assert_eq!(a.then(b).then(c), a.then(b.then(c)));
    }
}

// --- Phase 4: the plan's verifier ---------------------------------------------------------
//
// The plans are made again for Metal from the tree section's state machine (doc 18 §5); until
// then the verifier is pinned on a plan built here, in the shape of the old one: pass A writes
// summaries through a staging store, the scan exchanges them, pass B writes the tape.

use super::block_plan::{BUDGET, Mem, Plan, Step, Store, Target, device};

fn work(what: &str, simd: bool, reads: &[&str], writes: &[&str]) -> Step {
    Step::Work {
        what: what.into(),
        simd,
        reads: reads.iter().map(|s| (*s).to_owned()).collect(),
        writes: writes.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn threadgroup(name: &str, bytes: u32, double: bool) -> Store {
    Store { name: name.into(), mem: Mem::Threadgroup, bytes, double, note: String::new() }
}

/// Two passes and a scan, with every barrier they need.
fn small_plan() -> Plan {
    let tiles = |what: &str, body: Vec<Step>| Step::Loop { what: what.into(), uniform: true, by: "tiles".into(), body };
    let a = vec![
        work("summaries", true, &["bytes"], &["staging"]),
        Step::Barrier(Mem::Threadgroup),
        work("flush", false, &["staging"], &["entries"]),
        Step::Barrier(Mem::Threadgroup),
    ];
    let scan = vec![
        work("scan entries", true, &["entries"], &["exchange"]),
        Step::Barrier(Mem::Threadgroup),
        work("scan totals", true, &["exchange"], &["prefixes"]),
    ];
    let b = vec![work("tape", true, &["bytes", "prefixes"], &["tape"])];
    Plan {
        target: Target::Gpu,
        candidates: 1,
        s1_bits: 0,
        fields: Vec::new(),
        cand_words: 1,
        stores: vec![
            device("bytes"),
            device("entries"),
            device("prefixes"),
            device("tape"),
            threadgroup("staging", 2048, false),
            // Double: a round's scan writes the other buffer, so it needs no barrier after the
            // last round's reads.
            threadgroup("exchange", 2 * 128, true),
        ],
        steps: vec![
            tiles("pass A", a),
            Step::Barrier(Mem::Device),
            tiles("scan", scan),
            Step::Barrier(Mem::Device),
            tiles("pass B", b),
        ],
        budget: BUDGET,
    }
}

#[test]
fn plan_verifier_accepts_a_whole_plan() {
    let p = small_plan();
    p.verify().unwrap_or_else(|e| panic!("{e}\n{}", p.text()));
}

/// Every barrier of the plan is needed: without any one, a store is read before another step's
/// write is visible, or rewritten while another step may still read it.
#[test]
fn plan_verifier_refuses_a_missing_barrier() {
    let p = small_plan();
    let n = p.barriers();
    assert_eq!(n, 5);
    for k in 0..n {
        let cut = p.without_barrier(k);
        let e = cut.verify().expect_err(&format!("barrier {k} removed: accepted\n{}", cut.text()));
        assert!(e.contains("barrier"), "{e}");
    }
}

/// A double store needs no barrier between one iteration's reads and the next one's write; a
/// single one does.
#[test]
fn plan_verifier_models_double_buffers() {
    let mut p = small_plan();
    let exchange = p.stores.iter_mut().find(|s| s.name == "exchange").expect("exchange");
    exchange.double = false;
    assert!(p.verify().expect_err("a single exchange, rewritten each round").contains("rewrites exchange"));
}

/// A SIMD operation or a barrier in a loop whose threads run different iterations is undefined
/// on Metal: the verifier refuses it.
#[test]
fn plan_verifier_refuses_simd_in_a_ragged_loop() {
    let mut p = small_plan();
    let Step::Loop { uniform, .. } = &mut p.steps[0] else { panic!("pass A is a loop") };
    *uniform = false;
    assert!(p.verify().expect_err("a ragged pass A").contains("uniform"));
}

#[test]
fn plan_verifier_refuses_threadgroup_memory_over_the_budget() {
    let mut p = small_plan();
    p.stores.push(threadgroup("opens", BUDGET, false));
    assert!(p.verify().expect_err("over the budget").contains("budget"));
}
