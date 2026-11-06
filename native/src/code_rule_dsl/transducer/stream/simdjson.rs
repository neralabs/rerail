//! simdjson's stage 1, lowered by hand to a stream program: the manual test case asked for
//! before step 1. It checks that the program's shape is right (straight-line bit operations, no
//! loop, no branch, no `rounds`), that it is small (no more word operations per block than
//! simdjson's own), and that its masks equal simdjson's, block by block.
//!
//! The sources lowered, from simdjson's `src/generic/stage1` and `src/arm64.cpp`:
//!
//! - `json_character_block::classify`: the operators `, : [ ] { }` and the whitespace
//!   `space \t \n \r`. Here one nibble table pair holds both, and the backslash, the quote and the
//!   control characters as well, eight classes in all.
//! - `json_escape_scanner::next`: `escaped` by the odd-bits subtraction. simdjson cuts the stream
//!   into words and patches the cut with `next_is_escaped`, fed back from the previous block's
//!   result. Here the stream is one integer: the shift carries a halo and the subtraction a
//!   borrow, so no value is fed back.
//! - `json_string_scanner::next`: `quote`, and `in_string` by `prefix_xor` with the previous
//!   block's sign spread (the toggle's parity carry).
//! - `json_scanner::next` and `json_block`: `follows`, `potential_scalar_start`,
//!   `potential_structural_start`, `string_tail`, `structural_start`.
//! - `json_structural_indexer::next`: the unescaped control characters inside strings.

use super::program::Op;
use super::tests::{Rng, Stream, bit, run};
use super::text;

/// The program, written by hand.
///
/// Table 0's classes: `0x01` `,` (0x2c); `0x02` `:` (0x3a); `0x04` `[ ] { }` (high nibble 5 or 7,
/// low B or D); `0x08` space; `0x10` `\t \n \r` (high 0, low 9, A or D); `0x20` `\`; `0x40` `"`;
/// `0x80` the controls 0x00–0x1f (high 0 or 1).
pub(super) const STAGE1: &str = "stream program -
table 0 lo=8880c0808080808080909284a1948080 hi=90804902002400040000000000000000
; json_character_block::classify
%0 = class t0 0x07          ; op()
%1 = class t0 0x18          ; whitespace()
; json_escape_scanner::next, the stream as one integer
%2 = class t0 0x20          ; backslash = in.eq('\\\\')
%3 = prev %2, 1             ; maybe_escaped = potential_escape << 1
%4 = const 0xaaaaaaaaaaaaaaaa ; ODD_BITS
%5 = or %3, %4              ; maybe_escaped_and_odd_bits
%6 = sub %5, %2             ; even_series_codes_and_odd_bits
%7 = xor %6, %4             ; escape_and_terminal_code
%8 = xor %7, %2             ; escaped
; json_string_scanner::next
%9 = class t0 0x40          ; in.eq('\"')
%10 = andnot %9, %8         ; quote
%11 = toggle %10            ; in_string = prefix_xor(quote) ^ prev_in_string
; json_scanner::next and json_block
%12 = or %0, %1
%13 = not %12               ; scalar()
%14 = andnot %13, %10       ; nonquote_scalar
%15 = prev %14, 1           ; follows(nonquote_scalar, prev_scalar)
%16 = andnot %13, %15       ; potential_scalar_start
%17 = or %0, %16            ; potential_structural_start
%18 = xor %11, %10          ; string_tail
%19 = andnot %17, %18       ; structural_start
; json_structural_indexer::next
%20 = class t0 0x80         ; unescaped = in.lteq(0x1f)
%21 = and %20, %11          ; non_quote_inside_string(unescaped)
output escaped %8
output quote %10
output in_string %11
output structural_start %19
output unescaped_error %21
";

fn stage1() -> super::program::Program {
    text::parse(STAGE1).unwrap_or_else(|e| panic!("{e:?}"))
}

// --- simdjson, block by block, as its source is written -----------------------------------

/// simdjson's stage-1 state between blocks.
#[derive(Default)]
struct Simdjson {
    next_is_escaped: u64,
    prev_in_string: u64,
    prev_scalar: u64,
}

/// One block's masks, as simdjson computes them.
#[derive(Debug, PartialEq, Eq)]
struct Masks {
    escaped: u64,
    quote: u64,
    in_string: u64,
    structural_start: u64,
    unescaped_error: u64,
}

/// `in.eq(c)` etc. over a block: a byte predicate as a mask. Reference only.
fn eq(bytes: &[u8; 64], f: impl Fn(u8) -> bool) -> u64 {
    bytes.iter().enumerate().fold(0, |m, (j, &b)| m | (u64::from(f(b)) << j))
}

fn prefix_xor(a: u64) -> u64 {
    let mut a = a;
    a ^= a << 1;
    a ^= a << 2;
    a ^= a << 4;
    a ^= a << 8;
    a ^= a << 16;
    a ^ (a << 32)
}

impl Simdjson {
    fn next(&mut self, bytes: &[u8; 64]) -> Masks {
        // classify (arm64.cpp): op by `op_table[(d + 3) >> 4] == d`; whitespace by `' '` and
        // `ws_table[d]` for d < 16 (vqtbx1q keeps the compare for larger d).
        const OP_TABLE: [u8; 16] = [0xff, 0, b',', b':', 0, b'[', b']', b'{', b'}', 0, 0, 0, 0, 0, 0, 0];
        const WS_TABLE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0, 0];
        let op = eq(bytes, |d| OP_TABLE[usize::from(d.wrapping_add(3) >> 4)] == d);
        let ws = eq(bytes, |d| if d < 16 { WS_TABLE[usize::from(d)] != 0 } else { d == b' ' });
        // json_escape_scanner::next (without the backslash-free short circuit, which gives the
        // same result).
        const ODD_BITS: u64 = 0xAAAA_AAAA_AAAA_AAAA;
        let backslash = eq(bytes, |d| d == b'\\');
        let potential_escape = backslash & !self.next_is_escaped;
        let maybe_escaped = potential_escape << 1;
        let maybe_escaped_and_odd_bits = maybe_escaped | ODD_BITS;
        let even_series_codes_and_odd_bits = maybe_escaped_and_odd_bits.wrapping_sub(potential_escape);
        let escape_and_terminal_code = even_series_codes_and_odd_bits ^ ODD_BITS;
        let escaped = escape_and_terminal_code ^ (backslash | self.next_is_escaped);
        let escape = escape_and_terminal_code & backslash;
        self.next_is_escaped = escape >> 63;
        // json_string_scanner::next
        let quote = eq(bytes, |d| d == b'"') & !escaped;
        let in_string = prefix_xor(quote) ^ self.prev_in_string;
        self.prev_in_string = ((in_string as i64) >> 63) as u64;
        // json_scanner::next
        let scalar = !(op | ws);
        let nonquote_scalar = scalar & !quote;
        let follows = nonquote_scalar << 1 | self.prev_scalar;
        self.prev_scalar = nonquote_scalar >> 63;
        // json_block
        let potential_scalar_start = scalar & !follows;
        let potential_structural_start = op | potential_scalar_start;
        let string_tail = in_string ^ quote;
        let structural_start = potential_structural_start & !string_tail;
        // json_structural_indexer::next
        let unescaped = eq(bytes, |d| d <= 0x1f);
        let unescaped_error = unescaped & in_string;
        Masks { escaped, quote, in_string, structural_start, unescaped_error }
    }
}

// --- JSON samples -------------------------------------------------------------------------

/// A random JSON value: nesting, strings with escape runs, numbers, literals, whitespace.
fn value(rng: &mut Rng, depth: u32, out: &mut Vec<u8>) {
    let ws = |rng: &mut Rng, out: &mut Vec<u8>| {
        for _ in 0..rng.below(3) {
            out.push([b' ', b'\n', b'\t', b'\r'][rng.below(4) as usize]);
        }
    };
    ws(rng, out);
    match if depth > 4 { 2 + rng.below(4) } else { rng.below(6) } {
        0 => {
            out.push(b'[');
            for i in 0..rng.below(5) {
                if i > 0 {
                    out.push(b',');
                }
                value(rng, depth + 1, out);
            }
            out.push(b']');
        }
        1 => {
            out.push(b'{');
            for i in 0..rng.below(5) {
                if i > 0 {
                    out.push(b',');
                }
                string(rng, out);
                ws(rng, out);
                out.push(b':');
                value(rng, depth + 1, out);
            }
            out.push(b'}');
        }
        2 | 3 => string(rng, out),
        4 => out.extend_from_slice(format!("{}", rng.below(100_000) as i64 - 500).as_bytes()),
        _ => out.extend_from_slice([&b"true"[..], b"false", b"null"][rng.below(3) as usize]),
    }
    ws(rng, out);
}

/// A string with runs of backslashes of every length, escaped quotes, and sometimes a raw
/// control character (an error simdjson reports).
fn string(rng: &mut Rng, out: &mut Vec<u8>) {
    out.push(b'"');
    for _ in 0..rng.below(40) {
        match rng.below(10) {
            0 => {
                for _ in 0..rng.below(6) + 1 {
                    out.push(b'\\');
                }
            }
            1 => out.extend_from_slice(b"\\\""),
            2 if rng.below(8) == 0 => out.push(rng.below(32) as u8),
            3 => out.extend_from_slice("é{,}[:]".as_bytes()),
            _ => out.push(b'a' + rng.below(26) as u8),
        }
    }
    // An odd run would escape the closing quote; keep it closed.
    let run = out.iter().rev().take_while(|&&b| b == b'\\').count();
    if run % 2 == 1 {
        out.push(b'\\');
    }
    out.push(b'"');
}

/// Hand-made documents with escape runs and quotes on both sides of every block edge, then
/// random ones.
fn samples() -> Vec<Vec<u8>> {
    let mut out = vec![br#"{"a":[1,true]}"#.to_vec()];
    for pad in 55..72 {
        for run in 1..6 {
            let mut d = b"[\"".to_vec();
            d.extend(std::iter::repeat_n(b'x', pad));
            d.extend(std::iter::repeat_n(b'\\', run));
            d.extend_from_slice(b"\" , 12 , \"q\\\"\" , nul]");
            out.push(d);
        }
    }
    for seed in 0..200 {
        let mut rng = Rng::new(1000 + seed);
        let mut d = vec![];
        value(&mut rng, 0, &mut d);
        out.push(d);
    }
    out
}

// --- The checks ---------------------------------------------------------------------------

#[test]
fn simdjson_stage1_masks_equal_simdjson_block_by_block() {
    let p = stage1();
    let output = |name: &str| p.outputs().iter().find(|(n, _)| n == name).unwrap().1 as usize;
    let names = ["escaped", "quote", "in_string", "structural_start", "unescaped_error"];
    for doc in samples() {
        let s = Stream::new(&[&doc]);
        let vals = run(&p, &s);
        let mut sj = Simdjson::default();
        for b in 0..s.blocks() {
            let m = sj.next(s.block(b).bytes);
            let want = [m.escaped, m.quote, m.in_string, m.structural_start, m.unescaped_error];
            for (name, w) in names.iter().zip(want) {
                let got = vals[output(name)][b].mask();
                let valid = s.valid[b];
                assert_eq!(
                    got & valid,
                    w & valid,
                    "{name}, block {b} of {:?}",
                    String::from_utf8_lossy(&doc)
                );
            }
        }
    }
}

#[test]
fn simdjson_stage1_structurals_of_a_small_document() {
    let p = stage1();
    let doc = br#"{"a":[1,true]}"#;
    let s = Stream::new(&[doc]);
    let vals = run(&p, &s);
    let v = p.outputs().iter().find(|(n, _)| n == "structural_start").unwrap().1 as usize;
    let at: Vec<usize> = (0..doc.len()).filter(|&j| bit(&vals[v], j)).collect();
    assert_eq!(at, [0, 1, 4, 5, 6, 7, 8, 12, 13]);
}

/// The shape: straight-line, bit operations only, and no more of them than simdjson's.
#[test]
fn simdjson_stage1_shape_is_straight_line_bit_operations() {
    let p = stage1();
    let mut classes = 0;
    let mut word_ops = 0;
    for op in p.insts() {
        match op {
            Op::Class { .. } => classes += 1,
            Op::Const(_)
            | Op::Not(_)
            | Op::And(..)
            | Op::Or(..)
            | Op::Xor(..)
            | Op::AndNot(..)
            | Op::Prev(..)
            | Op::Sub(..)
            | Op::Toggle(_) => word_ops += 1,
            other => panic!("not a bit operation of stage 1: {other:?}"),
        }
    }
    // simdjson's word operations for the same masks, counted from the source above:
    // escape scanner 9 (`& ~`, `<<`, `|`, `-`, `^`, `|`, `^`, `&`, `>> 63`), string scanner 4
    // (`& ~`, `prefix_xor` as one carry-less multiply, `^`, the sign spread), json_scanner and
    // json_block 10 (`|`, `~`, `& ~`, `<<`, `|`, `>> 63`, `& ~`, `|`, `^`, `& ~`), the indexer 1.
    const SIMDJSON_WORD_OPS: usize = 24;
    // Five classes (operators, whitespace, backslash, quote, controls), all from one table pair:
    // simdjson does the first two by lookups and the other three by compares.
    println!(
        "simdjson stage 1 by hand: {} instructions, {classes} class lookups, {word_ops} word \
         operations per 64-byte block (simdjson: {SIMDJSON_WORD_OPS}, carries patched by hand)",
        p.insts().len()
    );
    assert_eq!(classes, 5);
    assert_eq!(word_ops, 17);
    assert!(word_ops < SIMDJSON_WORD_OPS);
    // Every carry is an operation's own: none is fed back from a later instruction.
    assert!(p.insts().iter().all(|op| !matches!(op, Op::Rounds { .. } | Op::Feed(_) | Op::End(_))));
}

/// The table's eight classes are exactly the bytes named.
#[test]
fn simdjson_stage1_table_is_exact() {
    let p = stage1();
    let t = &p.tables()[0];
    for b in 0..=255u8 {
        let expect = [
            (0x01, b == b','),
            (0x02, b == b':'),
            (0x04, matches!(b, b'[' | b']' | b'{' | b'}')),
            (0x08, b == b' '),
            (0x10, matches!(b, b'\t' | b'\n' | b'\r')),
            (0x20, b == b'\\'),
            (0x40, b == b'"'),
            (0x80, b <= 0x1f),
        ];
        for (bits, want) in expect {
            assert_eq!(t.member(b, bits), want, "byte {b:#04x} class {bits:#04x}");
        }
    }
}

/// The kernels are loop-free: `words.rs` has no loop but in the portable class stand-in for
/// hosts without NEON, and every loop in `planes.rs` runs over planes or the fixed shifts.
#[test]
fn kernels_have_no_data_loops() {
    let words = include_str!("words.rs");
    let stand_in = words.find("#[cfg(not(target_arch = \"aarch64\"))]").expect("stand-in");
    let end = stand_in + words[stand_in..].find("\n}\n").expect("end of stand-in");
    let rest = format!("{}{}", &words[..stand_in], &words[end..]);
    for token in ["for ", "while ", "loop {", ".iter()", ".fold(", ".map("] {
        let code: String = rest.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
        assert!(!code.contains(token), "words.rs has `{token}`");
    }
    let planes = include_str!("planes.rs");
    for line in planes.lines().map(str::trim).filter(|l| l.starts_with("for ")) {
        let over_planes = ["in 0..n {", "in 0..w {", "in 1..n {", "in n..WORK {", "in 0..a.len() {",
            "in 0..b.len() {", "in 0..self.len() {", "in (0..a.len()).rev() {", "in SHIFTS {"]
            .iter()
            .any(|t| line.ends_with(t));
        assert!(over_planes, "planes.rs loops over something other than planes: {line}");
    }
    assert!(!planes.contains("while ") && !planes.contains("loop {"));
}
