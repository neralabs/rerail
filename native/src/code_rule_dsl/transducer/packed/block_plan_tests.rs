//! Tier 2's analysis and plan, pinned (docs/parser-generators/17-tier-2-tdd-plan.md): the
//! monoids' laws and references (phase 2), JSON's dataflow (phase 3), and its plans (phase 4).

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

// --- Phase 3: the analysis ----------------------------------------------------------------

use super::block_plan::{Rule, Target, analyse, analyse_rule};

/// JSON's block program, as compiled from `json.bitlex`.
fn json_block() -> super::block::BlockProgram {
    super::bitlex_compile_tests::json_block_program()
}

/// JSON's dataflow on Metal (doc 16 §4.1, doc 17 §3): every value's class, monoid and depth; the
/// scans by depth; the candidates; the passes over the bytes. Derived from the program, never
/// written by hand: this text pins the analysis.
#[test]
fn json_values_are_classed_as_designed() {
    let flow = analyse(&json_block(), Target::Gpu).expect("analyses");
    let want = "\
dataflow gpu, rule when
  tier 1: 41 ops: 11 local, 26 given S1, 2 carried, 2 halos
  value                 class     monoid               depth  scan
  %1 escaped            forward   fn                   1      S1
  %4 toggle             forward   xor                  1      S1
  %20 prev %12          halo      1 bit                1      S1
  %27 prev %19          halo      1 bit                1      S1
  tokens                local                          1      rows 5 6 7 8 9
  kinds                 local                          1      byte '{' '}' '[' ']' ',' ':'; text 12
  r1 before ':'         backward  last 1               2      S2
  r1 key                backward  last 1               2      S2
  after ':'             forward   last 1               2      S2
  first token           forward   or                   2      S2
  last token start      forward   last 12              2      S2
  stack                 forward   dyck 2 max 13 pair   3      S3
  nodes per byte        local                          2      4 at most: 3 planes
  ord                   forward   add 14               3      S3
  open ords             forward   last 32 x 27         4      pass B
  scans
  S1  depth 1  forward fn 2->4 (16 bits)
  S2  depth 2  backward last 1, last 1; forward last 1, or, last 12  cand S1
  S3  depth 3  forward dyck 41, add 14  cand S1, fix r1 key, first token, halos
  candidates 4, from S1's 2 state bits
  byte passes 2
";
    assert_eq!(flow.text(), want, "\n{}", flow.text());
}

/// The pair by the rule from the left reads the stack's top inside a block: wide, so no
/// candidates cover it, and the node counts wait for a third pass over the bytes (doc 16 §4.2).
#[test]
fn json_rule_from_the_left_needs_a_third_pass() {
    let flow = analyse_rule(&json_block(), Rule::Also).expect("analyses");
    assert_eq!(flow.byte_passes, 3, "\n{}", flow.text());
    if std::env::var_os("RERAIL_SHOW").is_some() {
        eprintln!("{}", flow.text());
    }
    assert!(flow.text().contains("r1 parent"), "\n{}", flow.text());
    // Metal takes the cheaper rule; the CPU the one it can run forward.
    assert_eq!(analyse(&json_block(), Target::Gpu).expect("gpu").rule, Rule::When);
    assert_eq!(analyse(&json_block(), Target::Cpu).expect("cpu").rule, Rule::Also);
}

/// The CPU runs one pass: every scan a running carry, nothing backward.
#[test]
fn json_cpu_dataflow_is_forward() {
    let flow = analyse(&json_block(), Target::Cpu).expect("analyses");
    assert_eq!(flow.byte_passes, 1, "\n{}", flow.text());
    assert!(flow.values.iter().all(|v| v.class != super::block_plan::Class::Backward), "\n{}", flow.text());
}
