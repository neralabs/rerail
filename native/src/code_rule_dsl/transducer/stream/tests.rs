//! Step 1's checks (`10` §5): every operation equals a scalar reference on random streams, at
//! every block edge and across file starts; summaries combined in another order give the same
//! carries; the verifier refuses one broken program per rule; the text form round-trips.
//!
//! The drivers here run one instruction over every block before the next (forward, or backward
//! for `next` and `add_back`). They are test scaffolding for the kernels, not the interpreter
//! (step 2), and run no `rounds`. The references walk positions one at a time: test code only.

use super::program::{ClassTable, Op, Program, Ty, Value};
use super::text;
use super::verify::Rule;
use super::words::{self, Block, Carry, Summary, Word};

// --- Streams and drivers ------------------------------------------------------------------

/// Files end to end, padded with zero bytes to whole blocks.
pub(super) struct Stream {
    pub(super) bytes: Vec<u8>,
    pub(super) len: usize,
    pub(super) starts: Vec<u64>,
    pub(super) valid: Vec<u64>,
}

impl Stream {
    pub(super) fn new(files: &[&[u8]]) -> Self {
        let mut bytes = vec![];
        let mut start_at = vec![];
        for f in files {
            start_at.push(bytes.len());
            bytes.extend_from_slice(f);
        }
        let len = bytes.len();
        let blocks = len.div_ceil(64).max(1);
        bytes.resize(blocks * 64, 0);
        let mut starts = vec![0u64; blocks];
        for s in start_at.into_iter().filter(|&s| s < len) {
            starts[s / 64] |= 1 << (s % 64);
        }
        let valid = (0..blocks)
            .map(|b| {
                let n = len.saturating_sub(b * 64).min(64);
                if n == 64 { !0 } else { (1u64 << n) - 1 }
            })
            .collect();
        Self { bytes, len, starts, valid }
    }

    pub(super) fn blocks(&self) -> usize {
        self.starts.len()
    }

    pub(super) fn block(&self, b: usize) -> Block<'_> {
        Block {
            bytes: self.bytes[b * 64..b * 64 + 64].try_into().unwrap(),
            starts: self.starts[b],
            valid: self.valid[b],
        }
    }
}

/// Every instruction's word in every block, carries passed block to block in order.
pub(super) fn run(p: &Program, s: &Stream) -> Vec<Vec<Word>> {
    let mut vals: Vec<Vec<Word>> = vec![];
    for (i, op) in p.insts().iter().enumerate() {
        assert!(!matches!(op, Op::Rounds { .. } | Op::Feed(_) | Op::End(_)), "no rounds here");
        let mut out = vec![Word::Mask(0); s.blocks()];
        let mut carry = words::initial(op);
        for b in order(op, s.blocks()) {
            let (w, c) = words::step(p, i, &s.block(b), &|v: Value| vals[v as usize][b], carry);
            out[b] = w;
            carry = c;
        }
        vals.push(out);
    }
    vals
}

/// The same from summaries: each block's summary from the block alone, every block's carry as
/// the combine of the summaries before it (grouped as a balanced tree, not left to right), and
/// then the blocks run in reverse of the carry's order.
fn run_from_summaries(p: &Program, s: &Stream) -> Vec<Vec<Word>> {
    let mut vals: Vec<Vec<Word>> = vec![];
    for (i, op) in p.insts().iter().enumerate() {
        let seq: Vec<usize> = order(op, s.blocks()).collect();
        let sums: Vec<Summary> = seq
            .iter()
            .map(|&b| words::summary(p, i, &s.block(b), &|v: Value| vals[v as usize][b]))
            .collect();
        let mut out = vec![Word::Mask(0); s.blocks()];
        for k in (0..seq.len()).rev() {
            let carry = match fold(op, &sums[..k]) {
                None => words::initial(op),
                Some(sum) => words::apply(op, sum, words::initial(op)),
            };
            let b = seq[k];
            out[b] = words::step(p, i, &s.block(b), &|v: Value| vals[v as usize][b], carry).0;
        }
        vals.push(out);
    }
    vals
}

fn fold(op: &Op, sums: &[Summary]) -> Option<Summary> {
    match sums.len() {
        0 => None,
        1 => Some(sums[0]),
        n => {
            let (a, b) = sums.split_at(n / 2);
            Some(words::combine(op, fold(op, a)?, fold(op, b)?))
        }
    }
}

fn order(op: &Op, blocks: usize) -> Box<dyn Iterator<Item = usize>> {
    if op.backward() { Box::new((0..blocks).rev()) } else { Box::new(0..blocks) }
}

/// Bit `j` of a mask value across the stream.
pub(super) fn bit(words: &[Word], j: usize) -> bool {
    words[j / 64].mask() >> (j % 64) & 1 != 0
}

// --- Random streams -----------------------------------------------------------------------

pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Bytes `@` to `G` (0x40 + i) are in class bit `i`: a random stream over them gives random
/// masks, of a density set by the byte weights.
fn letters() -> ClassTable {
    let mut lo = [0u8; 16];
    for (i, l) in lo.iter_mut().enumerate().take(8) {
        *l = 1 << i;
    }
    let mut hi = [0u8; 16];
    hi[4] = 0xff;
    ClassTable { lo, hi }
}

/// Random files over the letters, some runs long enough to cross blocks.
fn random_files(rng: &mut Rng, files: usize) -> Vec<Vec<u8>> {
    (0..files)
        .map(|_| {
            let len = rng.below(200) as usize + 1;
            let mut f = Vec::with_capacity(len);
            while f.len() < len {
                let b = 0x40 + rng.below(8) as u8;
                // Sometimes a run of one letter, up to two blocks long.
                let run = if rng.below(8) == 0 { rng.below(130) as usize + 1 } else { 1 };
                f.extend(std::iter::repeat_n(b, run.min(len - f.len())));
            }
            f
        })
        .collect()
}

/// The streams every operation is checked on: lengths around every block edge (one file), and
/// many files end to end.
fn streams() -> Vec<Stream> {
    let mut rng = Rng::new(7);
    let mut out = vec![];
    for len in [1usize, 2, 63, 64, 65, 127, 128, 129, 191, 192, 193, 300] {
        let f: Vec<u8> = (0..len).map(|_| 0x40 + rng.below(8) as u8).collect();
        out.push(Stream::new(&[&f]));
    }
    for seed in 0..24 {
        let mut rng = Rng::new(100 + seed);
        let files = random_files(&mut rng, 1 + seed as usize % 6);
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        out.push(Stream::new(&refs));
    }
    out
}

// --- References ---------------------------------------------------------------------------

/// The stream's masks per position, from the bytes: `a`, `b`, `c` as the program below defines
/// them, and the file starts.
struct Inputs {
    a: Vec<bool>,
    b: Vec<bool>,
    c: Vec<bool>,
    starts: Vec<bool>,
}

/// The test programs' first instructions: three overlapping masks.
const PRELUDE: &str = "stream program -
table 0 lo=01020408102040800000000000000000 hi=00000000ff0000000000000000000000
%0 = class t0 0x13
%1 = class t0 0x26
%2 = class t0 0x4c
";

fn inputs(s: &Stream) -> Inputs {
    let t = letters();
    let m = |bits: u8| (0..s.len).map(|j| t.member(s.bytes[j], bits)).collect();
    Inputs {
        a: m(0x13),
        b: m(0x26),
        c: m(0x4c),
        starts: (0..s.len).map(|j| s.starts[j / 64] >> (j % 64) & 1 != 0).collect(),
    }
}

fn add_ref(a: &[bool], b: &[bool]) -> Vec<bool> {
    let mut c = false;
    a.iter()
        .zip(b)
        .map(|(&x, &y)| {
            let s = x ^ y ^ c;
            c = (x & y) | (c & (x ^ y));
            s
        })
        .collect()
}

fn sub_ref(a: &[bool], b: &[bool]) -> Vec<bool> {
    // a - b = a + !b + 1.
    let mut c = true;
    a.iter()
        .zip(b)
        .map(|(&x, &y)| {
            let y = !y;
            let s = x ^ y ^ c;
            c = (x & y) | (c & (x ^ y));
            s
        })
        .collect()
}

/// A per-position reference for each test program's last instruction.
enum Expect {
    Mask(Box<dyn Fn(&Inputs) -> Vec<bool>>),
    Num(Box<dyn Fn(&Inputs) -> Vec<u16>>),
}

/// Check one program's last instruction against its reference, in both drivers.
fn check(body: &str, expect: Expect) {
    let p = text::parse(&format!("{PRELUDE}{body}")).unwrap_or_else(|e| panic!("{body}: {e:?}"));
    let last = p.insts().len() - 1;
    for s in streams() {
        let inp = inputs(&s);
        let seq = run(&p, &s);
        let alt = run_from_summaries(&p, &s);
        assert_eq!(seq[last], alt[last], "{body}: summaries differ, len {}", s.len);
        match &expect {
            Expect::Mask(f) => {
                let want = f(&inp);
                for (j, w) in want.iter().enumerate() {
                    assert_eq!(bit(&seq[last], j), *w, "{body}: byte {j} of {}", s.len);
                }
            }
            Expect::Num(f) => {
                let want = f(&inp);
                for (j, w) in want.iter().enumerate() {
                    let got = seq[last][j / 64].planes().at((j % 64) as u32);
                    assert_eq!(got, *w, "{body}: byte {j} of {}", s.len);
                }
            }
        }
    }
}

fn mask(f: impl Fn(&Inputs) -> Vec<bool> + 'static) -> Expect {
    Expect::Mask(Box::new(f))
}

fn num(f: impl Fn(&Inputs) -> Vec<u16> + 'static) -> Expect {
    Expect::Num(Box::new(f))
}

fn zip2(a: &[bool], b: &[bool], f: impl Fn(bool, bool) -> bool) -> Vec<bool> {
    a.iter().zip(b).map(|(&x, &y)| f(x, y)).collect()
}

// --- The operations -----------------------------------------------------------------------

#[test]
fn logic_ops_equal_reference() {
    check("%3 = not %0\n", mask(|i| i.a.iter().map(|x| !x).collect()));
    check("%3 = and %0, %1\n", mask(|i| zip2(&i.a, &i.b, |x, y| x & y)));
    check("%3 = or %0, %1\n", mask(|i| zip2(&i.a, &i.b, |x, y| x | y)));
    check("%3 = xor %0, %1\n", mask(|i| zip2(&i.a, &i.b, |x, y| x ^ y)));
    check("%3 = andnot %0, %1\n", mask(|i| zip2(&i.a, &i.b, |x, y| x & !y)));
    check(
        "%3 = select %0, %1, %2\n",
        mask(|i| (0..i.a.len()).map(|j| if i.a[j] { i.b[j] } else { i.c[j] }).collect()),
    );
    check("%3 = starts\n", mask(|i| i.starts.clone()));
    check(
        "%3 = const 0xaaaaaaaaaaaaaaaa\n",
        mask(|i| (0..i.a.len()).map(|j| j % 2 == 1).collect()),
    );
}

#[test]
fn class_equals_table_membership() {
    // Every byte value, in every position of a block.
    let all: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
    let s = Stream::new(&[&all]);
    let table = ClassTable {
        lo: [0x88, 0x80, 0xc0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x90, 0x92, 0x84, 0xa1, 0x94, 0x80, 0x80],
        hi: [0x90, 0x80, 0x49, 0x02, 0x00, 0x24, 0x00, 0x04, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    for bits in [0x01u8, 0x07, 0x18, 0x20, 0x40, 0x80, 0xff] {
        for b in 0..s.blocks() {
            let got = words::class(&table, bits, s.block(b).bytes);
            for j in 0..64 {
                let byte = s.bytes[b * 64 + j];
                assert_eq!(got >> j & 1 != 0, table.member(byte, bits), "byte {byte:#x} bits {bits:#x}");
            }
        }
    }
}

#[test]
fn shifts_equal_reference() {
    for k in [1u8, 2, 3, 31, 63] {
        check(
            &format!("%3 = prev %0, {k}\n"),
            mask(move |i| (0..i.a.len()).map(|j| j >= k as usize && i.a[j - k as usize]).collect()),
        );
        check(
            &format!("%3 = next %0, {k}\n"),
            mask(move |i| (0..i.a.len()).map(|j| i.a.get(j + k as usize).copied().unwrap_or(false)).collect()),
        );
    }
}

#[test]
fn carries_equal_reference() {
    check("%3 = add %0, %1\n", mask(|i| add_ref(&i.a, &i.b)));
    check("%3 = sub %0, %1\n", mask(|i| sub_ref(&i.a, &i.b)));
    check(
        "%3 = add_back %0, %1\n",
        mask(|i| {
            let ra: Vec<bool> = i.a.iter().rev().copied().collect();
            let rb: Vec<bool> = i.b.iter().rev().copied().collect();
            add_ref(&ra, &rb).into_iter().rev().collect()
        }),
    );
    check(
        "%3 = toggle %0\n",
        mask(|i| {
            let mut p = false;
            i.a.iter().map(|&x| {
                p ^= x;
                p
            })
            .collect()
        }),
    );
}

/// A scan's reference: `s = step((reset ? 0 : s'), up, down)`.
fn scan_ref(i: &Inputs, f: impl Fn(i32, bool, bool) -> i32) -> Vec<u16> {
    let mut s = 0i32;
    (0..i.a.len())
        .map(|j| {
            let from = if i.c[j] { 0 } else { s };
            s = f(from, i.a[j], i.b[j]);
            s as u16
        })
        .collect()
}

fn delta(up: bool, down: bool) -> i32 {
    i32::from(up) - i32::from(down)
}

#[test]
fn plane_scans_equal_reference() {
    for n in [2u8, 3, 4, 5, 7, 8, 16] {
        let m = i32::from(n);
        check(
            &format!("%3 = scan_mod {n} %0, %1, %2\n"),
            num(move |i| scan_ref(i, |x, u, d| (x + delta(u, d)).rem_euclid(m))),
        );
    }
    for max in [1u8, 2, 3, 7, 8, 30, 255] {
        let top = i32::from(max) + 1;
        check(
            &format!("%3 = scan_clamp {max} %0, %1, %2\n"),
            num(move |i| scan_ref(i, |x, u, d| (x + delta(u, d)).clamp(0, top))),
        );
    }
    // A plane and an equality read the planes back as masks.
    check(
        "%3 = scan_mod 3 %0, %1, %2\n%4 = plane_eq %3, 2\n",
        mask(|i| scan_ref(i, |x, u, d| (x + delta(u, d)).rem_euclid(3)).iter().map(|&v| v == 2).collect()),
    );
    check(
        "%3 = scan_clamp 8 %0, %1, %2\n%4 = plane %3, 3\n",
        mask(|i| scan_ref(i, |x, u, d| (x + delta(u, d)).clamp(0, 9)).iter().map(|&v| v & 8 != 0).collect()),
    );
}

#[test]
fn count_is_a_running_total_per_block() {
    let p = text::parse(&format!("{PRELUDE}%3 = count %0\n")).unwrap();
    for s in streams() {
        let inp = inputs(&s);
        let seq = run(&p, &s);
        let alt = run_from_summaries(&p, &s);
        assert_eq!(seq[3], alt[3]);
        for b in 0..s.blocks() {
            let end = ((b + 1) * 64).min(s.len);
            let want = inp.a[..end].iter().filter(|&&x| x).count() as u64;
            assert_eq!(seq[3][b], Word::Count(want));
        }
    }
}

#[test]
fn summaries_combine_associatively() {
    // Three blocks' summaries, grouped both ways, for every carrying operation.
    let mut rng = Rng::new(3);
    let f: Vec<u8> = (0..192).map(|_| 0x40 + rng.below(8) as u8).collect();
    let s = Stream::new(&[&f]);
    for body in [
        "%3 = prev %0, 5\n",
        "%3 = next %0, 5\n",
        "%3 = add %0, %1\n",
        "%3 = sub %0, %1\n",
        "%3 = add_back %0, %1\n",
        "%3 = toggle %0\n",
        "%3 = scan_mod 5 %0, %1, %2\n",
        "%3 = scan_clamp 8 %0, %1, %2\n",
        "%3 = count %0\n",
    ] {
        let p = text::parse(&format!("{PRELUDE}{body}")).unwrap();
        let vals = run(&p, &s);
        let op = &p.insts()[3];
        let sums: Vec<Summary> =
            (0..3).map(|b| words::summary(&p, 3, &s.block(b), &|v: Value| vals[v as usize][b])).collect();
        let left = words::combine(op, words::combine(op, sums[0], sums[1]), sums[2]);
        let right = words::combine(op, sums[0], words::combine(op, sums[1], sums[2]));
        assert_eq!(left, right, "{body}");
        assert_ne!(words::initial(op), Carry::None, "{body}");
    }
}

// --- The verifier -------------------------------------------------------------------------

fn refused(body: &str) -> Rule {
    match text::parse(&format!("{PRELUDE}{body}")) {
        Err(text::ParseError::Refused(r)) => r.rule,
        other => panic!("{body}: not refused: {other:?}"),
    }
}

#[test]
fn verifier_refuses_one_program_per_rule() {
    let cases: &[(&str, Rule)] = &[
        ("%3 = and %0, %3\n", Rule::OperandNotEarlier),
        ("%3 = rounds 2\n%4 = feed %0\n%5 = end %4 <- %4\n%6 = not %3\n", Rule::OperandHasNoValue),
        ("%3 = scan_mod 3 %0, %1, %2\n%4 = not %3\n", Rule::OperandType),
        ("%3 = count %0\n%4 = toggle %3\n", Rule::OperandType),
        ("%3 = plane %0, 0\n", Rule::OperandType),
        (
            "%3 = rounds 2\n%4 = feed %0\n%5 = not %4\n%6 = end %4 <- %5\n\
             %7 = rounds 2\n%8 = feed %0\n%9 = and %8, %5\n%10 = end %8 <- %9\n",
            Rule::OperandOutOfScope,
        ),
        ("%3 = prev %0, 0\n", Rule::ShiftRange),
        ("%3 = next %0, 64\n", Rule::ShiftRange),
        ("%3 = scan_mod 1 %0, %1, %2\n", Rule::ModRange),
        ("%3 = scan_mod 17 %0, %1, %2\n", Rule::ModRange),
        ("%3 = scan_clamp 0 %0, %1, %2\n", Rule::ClampRange),
        ("%3 = class t1 0x01\n", Rule::ClassTable),
        ("%3 = class t0 0x00\n", Rule::ClassTable),
        ("%3 = scan_mod 3 %0, %1, %2\n%4 = plane %3, 2\n", Rule::PlaneRange),
        ("%3 = scan_mod 3 %0, %1, %2\n%4 = plane_eq %3, 4\n", Rule::PlaneRange),
        ("%3 = rounds 0\n%4 = end\n", Rule::RoundsCount),
        ("%3 = rounds 65\n%4 = end\n", Rule::RoundsCount),
        (
            "%3 = rounds 2\n%4 = rounds 2\n%5 = rounds 2\n%6 = end\n%7 = end\n%8 = end\n",
            Rule::RoundsDepth,
        ),
        ("%3 = feed %0\n", Rule::FeedPlace),
        ("%3 = rounds 2\n%4 = not %0\n%5 = feed %0\n%6 = end %5 <- %4\n", Rule::FeedPlace),
        ("%3 = end\n", Rule::RoundsUnbalanced),
        ("%3 = rounds 2\n%4 = not %0\n", Rule::RoundsUnbalanced),
        ("%3 = rounds 2\n%4 = feed %0\n%5 = not %4\n%6 = end\n", Rule::EndFeeds),
        ("%3 = rounds 2\n%4 = feed %0\n%5 = end %4 <- %1\n", Rule::EndFeeds),
        (
            "%3 = rounds 2\n%4 = feed %0\n%5 = count %4\n%6 = end %4 <- %5\n",
            Rule::EndFeeds,
        ),
        ("%3 = not %0\noutput x %9\n", Rule::Output),
        ("%3 = not %0\noutput x %3\noutput x %2\n", Rule::Output),
    ];
    for (body, rule) in cases {
        assert_eq!(refused(body), *rule, "{body}");
    }
    // Rules no case reaches would be dead; every rule has a case.
    let all = [
        Rule::OperandNotEarlier,
        Rule::OperandHasNoValue,
        Rule::OperandType,
        Rule::OperandOutOfScope,
        Rule::ShiftRange,
        Rule::ModRange,
        Rule::ClampRange,
        Rule::ClassTable,
        Rule::PlaneRange,
        Rule::RoundsCount,
        Rule::RoundsDepth,
        Rule::FeedPlace,
        Rule::RoundsUnbalanced,
        Rule::EndFeeds,
        Rule::Output,
    ];
    for r in all {
        assert!(cases.iter().any(|(_, c)| *c == r), "{r:?} has no case");
    }
}

#[test]
fn parser_refuses_any_other_operation() {
    for line in [
        "%3 = loop %0\n",
        "%3 = branch %0, %1\n",
        "%3 = for_each_byte %0\n",
        "%3 = call %0\n",
        "%3 = gather %0, %1\n",
        "%3 = and %0\n",
        "%4 = not %0\n",
    ] {
        assert!(
            matches!(text::parse(&format!("{PRELUDE}{line}")), Err(text::ParseError::Line(..))),
            "{line}"
        );
    }
}

#[test]
fn rounds_and_scopes_that_verify() {
    // A nested body, a value read after its body (the last round's), and an outer feed fed from
    // an inner body's value.
    let body = "%3 = rounds 8 settle
%4 = feed %0
%5 = rounds 4
%6 = feed %4
%7 = toggle %6
%8 = end %6 <- %7
%9 = or %7, %1
%10 = end %4 <- %9
%11 = and %9, %7
output out %11
";
    let p = text::parse(&format!("{PRELUDE}{body}")).unwrap();
    assert_eq!(p.ty(3), None);
    assert_eq!(p.ty(4), Some(Ty::Mask));
    assert_eq!(p.ty(11), Some(Ty::Mask));
}

// --- The text form ------------------------------------------------------------------------

#[test]
fn text_round_trips_with_its_digest() {
    let body = "%3 = prev %0, 3
%4 = next %1, 2
%5 = sub %3, %4
%6 = add_back %5, %2
%7 = scan_mod 3 %0, %1, %2
%8 = plane_eq %7, 1
%9 = scan_clamp 8 %0, %1, %2
%10 = plane %9, 0
%11 = rounds 3 settle
%12 = feed %8
%13 = select %12, %10, %6
%14 = end %12 <- %13
%15 = count %13
%16 = const 0x5555555555555555
output tokens %15
output mask %13
";
    let p = text::parse(&format!("{PRELUDE}{body}")).unwrap();
    let printed = text::print(&p);
    let q = text::parse(&printed).unwrap();
    assert_eq!(p, q);
    assert_eq!(p.digest(), q.digest());
    assert_eq!(printed, text::print(&q));
    // A changed line no longer matches the stated digest.
    let tampered = printed.replace("prev %0, 3", "prev %0, 4");
    assert_eq!(text::parse(&tampered), Err(text::ParseError::Digest));
}

/// Kept so an `Op` added without a reference here fails to build: every operation is listed.
#[allow(dead_code)]
fn every_operation_has_a_test(op: &Op) -> &'static str {
    match op {
        Op::Starts | Op::Const(_) | Op::Not(_) | Op::And(..) | Op::Or(..) | Op::Xor(..) => "logic",
        Op::AndNot(..) | Op::Select(..) => "logic",
        Op::Class { .. } => "class",
        Op::Prev(..) | Op::Next(..) => "shifts",
        Op::Add(..) | Op::Sub(..) | Op::AddBack(..) | Op::Toggle(_) => "carries",
        Op::ScanMod { .. } | Op::ScanClamp { .. } | Op::Plane(..) | Op::PlaneEq(..) => "plane scans",
        Op::Count(_) => "count",
        Op::Rounds { .. } | Op::Feed(_) | Op::End(_) => "verifier and text",
    }
}
