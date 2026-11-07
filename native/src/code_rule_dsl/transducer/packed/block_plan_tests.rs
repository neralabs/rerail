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
  last token start      forward   last 11              2      S2
  stack                 forward   dyck 2 max 13 pair   3      S3
  nodes per byte        local                          2      4 at most: 3 planes
  ord                   forward   add 14               3      S3
  open ords             forward   last 32 x 27         4      pass B
  scans
  S1  depth 1  forward fn 2->4 (16 bits)
  S2  depth 2  backward last 1, last 1; forward last 1, or, last 11  cand S1
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

// --- Phase 4: the plan --------------------------------------------------------------------

use super::block_plan::{Plan, Step, plan};

fn json_match_scan_block() -> super::block::BlockProgram {
    let src = super::bitlex_compile_tests::JSON_BITLEX.replace("nested max 13", "nested max 13 match scan");
    super::bitlex_compile_tests::json_variant_block_program(&src)
}

/// JSON's plan on Metal (doc 17 §0, §4): the entry as arrays of words, the threadgroup memory
/// within budget, and every step with the stores it reads and writes. Derived from the dataflow:
/// this text pins the plan's shape.
#[test]
fn json_gpu_plan_prints_as_designed() {
    let p = plan(&json_block(), Target::Gpu).expect("plans");
    let want = "\
plan gpu, match barrier
  threads 1024, simd 32, lane 64 bytes, entry 2048 bytes, tile 65536 bytes
  entry: 13 words, 52 bytes, in device memory as 13 arrays, candidate-major
    word 0: s1: fn 2->4 (16 bits), identity 1
    candidate words 1-3, 4-6, 7-9, 10-12:
      r1 before ':' 0..2, r1 key 2..4, after ':' 4..6, first token 6..7, last token start 7..19, stack 32..73, ord 73..87, fix 87..92, error 92..93
  threadgroup memory: 10112 of 10240 bytes
    staging 3328: 2 x 32 entries x 13 words
    exchange 3328: 2 x 32 SIMD totals x 13 words
    opens 3456: 32 SIMD groups x 27 levels x 4 bytes
  steps
    pass A over the bytes, uniform, by tiles of 65536 bytes
      lanes: tier 1 x4, summaries x4; reads bytes
      simd: S1, halos, summaries x4; writes staging
      barrier threadgroup
      flush: 13 arrays x 32 words; reads staging; writes entries
    barrier device
    scan over the entries, uniform, by rounds of 1024
      simd: S1, S2, S3 over 32 entries; reads entries; writes exchange
      barrier threadgroup
      simd: S1, S2, S3 over 32 totals; reads exchange; writes prefixes
    barrier device
    pass B over the bytes, uniform, by tiles of 65536 bytes
      B1 lanes: tier 1, tier-2 masks, 3 planes; reads bytes, prefixes
      B2 outputs: the nodes spread over each SIMD group; writes tape
      B3 lanes: opens and closes in order; writes tape, opens
      barrier threadgroup
      B3: closes of earlier lanes' opens; reads opens; writes tape
      barrier threadgroup
    barrier device
    pass C over the tape, uniform, by rounds of 1024
      off, narrowing, codes; reads tape; writes tape
  barriers: pass A 1 per tile, scan 1 per round, pass B 2 per tile, 3 between passes
";
    assert_eq!(p.text(), want, "\n{}", p.text());
    p.verify().expect("JSON's plan verifies");
}

#[test]
fn json_cpu_plan_is_one_pass() {
    let p = plan(&json_block(), Target::Cpu).expect("plans");
    assert_eq!(p.text(), "plan cpu, rule also\n  steps\n    pass over the bytes, one thread: tier 1, then tier 2, every scan a running carry; reads bytes; writes tape\n", "\n{}", p.text());
    p.verify().expect("verifies");
}

/// Every barrier of JSON's plan is needed: without any one, a store is read before another
/// thread's write is visible, or rewritten while another thread may still read it.
#[test]
fn plan_verifier_refuses_a_missing_barrier() {
    let p = plan(&json_block(), Target::Gpu).expect("plans");
    let n = p.barriers();
    assert_eq!(n, 7);
    for k in 0..n {
        let cut = p.without_barrier(k);
        let e = cut.verify().expect_err(&format!("barrier {k} removed: accepted\n{}", cut.text()));
        assert!(e.contains("barrier"), "{e}");
    }
}

/// A SIMD operation or a barrier in a loop whose threads run different iterations is undefined
/// on Metal: the verifier refuses it.
#[test]
fn plan_verifier_refuses_simd_in_a_ragged_loop() {
    let mut p = plan(&json_block(), Target::Gpu).expect("plans");
    let Step::Loop { uniform, .. } = &mut p.steps[0] else { panic!("pass A is a loop") };
    *uniform = false;
    assert!(p.verify().expect_err("a ragged pass A").contains("uniform"));
}

#[test]
fn json_entry_is_13_arrays_of_words() {
    let p: Plan = plan(&json_block(), Target::Gpu).expect("plans");
    assert_eq!((p.entry_words(), p.entry_words() * 4), (13, 52));
    assert!(p.threadgroup_bytes() <= p.budget, "{} of {}", p.threadgroup_bytes(), p.budget);
}

/// `match scan`: the opens go through the scans, so pass B has no barrier of its own, the entry
/// grows by 27 words a candidate, and the scan exchanges S1 first, then the picked candidate.
#[test]
fn match_scan_has_no_mid_pass_barrier() {
    let p = plan(&json_match_scan_block(), Target::Gpu).expect("plans");
    p.verify().expect("verifies");
    let text = p.text();
    if std::env::var_os("RERAIL_SHOW").is_some() {
        eprintln!("{text}");
    }
    assert!(text.starts_with("plan gpu, match scan\n"), "{text}");
    assert!(text.contains("pass B 0 per tile") && text.contains("scan 2 per round"), "{text}");
    assert_eq!(p.entry_words(), 1 + 4 * (3 + 27), "{text}");
    assert!(p.threadgroup_bytes() <= p.budget, "{text}");
}
