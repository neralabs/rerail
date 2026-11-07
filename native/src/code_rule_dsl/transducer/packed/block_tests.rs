//! Block programs, built up one primitive at a time (`docs/parser-generators/12-what-is-wanted.md`).
//! Each primitive is a tiny transducer program written in `rir`, checked on every executor (the
//! CPU interpreter, Cranelift, and the Metal interpreter and generated MSL at every lane width)
//! against a byte-by-byte reference that exists only here, and checked for what it lowers to:
//! straight bit operations.

use super::*;
use super::block::BlockRun;

/// A transducer program whose walk does nothing and whose block program is `rows`.
fn program(rows: &str) -> Program {
    let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
    rir::assemble(&text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

/// Inputs that put every byte value at every position of a block, and lengths around every
/// block edge.
fn inputs() -> Vec<Vec<u8>> {
    let every: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).chain(0..=255u8).collect();
    let mut out = vec![vec![], b"\"".to_vec(), every.clone()];
    for len in [1usize, 2, 63, 64, 65, 127, 128, 129, 200] {
        out.push(every.iter().copied().cycle().skip(len * 7).take(len).collect());
    }
    // A quote at each position of two blocks.
    for at in 0..128 {
        let mut v = vec![b'x'; 130];
        v[at] = b'"';
        out.push(v);
    }
    out.extend(runs());
    out
}

/// Runs of backslashes, quotes and other bytes of every length to 200, over enough bytes that
/// carries cross lanes, SIMD groups and threadgroups at every lane width (a group is 512 to 2048
/// bytes), with a stretch of one long run.
fn runs() -> Vec<Vec<u8>> {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut out = Vec::new();
    for len in [700usize, 3000, 20_000] {
        let mut v = Vec::with_capacity(len);
        while v.len() < len {
            let byte = [b'\\', b'"', b'a', b' ', b',', b'\\'][next(6) as usize];
            let long = next(4) == 0;
            let run = 1 + next(if long { 200 } else { 5 }) as usize;
            v.extend(std::iter::repeat_n(byte, run));
        }
        v.truncate(len);
        out.push(v);
    }
    let mut long = vec![b'\\'; 9000];
    long.extend(b"\"x\"");
    out.push(long);
    out
}

/// The reference: bit `j` of word `j / 64` where byte `j` passes.
fn reference(bytes: &[u8], f: impl Fn(u8) -> bool) -> Vec<u64> {
    let mut words = vec![0u64; bytes.len().div_ceil(64)];
    for (j, &b) in bytes.iter().enumerate() {
        words[j / 64] |= u64::from(f(b)) << (j % 64);
    }
    words
}

/// Every executor's outputs for the program's first block program, with the executor's name.
fn executors(p: &Program, bytes: &[u8]) -> Vec<(String, Vec<Vec<u64>>)> {
    let block = &p.block_programs()[0];
    let native = native_ir::lower(p).expect("lowers").block_programs.remove(0);
    let mut out = vec![
        ("cpu interpreter".to_owned(), block.interpret(bytes)),
        ("cranelift".to_owned(), native_cpu_block::compile(&native).expect("compiles").run(bytes)),
    ];
    #[cfg(target_os = "macos")]
    {
        use metal_program_backend::block_program::{ClassStrategy, LaneWidth, interpret, run_generated_with};
        for width in LaneWidth::ALL {
            let w = width.bytes();
            out.push((format!("metal interpreter, {w}-byte lanes"), interpret(block, bytes, width).expect("runs")));
            for strategy in ClassStrategy::ALL {
                let (got, _) = run_generated_with(&native, bytes, width, strategy).expect("runs");
                out.push((format!("generated msl, {w}-byte lanes, {strategy:?}"), got));
            }
        }
    }
    out
}

/// Check a program's outputs against references on every input and every executor.
fn check_all(rows: &str, want: impl Fn(&[u8]) -> Vec<Vec<u64>>) {
    let p = program(rows);
    for bytes in inputs() {
        let want = want(&bytes);
        for (name, got) in executors(&p, &bytes) {
            assert_eq!(got, want, "{name}, {} bytes, program:\n{rows}", bytes.len());
        }
    }
}

// --- Primitive 1: eq ----------------------------------------------------------------------

const EQ_QUOTE: &str = "  block 0 %0 = eq 0x22\n  block 0 output %0\n";

#[test]
fn eq_round_trips_through_rir() {
    let p = program(EQ_QUOTE);
    assert_eq!(p.block_programs()[0].ops, [block::BlockOp::Eq(b'"')]);
    let dump = rir::disassemble(&p, None);
    assert!(dump.contains("  block 0 %0 = eq 0x22\n  block 0 output %0\n"), "{dump}");
    let again = rir::assemble(&dump).expect("the dump assembles");
    assert_eq!(again.plain_digest(), p.plain_digest());
    // The block program is in the digest.
    let other = program("  block 0 %0 = eq 0x5c\n  block 0 output %0\n");
    assert_ne!(other.plain_digest(), p.plain_digest());
    let none = rir::assemble("program -\n  mode set=0 query=0\n  0000 STOP 0\n").unwrap();
    assert_ne!(none.plain_digest(), p.plain_digest());
}

#[test]
fn eq_refusals() {
    let refused = |rows: &str| {
        rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n"))
            .err()
            .unwrap_or_else(|| panic!("not refused: {rows}"))
    };
    assert!(refused("  block 0 %0 = eq 0x22\n  block 0 output %1\n").contains("verifier"));
    assert!(refused("  block 0 %0 = eq 0x22\n").contains("verifier"));
    assert!(refused("  block 0 %0 = loop 0x22\n  block 0 output %0\n").contains("unknown block operation"));
    assert!(refused("  block 0 %1 = eq 0x22\n").contains("out of order"));
    assert!(refused("  block 1 %0 = eq 0x22\n").contains("out of order"));
}

#[test]
fn eq_equals_reference_on_every_executor() {
    for byte in [b'"', b'\\', 0x00, 0xff, b'a'] {
        let p = program(&format!("  block 0 %0 = eq {byte:#04x}\n  block 0 output %0\n"));
        for bytes in inputs() {
            let want = reference(&bytes, |b| b == byte);
            for (name, got) in executors(&p, &bytes) {
                assert_eq!(got, [want.clone()], "{name}, eq {byte:#04x}, {} bytes", bytes.len());
            }
        }
    }
}

#[test]
fn eq_swar_form_equals_reference() {
    for bytes in inputs() {
        for byte in [b'"', 0x00, 0x80, 0xff] {
            let got: Vec<u64> = block::padded(&bytes)
                .iter()
                .map(|b| block::eq_swar(b, byte))
                .collect::<Vec<_>>();
            let mut got = got;
            block::mask_tail(&mut got, bytes.len());
            assert_eq!(got, reference(&bytes, |b| b == byte));
        }
    }
}

// --- What each executor lowers to ----------------------------------------------------------

/// The Cranelift IR of the loop body: everything between the body block's label and the next
/// block's.
fn body_of(clif: &str) -> Vec<String> {
    let lines: Vec<&str> = clif.lines().collect();
    let start = lines.iter().position(|l| l.trim_start().starts_with("block1")).expect("a body block") + 1;
    lines[start..]
        .iter()
        .take_while(|l| !l.trim_start().starts_with("block"))
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The CLIF opcode of an instruction line (`v3 = icmp eq v1, v2` → `icmp`).
fn opcode(line: &str) -> &str {
    let rhs = line.split_once(" = ").map_or(line, |(_, r)| r);
    rhs.split([' ', '.']).next().unwrap_or("")
}

/// The opcodes a block body may contain: loads and stores of the block, bit operations (the
/// bitmask's pairwise adds included), the block's address arithmetic, and the one branch back to
/// the loop's head.
const BODY_OPCODES: &[&str] = &[
    "load", "store", "icmp", "icmp_imm", "vhigh_bits", "uextend", "ishl_imm", "bor", "band", "bxor",
    "bnot", "band_not", "iadd", "iadd_imm", "iconst", "brif", "iadd_pairwise", "bitcast", "extractlane",
    "swizzle", "ushr_imm", "sshr_imm", "uadd_overflow",
    // The extract pass's size: each extract's popcount, summed as the plane is written.
    "popcnt",
];

/// Check the lowered forms of a program and print them.
fn check_lowering(name: &str, p: &Program) {
    let native = native_ir::lower(p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let body = body_of(&compiled.clif);
    println!("== {name}: Cranelift IR of the block body ==\n{}", body.join("\n"));
    for line in &body {
        assert!(BODY_OPCODES.contains(&opcode(line)), "{name}: `{line}` is not a bit operation");
    }
    let branches = body.iter().filter(|l| opcode(l) == "brif").count();
    assert_eq!(branches, 1, "{name}: the body's only branch is the loop's");
    println!("== {name}: machine code ==\n{}", compiled.disassembly);
    assert!(!compiled.disassembly.contains(" bl ") && !compiled.disassembly.contains("blr"), "{name}: no calls");

    let instructions = compiled.disassembly.lines().filter(|l| l.starts_with("  ") && !l.contains("unwind")).count();
    println!("== {name}: {instructions} machine instructions in the function ==");
    for width in [16usize, 32, 64] {
        let carried = native.insts.iter().any(native_ir::BlockInst::carried);
        for (strategy, msl) in metal_program_backend_generate(&native, width) {
            println!("== {name}: generated MSL, {width}-byte lanes, {strategy} ==\n{msl}");
            // The lane code (the carry helpers, each round's summary and `tempo_block`) is
            // straight-line; only a round kernel's look-back loops, over predecessor groups.
            for (section, code) in lane_code(&msl) {
                for word in ["for (", "while", "switch", "goto", "?", "if (", "break"] {
                    assert!(!code.contains(word), "{name}: generated MSL ({strategy}), {section}, has `{word}`");
                }
                // SIMD-group operations only combine carries: none without a carried instruction.
                if !carried && section == "tempo_block" {
                    assert!(!code.contains("simd_"), "{name}: generated MSL ({strategy}) has a SIMD-group operation");
                }
            }
            // The only memory a class formula reads is the bytes: no tables.
            if strategy == "Planes" {
                assert!(!msl.contains("constant uchar"), "{name}: the planes form reads a table");
            }
        }
    }
}

/// The generated MSL's lane code: every function (the carry helpers, the rounds' carry-outs,
/// tables, carry-ins and halo operands, `halo_window`) and `tempo_block`. Only round kernels,
/// which hold the look-back, are left out.
fn lane_code(msl: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for marker in ["static inline ", "kernel void tempo_block("] {
        for (at, _) in msl.match_indices(marker) {
            let code = &msl[at..];
            let end = code.find("\n}\n").map_or(code.find('\n').unwrap_or(code.len()), |e| e + 2);
            let code = &code[..end.min(code.len())];
            let name = code[..code.find('(').unwrap_or(0)].rsplit(' ').next().unwrap_or("").to_owned();
            out.push((name, code.to_owned()));
        }
    }
    assert!(out.iter().any(|(name, _)| name == "tempo_block"), "no tempo_block");
    out
}

/// The generated MSL of every class strategy, from its first line (helpers included).
#[cfg(target_os = "macos")]
fn metal_program_backend_generate(native: &native_ir::BlockNative, width: usize) -> Vec<(String, String)> {
    use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_with};
    let width = LaneWidth::ALL.into_iter().find(|w| w.bytes() == width).expect("a width");
    ClassStrategy::ALL
        .into_iter()
        .map(|s| {
            let msl = generate_with(native, width, s);
            let body = msl[msl.find("using namespace metal;").expect("the prelude")..].to_owned();
            (format!("{s:?}"), body)
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn metal_program_backend_generate(_native: &native_ir::BlockNative, _width: usize) -> Vec<(String, String)> {
    vec![]
}

#[test]
fn eq_lowers_to_straight_bit_operations() {
    check_lowering("eq", &program(EQ_QUOTE));
}

/// The interpreters run the same bit operations as the generated code, one operation at a time:
/// the CPU interpreter's kernel is a NEON compare and `to_bitmask`, the Metal interpreter's a
/// compare and a ballot. Neither loops over bytes.
#[test]
fn interpreters_have_no_byte_loops() {
    let cpu = include_str!("block.rs");
    let kernel = &cpu[cpu.find("pub(super) fn eq(block").expect("the eq kernel")..];
    let kernel = &kernel[..kernel.find("\n}\n").expect("its end")];
    for word in ["for ", "while ", "loop {", ".iter()", "[j]", "[i]"] {
        assert!(!kernel.contains(word), "the CPU eq kernel has `{word}`");
    }
    // The Metal interpreter's `run`: one loop, over operations; the carry helpers it calls have
    // none. Loops elsewhere are over groups (`block_carry_scan`).
    let metal = include_str!("metal_block_program.metal");
    let run = &metal[metal.find("static inline void run(").expect("run")..];
    let run = &run[..run.find("\n}\n").expect("its end")];
    assert_eq!(run.matches("for (").count(), 1, "the Metal interpreter's one loop is over operations");
    assert!(run.contains("for (uint at = p.from; at < p.to; at++)"));
    for word in ["while", "if (", "simd_ballot"] {
        assert!(!run.contains(word), "the Metal interpreter has `{word}`");
    }
    let carry = include_str!("metal_block_carry.metal");
    for word in ["for (", "while", "if (", "?"] {
        assert!(!carry.contains(word), "the carry helpers have `{word}`");
    }
}

// --- Primitives 2-7: const, not, and, or, xor, andnot ---------------------------------------

/// The quote and backslash masks the operator programs start from: `%0` and `%1`.
const QUOTE_AND_BACKSLASH: &str = "  block 0 %0 = eq 0x22\n  block 0 %1 = eq 0x5c\n";

fn quote(bytes: &[u8]) -> Vec<u64> {
    reference(bytes, |b| b == b'"')
}

fn backslash(bytes: &[u8]) -> Vec<u64> {
    reference(bytes, |b| b == b'\\')
}

/// `words` with the bits past `len` bytes cleared.
fn tail(mut words: Vec<u64>, len: usize) -> Vec<u64> {
    block::mask_tail(&mut words, len);
    words
}

fn zip(a: &[u64], b: &[u64], f: impl Fn(u64, u64) -> u64) -> Vec<u64> {
    a.iter().zip(b).map(|(&x, &y)| f(x, y)).collect()
}

#[test]
fn const_equals_reference_on_every_executor() {
    for c in [0xaaaa_aaaa_aaaa_aaaau64, 0x0123_4567_89ab_cdef, 0, u64::MAX, 1 << 63 | 1] {
        check_all(&format!("  block 0 %0 = const {c:#018x}\n  block 0 output %0\n"), |bytes| {
            vec![tail(vec![c; bytes.len().div_ceil(64)], bytes.len())]
        });
    }
}

#[test]
fn not_equals_reference_on_every_executor() {
    check_all("  block 0 %0 = eq 0x22\n  block 0 %1 = not %0\n  block 0 output %1\n", |bytes| {
        vec![tail(quote(bytes).iter().map(|w| !w).collect(), bytes.len())]
    });
}

#[test]
fn binary_operators_equal_reference_on_every_executor() {
    let cases: [(&str, fn(u64, u64) -> u64); 4] =
        [("and", |x, y| x & y), ("or", |x, y| x | y), ("xor", |x, y| x ^ y), ("andnot", |x, y| x & !y)];
    for (name, f) in cases {
        // Each operator on the two masks, and on a mask with itself.
        check_all(&format!("{QUOTE_AND_BACKSLASH}  block 0 %2 = {name} %0, %1\n  block 0 output %2\n"), |bytes| {
            vec![zip(&quote(bytes), &backslash(bytes), f)]
        });
        check_all(&format!("{QUOTE_AND_BACKSLASH}  block 0 %2 = {name} %0, %0\n  block 0 output %2\n"), |bytes| {
            vec![zip(&quote(bytes), &quote(bytes), f)]
        });
    }
}

/// Combinations through operators, with several outputs: simdjson's first lines, in the small.
/// `%2` is "a quote or a backslash", `%3` "neither" (and so set past the end before the tail is
/// cleared), `%4` "odd bytes that are a quote", `%5` everything but those.
#[test]
fn operator_combinations_equal_reference_on_every_executor() {
    let rows = format!(
        "{QUOTE_AND_BACKSLASH}  block 0 %2 = or %0, %1\n  block 0 %3 = not %2\n  \
         block 0 %4 = const 0xaaaaaaaaaaaaaaaa\n  block 0 %5 = and %0, %4\n  \
         block 0 %6 = xor %5, %3\n  block 0 %7 = andnot %6, %1\n  \
         block 0 output %2\n  block 0 output %3\n  block 0 output %7\n"
    );
    check_all(&rows, |bytes| {
        let (q, b) = (quote(bytes), backslash(bytes));
        let either = zip(&q, &b, |x, y| x | y);
        let neither: Vec<u64> = either.iter().map(|w| !w).collect();
        let odd_quote: Vec<u64> = q.iter().map(|w| w & 0xaaaa_aaaa_aaaa_aaaa).collect();
        let mixed = zip(&odd_quote, &neither, |x, y| x ^ y);
        let last = zip(&mixed, &b, |x, y| x & !y);
        vec![either, tail(neither, bytes.len()), tail(last, bytes.len())]
    });
}

#[test]
fn operators_lower_to_straight_bit_operations() {
    check_lowering(
        "operators",
        &program(&format!(
            "{QUOTE_AND_BACKSLASH}  block 0 %2 = or %0, %1\n  block 0 %3 = not %2\n  \
             block 0 %4 = const 0xaaaaaaaaaaaaaaaa\n  block 0 %5 = and %0, %4\n  \
             block 0 %6 = xor %5, %3\n  block 0 %7 = andnot %6, %1\n  block 0 output %7\n"
        )),
    );
}

#[test]
fn operand_refusals() {
    let refused = |rows: &str| {
        rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n"))
            .err()
            .unwrap_or_else(|| panic!("not refused: {rows}"))
    };
    // An operand that is not an earlier value.
    assert!(refused("  block 0 %0 = not %0\n  block 0 output %0\n").contains("verifier"));
    assert!(refused("  block 0 %0 = eq 0x22\n  block 0 %1 = and %0, %2\n  block 0 output %1\n").contains("verifier"));
    assert!(refused("  block 0 %0 = eq 0x22\n  block 0 %1 = and %0\n  block 0 output %1\n").contains("unknown block operation"));
}

// --- Primitive 8: class ---------------------------------------------------------------------

/// simdjson's table (`03-bitlex.md` §3.1): 0x01 `,`, 0x02 `:`, 0x04 `[]{}`, 0x08 space,
/// 0x10 `\t \n \r`, 0x20 `\`, 0x40 `"`, 0x80 controls
/// (every byte below 0x20, whitespace included, as in simdjson).
const SIMDJSON_TABLE: &str = "  block 0 table 0 lo=8880c0808080808080909284a1948080 hi=90804902002400040000000000000000\n";

/// The five masks simdjson's stage 1 classifies: structural operators, whitespace, backslash,
/// quote, and controls, all from one table.
fn simdjson_classes() -> String {
    format!(
        "{SIMDJSON_TABLE}  block 0 %0 = class t0 0x07\n  block 0 %1 = class t0 0x18\n  block 0 %2 = class t0 0x20\n\
         \x20 block 0 %3 = class t0 0x40\n  block 0 %4 = class t0 0x80\n\
         \x20 block 0 output %0\n  block 0 output %1\n  block 0 output %2\n  block 0 output %3\n  block 0 output %4\n"
    )
}

fn simdjson_want(bytes: &[u8]) -> Vec<Vec<u64>> {
    vec![
        reference(bytes, |b| b",:[]{}".contains(&b)),
        reference(bytes, |b| b" \t\n\r".contains(&b)),
        reference(bytes, |b| b == b'\\'),
        reference(bytes, |b| b == b'"'),
        reference(bytes, |b| b < 0x20),
    ]
}

#[test]
fn class_round_trips_through_rir() {
    let p = program(&simdjson_classes());
    let text = rir::disassemble(&p, None);
    assert!(text.contains("block 0 table 0 lo=8880c0808080808080909284a1948080 hi=90804902002400040000000000000000"), "{text}");
    assert!(text.contains("block 0 %0 = class t0 0x07"), "{text}");
    let again = rir::assemble(&text).expect("reassembles");
    assert_eq!(again.canonical_digest("-", b""), p.canonical_digest("-", b""));
}

#[test]
fn class_refusals() {
    for (rows, why) in [
        ("  block 0 %0 = class t0 0x01\n  block 0 output %0\n", "no table 0"),
        (&format!("{SIMDJSON_TABLE}  block 0 %0 = class t0 0x00\n  block 0 output %0\n") as &str, "no class bits"),
        ("  block 0 %0 = eq 0x22\n  block 0 table 0 lo=00000000000000000000000000000000 hi=00000000000000000000000000000000\n  block 0 output %0\n", "a table after an op"),
        ("  block 0 table 1 lo=00000000000000000000000000000000 hi=00000000000000000000000000000000\n  block 0 %0 = eq 0x22\n  block 0 output %0\n", "table 1 first"),
        ("  block 0 table 0 lo=00 hi=00\n  block 0 %0 = eq 0x22\n  block 0 output %0\n", "short table"),
    ] {
        let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
        assert!(rir::assemble(&text).is_err(), "{why}: accepted\n{text}");
    }
}

#[test]
fn class_equals_reference_on_every_executor() {
    check_all(&simdjson_classes(), simdjson_want);
}

/// Each class bit alone, and every pair, on a table of irregular classes (not products a table
/// was designed for): the formulas' constant folding and sharing are exercised on all of them.
#[test]
fn class_bits_equal_reference_on_every_executor() {
    let lo: [u8; 16] = [0x01, 0x82, 0x44, 0x08, 0x10, 0xff, 0x00, 0x21, 0x40, 0x80, 0x03, 0x0c, 0x30, 0xc0, 0x55, 0xaa];
    let hi: [u8; 16] = [0xff, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x00, 0x0f, 0xf0, 0x33, 0xcc, 0x5a, 0xa5];
    let table = block::ClassTable { lo, hi };
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let selectors: Vec<u8> = (0..8).map(|c| 1u8 << c).chain([0x03, 0x81, 0xff, 0x5a]).collect();
    let mut rows = format!("  block 0 table 0 lo={} hi={}\n", hex(&lo), hex(&hi));
    for (i, s) in selectors.iter().enumerate() {
        rows.push_str(&format!("  block 0 %{i} = class t0 {s:#04x}\n"));
    }
    for i in 0..selectors.len() {
        rows.push_str(&format!("  block 0 output %{i}\n"));
    }
    check_all(&rows, |bytes| selectors.iter().map(|&s| reference(bytes, |b| table.member(b, s))).collect());
}

/// `pair`: the byte before's class bytes AND the byte's, on irregular tables, for each class
/// bit, several selections and both orders, beside a `class` of the same table (the lookups are
/// shared); in the planes form too, through Cranelift. The byte before the stream is 0x00.
#[test]
fn pair_equals_reference_on_every_executor() {
    let a = block::ClassTable {
        lo: [0x01, 0x82, 0x44, 0x08, 0x10, 0xff, 0x00, 0x21, 0x40, 0x80, 0x03, 0x0c, 0x30, 0xc0, 0x55, 0xaa],
        hi: [0xff, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x00, 0x0f, 0xf0, 0x33, 0xcc, 0x5a, 0xa5],
    };
    let b = block::ClassTable { lo: a.hi, hi: a.lo };
    let tables = [a, b];
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let mut cases: Vec<(u16, u16, u8)> = (0..8).map(|c| (0, 1, 1u8 << c)).collect();
    cases.extend([(0, 1, 0xff), (1, 0, 0x5a), (0, 0, 0x81), (1, 1, 0x7f)]);
    let mut rows = format!("  block 0 table 0 lo={} hi={}\n  block 0 table 1 lo={} hi={}\n", hex(&a.lo), hex(&a.hi), hex(&b.lo), hex(&b.hi));
    for (i, (p, t, bits)) in cases.iter().enumerate() {
        rows.push_str(&format!("  block 0 %{i} = pair t{p} t{t} {bits:#04x}\n"));
    }
    rows.push_str(&format!("  block 0 %{} = class t1 0x0f\n", cases.len()));
    for i in 0..=cases.len() {
        rows.push_str(&format!("  block 0 output %{i}\n"));
    }
    let classes = |t: &block::ClassTable, x: u8| t.lo[usize::from(x & 15)] & t.hi[usize::from(x >> 4)];
    let want = |bytes: &[u8]| -> Vec<Vec<u64>> {
        let mut out: Vec<Vec<u64>> = cases
            .iter()
            .map(|&(p, t, bits)| {
                let hits: Vec<bool> = (0..bytes.len())
                    .map(|j| {
                        let before = if j == 0 { 0 } else { bytes[j - 1] };
                        classes(&tables[usize::from(p)], before) & classes(&tables[usize::from(t)], bytes[j]) & bits != 0
                    })
                    .collect();
                words_of(&hits)
            })
            .collect();
        out.push(reference(bytes, |x| b.member(x, 0x0f)));
        out
    };
    check_all(&rows, want);
    let p = program(&rows);
    let planes = native_ir::lower(&p).expect("lowers").block_programs.remove(0).planes_form();
    assert!(!planes.insts.iter().any(|i| matches!(i, native_ir::BlockInst::Pair { .. })));
    let compiled = native_cpu_block::compile(&planes).expect("compiles");
    for bytes in inputs() {
        assert_eq!(compiled.run(&bytes), want(&bytes), "planes form, {} bytes", bytes.len());
    }
}

/// The planes form (`class` as formulas of bit-planes, which Metal runs) equals the reference on
/// the CPU too, through Cranelift; it has no `class` left, and the Metal interpreter reads no
/// table.
#[test]
fn class_planes_form_equals_reference() {
    let p = program(&simdjson_classes());
    let planes = native_ir::lower(&p).expect("lowers").block_programs.remove(0).planes_form();
    assert!(!planes.insts.iter().any(|i| matches!(i, native_ir::BlockInst::Class { .. })));
    println!("planes form: {} instructions for 5 classes", planes.insts.len());
    let compiled = native_cpu_block::compile(&planes).expect("compiles");
    for bytes in inputs() {
        assert_eq!(compiled.run(&bytes), simdjson_want(&bytes), "{} bytes", bytes.len());
    }
    let metal = include_str!("metal_block_program.metal");
    assert!(!metal.contains("constant uchar") && !metal.contains("tables"), "the Metal interpreter reads a table");
}

#[test]
fn class_lowers_to_straight_bit_operations() {
    check_lowering("class", &program(&simdjson_classes()));
}

/// Lookup against bit-planes for `class` on Metal: simdjson's five masks over 64 MiB of
/// JSON-like bytes, per strategy and lane width, in GPU seconds (best of 5), with five `eq`s as
/// the baseline. Run by name: it is a measurement, not a check.
#[test]
#[ignore = "a measurement; run by name"]
#[cfg(target_os = "macos")]
fn class_strategies_gpu_time() {
    use metal_program_backend::block_program::{ClassStrategy, LaneWidth, run_generated_with};
    let sample = br#"{"name": "rerail", "tags": ["a", "b\"c"], "n": 12345, "ok": true, "nested": {"x": [1, 2, 3]}}
"#;
    let bytes: Vec<u8> = sample.iter().copied().cycle().take(64 << 20).collect();
    let p = program(&simdjson_classes());
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let eqs = program(
        "  block 0 %0 = eq 0x2c\n  block 0 %1 = eq 0x20\n  block 0 %2 = eq 0x5c\n  block 0 %3 = eq 0x22\n  block 0 %4 = eq 0x0a\n\
         \x20 block 0 output %0\n  block 0 output %1\n  block 0 output %2\n  block 0 output %3\n  block 0 output %4\n",
    );
    let eq_native = native_ir::lower(&eqs).expect("lowers").block_programs.remove(0);
    let want = simdjson_want(&bytes);
    let gib = bytes.len() as f64 / f64::from(1u32 << 30);
    for width in LaneWidth::ALL {
        let best = |native: &native_ir::BlockNative, strategy| {
            (0..5).map(|_| run_generated_with(native, &bytes, width, strategy).expect("runs").1).fold(f64::MAX, f64::min)
        };
        for strategy in ClassStrategy::ALL {
            let (got, _) = run_generated_with(&native, &bytes, width, strategy).expect("runs");
            assert!(got == want, "{strategy:?} at {} bytes differs", width.bytes());
            let s = best(&native, strategy);
            println!("class {strategy:?}, {:2}-byte lanes: {:.3} ms, {:.1} GiB/s", width.bytes(), s * 1e3, gib / s);
        }
        let s = best(&eq_native, ClassStrategy::Lookup);
        println!("5 eqs (baseline), {:2}-byte lanes: {:.3} ms, {:.1} GiB/s", width.bytes(), s * 1e3, gib / s);
    }
}

// --- Primitives 9-11: prev, toggle, after_odd_run (carried) ---------------------------------

/// The byte-by-byte references of the carried operations, over a mask of bytes.
fn bits(bytes: &[u8], f: impl Fn(u8) -> bool) -> Vec<bool> {
    bytes.iter().map(|&b| f(b)).collect()
}

fn words_of(bits: &[bool]) -> Vec<u64> {
    let mut words = vec![0u64; bits.len().div_ceil(64)];
    for (j, &b) in bits.iter().enumerate() {
        words[j / 64] |= u64::from(b) << (j % 64);
    }
    words
}

fn prev_reference(a: &[bool], k: usize) -> Vec<bool> {
    (0..a.len()).map(|j| j >= k && a[j - k]).collect()
}

fn toggle_reference(a: &[bool]) -> Vec<bool> {
    let mut parity = false;
    a.iter().map(|&x| { parity ^= x; parity }).collect()
}

fn after_odd_run_reference(a: &[bool]) -> Vec<bool> {
    let mut run = 0usize;
    a.iter()
        .map(|&x| {
            let after = !x && run % 2 == 1;
            run = if x { run + 1 } else { 0 };
            after
        })
        .collect()
}

#[test]
fn carried_round_trip_and_refusals() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 3\n  block 0 %2 = toggle %0\n  block 0 %3 = after_odd_run %0\n  block 0 output %1\n  block 0 output %2\n  block 0 output %3\n";
    let p = program(rows);
    let text = rir::disassemble(&p, None);
    for line in ["%1 = prev %0, 3", "%2 = toggle %0", "%3 = after_odd_run %0"] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert_eq!(rir::assemble(&text).expect("reassembles").canonical_digest("-", b""), p.canonical_digest("-", b""));
    for (rows, why) in [
        ("  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 0\n  block 0 output %1\n", "distance 0"),
        ("  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 64\n  block 0 output %1\n", "distance 64"),
        ("  block 0 %0 = toggle %0\n  block 0 output %0\n", "toggle of itself"),
        ("  block 0 %0 = after_odd_run %1\n  block 0 output %0\n", "a later operand"),
    ] {
        let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
        assert!(rir::assemble(&text).is_err(), "{why}: accepted\n{text}");
    }
}

#[test]
fn prev_equals_reference_on_every_executor() {
    for k in [1usize, 2, 7, 15, 16, 17, 31, 32, 33, 48, 63] {
        let rows = format!("  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, {k}\n  block 0 output %1\n");
        check_all(&rows, |bytes| vec![words_of(&prev_reference(&bits(bytes, |b| b == b'\\'), k))]);
    }
}

#[test]
fn toggle_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x22\n  block 0 %1 = toggle %0\n  block 0 output %1\n";
    check_all(rows, |bytes| vec![words_of(&toggle_reference(&bits(bytes, |b| b == b'"')))]);
}

#[test]
fn after_odd_run_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = after_odd_run %0\n  block 0 output %1\n";
    check_all(rows, |bytes| vec![words_of(&after_odd_run_reference(&bits(bytes, |b| b == b'\\')))]);
}

/// Carried operations in a chain, each round's carry depending on the one before: the escaped
/// quotes removed before the toggle, then a shift of the result.
#[test]
fn carried_chain_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = after_odd_run %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 %5 = prev %4, 1\n  block 0 output %4\n  block 0 output %5\n";
    check_all(rows, |bytes| {
        let escaped = after_odd_run_reference(&bits(bytes, |b| b == b'\\'));
        let quote: Vec<bool> = bytes.iter().zip(&escaped).map(|(&b, &e)| b == b'"' && !e).collect();
        let inside = toggle_reference(&quote);
        vec![words_of(&inside), words_of(&prev_reference(&inside, 1))]
    });
}

/// The look-back's fallback: polling no times, every group computes its predecessors' aggregates
/// itself, and the outputs are the same. The second program needs two rounds (a `toggle` of a
/// halo the first round publishes), so the fallback also recomputes an earlier round's carries.
#[test]
#[cfg(target_os = "macos")]
fn lookback_fallback_equals_reference() {
    use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_polling, rounds, run_source};
    let one = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = after_odd_run %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 output %4\n";
    let two = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = after_odd_run %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 %5 = prev %4, 5\n  block 0 %6 = toggle %5\n  block 0 output %4\n  block 0 output %6\n";
    for (rows, want_rounds) in [(one, 1), (two, 2)] {
        let p = program(rows);
        let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
        assert_eq!(rounds(&native), want_rounds, "rounds of\n{rows}");
        for bytes in runs() {
            let escaped = after_odd_run_reference(&bits(&bytes, |b| b == b'\\'));
            let quote: Vec<bool> = bytes.iter().zip(&escaped).map(|(&b, &e)| b == b'"' && !e).collect();
            let inside = toggle_reference(&quote);
            let mut want = vec![words_of(&inside)];
            if want_rounds == 2 {
                want.push(words_of(&toggle_reference(&prev_reference(&inside, 5))));
            }
            for width in LaneWidth::ALL {
                for spins in [0, 64] {
                    let source = generate_polling(&native, width, ClassStrategy::Planes, spins);
                    let (got, _) = run_source(&native, &source, &bytes, width).expect("runs");
                    assert_eq!(got.outputs, want, "{} bytes, {}-byte lanes, {spins} polls", bytes.len(), width.bytes());
                }
            }
        }
    }
}

/// Stage 1 on generated MSL is one round (its two scanned carries and the halo of
/// `nonquote_scalar` fused) and the final pass: two dispatches.
#[test]
#[cfg(target_os = "macos")]
fn stage1_is_one_round() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    assert_eq!(metal_program_backend::block_program::rounds(&native), 1);
}

#[test]
fn carried_lower_to_straight_bit_operations() {
    check_lowering(
        "carried",
        &program("  block 0 %0 = eq 0x5c\n  block 0 %1 = after_odd_run %0\n  block 0 %2 = toggle %1\n  block 0 %3 = prev %2, 1\n  block 0 output %3\n"),
    );
}

// --- simdjson's stage 1 ----------------------------------------------------------------------

/// Stage 1, written by hand in `rir` (`fixtures/simdjson-stage1/stage1.rir`).
const STAGE1: &str = include_str!("../../../../../fixtures/simdjson-stage1/stage1.rir");

/// simdjson's own stage 1 on each file (`fixtures/simdjson-stage1/stage1_indexes.cpp`): path,
/// length, number of structural indexes, FNV-1a of the indexes as little-endian u32s.
const STAGE1_EXPECTED: &str = include_str!("../../../../../fixtures/simdjson-stage1/expected.txt");

fn repo_file(path: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The set bits of a mask over `len` bytes, as byte indexes.
fn indexes(words: &[u64]) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, &w) in words.iter().enumerate() {
        let mut w = w;
        while w != 0 {
            out.push(i as u32 * 64 + w.trailing_zeros());
            w &= w - 1;
        }
    }
    out
}

fn fnv(indexes: &[u32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for v in indexes {
        for b in v.to_le_bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Every executor's run with the extract pass: the outputs and each extract's positions.
fn extract_executors(p: &Program, bytes: &[u8]) -> Vec<(String, BlockRun)> {
    let block = &p.block_programs()[0];
    let native = native_ir::lower(p).expect("lowers").block_programs.remove(0);
    let mut out = vec![
        ("cpu interpreter".to_owned(), block.run(bytes)),
        ("cranelift".to_owned(), native_cpu_block::compile(&native).expect("compiles").extract(bytes)),
    ];
    #[cfg(target_os = "macos")]
    {
        use metal_program_backend::block_program::{ClassStrategy, LaneWidth, interpret_extract, run_generated_extract};
        for width in LaneWidth::ALL {
            let w = width.bytes();
            out.push((format!("metal interpreter, {w}-byte lanes"), interpret_extract(block, bytes, width).expect("runs")));
            let (got, _) = run_generated_extract(&native, bytes, width, ClassStrategy::Planes).expect("runs");
            out.push((format!("generated msl, {w}-byte lanes"), got));
        }
    }
    out
}

/// The extract pass writes a plane's set bits as positions, none past the end, on every
/// executor: an all-ones plane (every chunk full, the tail cut at every length), a plane of
/// quotes, and an empty plane.
#[test]
fn extract_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = const 0xffffffffffffffff\n  block 0 %1 = eq 0x22\n  block 0 %2 = eq 0x01\n\
                \x20 block 0 output %0\n  block 0 output %1\n  block 0 output %2\n\
                \x20 block 0 extract %0\n  block 0 extract %1\n  block 0 extract %2\n";
    let p = program(rows);
    let mut cases = inputs();
    cases.extend([4095usize, 4096, 4097, 9000].map(|n| vec![b'"'; n]));
    for bytes in cases {
        let ones: Vec<u32> = (0..bytes.len() as u32).collect();
        let quotes: Vec<u32> = (0..bytes.len() as u32).filter(|&i| bytes[i as usize] == b'"').collect();
        let soh: Vec<u32> = (0..bytes.len() as u32).filter(|&i| bytes[i as usize] == 0x01).collect();
        let want = vec![ones.clone(), quotes.clone(), soh.clone()];
        for (name, got) in extract_executors(&p, &bytes) {
            assert_eq!(got.positions, want, "{name}, {} bytes", bytes.len());
        }
        // No polls: every group but the first defers, and the fallback (scan, then scatter)
        // writes the positions.
        #[cfg(target_os = "macos")]
        {
            use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_polling, run_source};
            let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
            for width in LaneWidth::ALL {
                let source = generate_polling(&native, width, ClassStrategy::Planes, 0);
                let (got, _) = run_source(&native, &source, &bytes, width).expect("runs");
                assert_eq!(got.positions, want, "no polls, {}-byte lanes, {} bytes", width.bytes(), bytes.len());
            }
        }
    }
}

#[test]
fn extract_round_trips_and_refusals() {
    let text = "program -\n  mode set=0 query=0\n  block 0 %0 = eq 0x22\n  block 0 %1 = eq 0x5c\n\
                \x20 block 0 output %0\n  block 0 output %1\n  block 0 extract %1\n  0000 STOP 0\n";
    let p = rir::assemble(text).unwrap_or_else(|e| panic!("{e}"));
    let dump = rir::disassemble(&p, None);
    assert!(dump.contains("block 0 extract %1"), "{dump}");
    assert_eq!(rir::assemble(&dump).expect("reassembles").plain_digest(), p.plain_digest());
    let without = rir::assemble(&text.replace("  block 0 extract %1\n", "")).unwrap();
    assert_ne!(without.plain_digest(), p.plain_digest(), "the digest covers extracts");
    let refused = |rows: &str| {
        rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n"))
            .err()
            .unwrap_or_else(|| panic!("not refused: {rows}"))
    };
    // An extract is an output; outputs come before extracts.
    assert!(refused("  block 0 %0 = eq 0x22\n  block 0 %1 = eq 0x5c\n  block 0 output %0\n  block 0 extract %1\n").contains("verifier"));
    assert!(refused("  block 0 %0 = eq 0x22\n  block 0 output %0\n  block 0 extract %0\n  block 0 output %0\n").contains("after an extract"));
}

/// Stage 1's three outputs on `bytes` are simdjson's: the same structural indexes, no control
/// character inside a string, and no unclosed string.
fn check_stage1(name: &str, bytes: &[u8], outputs: &[Vec<u64>], want: (usize, u64)) {
    let found = indexes(&outputs[0]);
    assert_eq!((found.len(), fnv(&found)), want, "{name}: structural indexes differ from simdjson's");
    assert!(outputs[1].iter().all(|&w| w == 0), "{name}: a control character inside a string");
    let last = bytes.len() - 1;
    assert_eq!(outputs[2][last / 64] >> (last % 64) & 1, 0, "{name}: an unclosed string");
    assert!(!utf8_invalid(bytes, outputs), "{name}: invalid UTF-8");
}

/// Stage 1's UTF-8 verdict: an error bit anywhere, or a sequence open at the last byte.
fn utf8_invalid(bytes: &[u8], outputs: &[Vec<u64>]) -> bool {
    let open = bytes.len().checked_sub(1).is_some_and(|last| outputs[4][last / 64] >> (last % 64) & 1 == 1);
    outputs[3].iter().any(|&w| w != 0) || open
}

/// Inputs around every UTF-8 rule: each boundary character, overlong forms, surrogates, values
/// past U+10FFFF, stray continuations and truncated leads, at offsets across block and lane
/// edges and at the very end.
fn utf8_inputs() -> Vec<Vec<u8>> {
    let atoms: Vec<&[u8]> = vec![
        b"a", b"\x7f", "\u{80}".as_bytes(), "\u{7ff}".as_bytes(), "\u{800}".as_bytes(), "\u{fffd}".as_bytes(),
        "\u{ffff}".as_bytes(), "\u{10000}".as_bytes(), "\u{10ffff}".as_bytes(), "\u{d7ff}".as_bytes(), "\u{e000}".as_bytes(),
        b"\xc0\x80", b"\xc1\xbf", b"\xe0\x80\x80", b"\xe0\x9f\xbf", b"\xf0\x80\x80\x80", b"\xf0\x8f\xbf\xbf",
        b"\xed\xa0\x80", b"\xed\xbf\xbf", b"\xf4\x90\x80\x80", b"\xf5\x80\x80\x80", b"\xff", b"\xfe",
        b"\x80", b"\xbf", b"\xc2", b"\xe0\xa0", b"\xf0\x90\x80", b"\xc2\xc2\x80", b"\xe1\x80\xc0",
    ];
    let mut out = Vec::new();
    for (i, atom) in atoms.iter().enumerate() {
        for pad in [0usize, 1, 13, 14, 15, 30, 31, 61, 62, 63, 64, 125, 126, 127] {
            let mut v = vec![b' '; pad];
            v.extend_from_slice(atom);
            out.push(v.clone());
            v.extend_from_slice(b" x");
            v.extend_from_slice(atoms[(i * 7 + pad) % atoms.len()]);
            v.extend(std::iter::repeat_n(b'y', pad % 5));
            out.push(v);
        }
    }
    // Long valid text with every length of character, then one error deep inside.
    let text: Vec<u8> = "a\u{e9}\u{4e2d}\u{1f600}".repeat(700).into_bytes();
    out.push(text.clone());
    let mut broken = text;
    broken[5000] = 0x80;
    out.push(broken);
    out
}

#[test]
fn stage1_utf8_equals_std_on_every_executor() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    for bytes in utf8_inputs() {
        let want = std::str::from_utf8(&bytes).is_err();
        for (name, got) in executors(&p, &bytes) {
            assert_eq!(utf8_invalid(&bytes, &got), want, "{name}: {:x?}", &bytes[bytes.len().saturating_sub(12)..]);
        }
    }
}

fn stage1_files() -> Vec<(String, usize, (usize, u64))> {
    STAGE1_EXPECTED
        .lines()
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            (f[0].to_owned(), f[1].parse().unwrap(), (f[2].parse().unwrap(), u64::from_str_radix(f[3], 16).unwrap()))
        })
        .collect()
}

/// Every executor finds simdjson's structural indexes. Files up to 200 KB run on all of them;
/// the 3.8 MB file on Cranelift and generated MSL.
#[test]
fn stage1_equals_simdjson_on_every_executor() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    for (path, len, want) in stage1_files() {
        let bytes = repo_file(&path);
        assert_eq!(bytes.len(), len, "{path} changed since simdjson's indexes were recorded");
        let runs: Vec<(String, BlockRun)> = if len < 200_000 {
            extract_executors(&p, &bytes)
        } else {
            let mut runs = vec![("cranelift".to_owned(), compiled.extract(&bytes))];
            #[cfg(target_os = "macos")]
            {
                use metal_program_backend::block_program::{ClassStrategy, LaneWidth, run_generated_extract};
                let (got, _) = run_generated_extract(&native, &bytes, LaneWidth::W64, ClassStrategy::Planes).expect("runs");
                runs.push(("generated msl".to_owned(), got));
            }
            runs
        };
        for (name, got) in runs {
            check_stage1(&format!("{path}, {name}"), &bytes, &got.outputs, want);
            // The extract pass's positions are simdjson's structural indexes.
            let found = &got.positions[0];
            assert_eq!((found.len(), fnv(found)), want, "{path}, {name}: extracted indexes differ from simdjson's");
        }
    }
}

#[test]
fn stage1_lowers_to_straight_bit_operations() {
    check_lowering("stage 1", &rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}")));
}

/// Stage 1's cost per 64-byte block: the JIT's CPU time and machine instructions per block, and
/// generated MSL's GPU time per lane width, on the 3.8 MB file. simdjson's time on the same file
/// comes from `stage1_indexes --time`. Run by name: a measurement, not a check.
#[test]
#[ignore = "a measurement; run by name"]
fn stage1_cost_per_block() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let (path, _, _) = stage1_files().into_iter().last().expect("the large file");
    let bytes = repo_file(&path);
    let blocks = bytes.len().div_ceil(64) as f64;
    let body: Vec<&str> = compiled.disassembly.lines().collect();
    let start = body.iter().position(|l| l.trim_start().starts_with("block1:")).unwrap_or(0);
    let end = body.iter().rposition(|l| l.contains("b.") || l.contains("b.lo") || l.contains("b.hs")).unwrap_or(body.len());
    let per_block = body[start..=end.min(body.len() - 1)].iter().filter(|l| l.starts_with("  ")).count();
    println!("{path}: {} bytes, {blocks} blocks", bytes.len());
    println!("cranelift: {per_block} machine instructions in the loop body (one block)");
    let s = compiled.cpu_seconds(&bytes, 20);
    println!("cranelift: {:.3} ns per block (CPU time, best of 20)", s / blocks * 1e9);
    #[cfg(target_os = "macos")]
    {
        use metal_program_backend::block_program::{ClassStrategy, LaneWidth, run_generated_with};
        // The mask passes alone, and with the extract pass (in the same dispatch unless a
        // look-back runs out of polls; the fallback's second command buffer is not timed, and
        // its runs are counted).
        let mut masks = native.clone();
        masks.extracts.clear();
        use metal_program_backend::block_program::EXTRACT_FALLBACKS;
        let before = EXTRACT_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed);
        for (what, program) in [("masks", &masks), ("masks + extract", &native)] {
            for strategy in ClassStrategy::ALL {
                for width in LaneWidth::ALL {
                    let s = (0..5)
                        .map(|_| run_generated_with(program, &bytes, width, strategy).expect("runs").1)
                        .fold(f64::MAX, f64::min);
                    println!(
                        "generated msl, {what}, {strategy:?}, {:2}-byte lanes: {:.3} ns per block (GPU time, all rounds, best of 5)",
                        width.bytes(),
                        s / blocks * 1e9
                    );
                }
            }
        }
        let fallbacks = EXTRACT_FALLBACKS.load(std::sync::atomic::Ordering::Relaxed) - before;
        println!("extract fallbacks (second command buffer): {fallbacks} of 30 runs with the extract");
    }
}
