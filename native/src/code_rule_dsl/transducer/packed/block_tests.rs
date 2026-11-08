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
pub(super) fn inputs() -> Vec<Vec<u8>> {
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
pub(super) fn runs() -> Vec<Vec<u8>> {
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
pub(super) fn executors(p: &Program, bytes: &[u8]) -> Vec<(String, Vec<Vec<u64>>)> {
    let (out, skipped) = Executors::new(p).run(bytes, true);
    assert!(skipped.is_empty(), "over the launch budget: {skipped:?}");
    out
}

/// A program's first block program, lowered and compiled once, for runs over many inputs.
pub(super) struct Executors<'a> {
    block: &'a block::BlockProgram,
    native: native_ir::BlockNative,
    cranelift: native_cpu_block::CompiledBlock,
}

impl<'a> Executors<'a> {
    pub(super) fn new(p: &'a Program) -> Self {
        let native = native_ir::lower(p).expect("lowers").block_programs.remove(0);
        let cranelift = native_cpu_block::compile(&native).expect("compiles");
        Executors { block: &p.block_programs()[0], native, cranelift }
    }

    /// Every executor's outputs over `bytes` (the Metal ones when `metal`), without a Metal
    /// executor whose run would take more launches than the budget (`metal_launch_budget`):
    /// those are named in the second list.
    pub(super) fn run(&self, bytes: &[u8], metal: bool) -> (Vec<(String, Vec<Vec<u64>>)>, Vec<String>) {
        let mut out = vec![("cpu interpreter".to_owned(), self.block.interpret(bytes)), ("cranelift".to_owned(), self.cranelift.run(bytes))];
        let mut skipped = Vec::new();
        #[cfg(target_os = "macos")]
        if metal {
            use metal_program_backend::block_program::{ClassStrategy, LaneWidth, interpret, run_generated_with};
            let mut keep = |name: String, got: Result<Vec<Vec<u64>>, Error>| match got {
                Ok(got) => out.push((name, got)),
                Err(Error::BackendLaunchLimit) => skipped.push(name),
                Err(e) => panic!("{name} runs: {e:?}"),
            };
            for width in LaneWidth::ALL {
                let w = width.bytes();
                keep(format!("metal interpreter, {w}-byte lanes"), interpret(self.block, bytes, width));
                for strategy in ClassStrategy::ALL {
                    keep(format!("generated msl, {w}-byte lanes, {strategy:?}"), run_generated_with(&self.native, bytes, width, strategy).map(|(got, _)| got));
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = metal;
        (out, skipped)
    }
}

/// Check a program's outputs against references on every input and every executor.
fn check_all(rows: &str, want: impl Fn(&[u8]) -> Vec<Vec<u64>>) {
    let p = program(rows);
    for bytes in inputs() {
        let want = want(&bytes);
        // Every executor that disagrees, so one run shows where a lowering is wrong.
        let wrong: Vec<String> = executors(&p, &bytes).into_iter().filter(|(_, got)| *got != want).map(|(name, _)| name).collect();
        assert!(wrong.is_empty(), "{} bytes: wrong on {wrong:?}, program:\n{rows}", bytes.len());
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
/// block's. `block0` is the entry, `block1` a file's row, `block2` the body over its blocks.
fn body_of(clif: &str) -> Vec<String> {
    hot_loop(clif).into_iter().flat_map(|(_, lines)| lines).collect()
}

/// The CLIF blocks of the hot loop, each with its instruction lines: the full body (`block2`) and
/// the blocks it reaches before it leaves for the head (`block4`) or loops back to itself, which
/// are its extracts' flattening.
pub(super) fn hot_loop(clif: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for line in clif.lines() {
        let t = line.trim();
        if t.starts_with("block") && t.ends_with(':') {
            let name = t.split(['(', ':']).next().unwrap_or("").to_owned();
            blocks.push((name, Vec::new()));
        } else if let Some((_, lines)) = blocks.last_mut()
            && !t.is_empty()
        {
            lines.push(t.to_owned());
        }
    }
    let successors = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| matches!(opcode(l), "brif" | "jump"))
            .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()).filter(|w| w.starts_with("block")).map(str::to_owned).collect::<Vec<_>>())
            .collect()
    };
    let (mut seen, mut work, mut out) = (vec!["block2".to_owned()], vec!["block2".to_owned()], Vec::new());
    while let Some(name) = work.pop() {
        let (_, lines) = blocks.iter().find(|(n, _)| *n == name).expect("a block of the function");
        for next in successors(lines) {
            if next != "block4" && !seen.contains(&next) {
                seen.push(next.clone());
                work.push(next);
            }
        }
        out.push((name, lines.clone()));
    }
    out
}

/// The sinks' loop-carried values (`native_cpu_block`): a running count per counted output and
/// a held word per delayed output.
pub(super) fn sink_params(native: &native_ir::BlockNative) -> usize {
    native_cpu_block::counted(native).len() + native.output_delays.iter().filter(|&&d| d > 0).count()
}

/// The CLIF opcode of an instruction line (`v3 = icmp eq v1, v2` → `icmp`).
fn opcode(line: &str) -> &str {
    let line = line.split(" ;").next().unwrap_or(line);
    let rhs = line.split_once(" = ").map_or(line, |(_, r)| r);
    rhs.split([' ', '.']).next().unwrap_or("")
}

/// The opcodes a block body may contain: loads and stores of the block, bit operations (the
/// bitmask's pairwise adds included), the block's address arithmetic, and the one branch back to
/// the loop's head.
const BODY_OPCODES: &[&str] = &[
    "load", "store", "icmp", "icmp_imm", "vhigh_bits", "uextend", "ishl_imm", "bor", "band", "bxor",
    "bnot", "band_not", "iadd", "iadd_imm", "iconst", "brif", "iadd_pairwise", "bitcast", "extractlane",
    "swizzle", "ushr_imm", "sshr_imm", "uadd_overflow", "bor_not",
    // `within`: the bits below the stream's end, from its length (clamp, two shifts).
    "isub", "smin", "smax", "ishl",
    // The extract pass's size: each extract's popcount, summed as the plane is written.
    "popcnt",
    // Constants, made where they are used (decision 0047): `movi` and literal-pool loads, and
    // the output words' one pointer (`i * outputs * 8`).
    "splat", "vconst", "imul_imm",
    // The packed carried state (decision 0048): a field's bits read with a mask.
    "band_imm",
    // The extracts' flattening (simdjson's): a position per set bit, eight at a time.
    "ctz", "istore32", "icmp_imm", "jump",
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
    // The loop's branch, and two per extract: more than eight bits in the block, and the loop
    // over them eight at a time, which steps its count by 8 and loops on itself.
    let branches = body.iter().filter(|l| opcode(l) == "brif").count();
    assert_eq!(branches, 1 + 2 * native.extracts.len(), "{name}: the body's only branches are the loop's and the flattening's");
    for (block, lines) in hot_loop(&compiled.clif).iter().skip(1) {
        if lines.iter().any(|l| opcode(l) == "brif" && l.contains(&format!("{block}("))) {
            assert!(lines.iter().any(|l| opcode(l) == "iadd_imm" && l.split(" ;").next().is_some_and(|l| l.ends_with(", 8"))), "{name}: {block} loops, not eight bits at a time");
        }
    }
    println!("== {name}: machine code ==\n{}", compiled.disassembly);
    assert!(!compiled.disassembly.contains(" bl ") && !compiled.disassembly.contains("blr"), "{name}: no calls");

    let instructions = compiled.disassembly.lines().filter(|l| l.starts_with("  ") && !l.contains("unwind")).count();
    println!("== {name}: {instructions} machine instructions in the function ==");
    for width in [16usize, 32, 64] {
        let carried = native.insts.iter().any(native_ir::BlockInst::carried);
        for (strategy, msl) in metal_program_backend_generate(&native, width) {
            println!("== {name}: generated MSL, {width}-byte lanes, {strategy} ==\n{msl}");
            // The lane code (the carry helpers and each program's function) is straight-line;
            // only the look-backs loop, over predecessor groups, and the workers' loop takes
            // groups. A program's only branches leave the group unfinished when a look-back has
            // to wait for another launch: the same in every lane.
            for (section, code) in lane_code(&msl) {
                let code: String = code.lines().filter(|l| !CONTROL.iter().any(|c| l.trim_start().starts_with(c))).map(|l| format!("{l}\n")).collect();
                for word in ["for (", "while", "switch", "goto", "?", "if (", "break"] {
                    assert!(!code.contains(word), "{name}: generated MSL ({strategy}), {section}, has `{word}`");
                }
                // SIMD-group operations only combine carries: none without a carried instruction.
                if !carried && section.starts_with("program_") {
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

/// A program function's control lines: each leaves the group unfinished, the same in every lane.
const CONTROL: [&str; 3] = ["if (!look_back(", "if (!halo_exchange(", "if (!extract_write("];

/// The generated MSL's lane code: every function (the carry helpers, `halo_window_P`, `load16`,
/// and each `program_P`). The look-backs, which loop over predecessor groups, and the workers'
/// kernel, which loops over groups, are left out.
fn lane_code(msl: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for marker in ["static inline "] {
        for (at, _) in msl.match_indices(marker) {
            let code = &msl[at..];
            let end = code.find("\n}\n").map_or(code.find('\n').unwrap_or(code.len()), |e| e + 2);
            let code = &code[..end.min(code.len())];
            let name = code[..code.find('(').unwrap_or(0)].rsplit(' ').next().unwrap_or("").to_owned();
            if ["look_back", "halo_exchange"].contains(&name.as_str()) {
                continue;
            }
            out.push((name, code.to_owned()));
        }
    }
    assert!(out.iter().any(|(name, _)| name == "program_0"), "no program_0");
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
    // The Metal interpreter's `interpret_file`: one loop over operations, and loops over the
    // program's outputs and extracts (sizes); a lane's chunks are a loop over `sizeof(M) / 2`.
    // Its only branches leave the group unfinished when a look-back has to wait.
    let metal = include_str!("metal_block_program.metal");
    let run = &metal[metal.find("static bool interpret_file(").expect("interpret_file")..];
    let run = &run[..run.find("\n}\n").expect("its end")];
    let loops: Vec<&str> = run.lines().filter(|l| l.contains("for (")).map(str::trim).collect();
    assert_eq!(loops.len(), 3, "{loops:?}");
    assert!(loops[0].starts_with("for (uint at = 0u; at < pr.steps; at++)"), "{loops:?}");
    assert!(loops[1].starts_with("for (uint k = 0u; k < pr.outs; k++)") && loops[2].starts_with("for (uint e = 0u; e < pr.exts; e++)"), "{loops:?}");
    for word in ["while", "simd_ballot"] {
        assert!(!run.contains(word), "the Metal interpreter has `{word}`");
    }
    for line in run.lines().filter(|l| l.contains("if (")) {
        let line = line.trim();
        assert!(["if (!halo_exchange(", "if (!look_back(", "if (!extract_write("].iter().any(|c| line.starts_with(c)), "the Metal interpreter branches: `{line}`");
    }
    for helper in ["eq_file", "plane_file"] {
        let code = &metal[metal.find(&format!("M {helper}(")).expect("helper")..];
        let code = &code[..code.find("\n}\n").expect("its end")];
        assert!(code.contains("for (uint c = 0u; c < uint(sizeof(M)) / 2u; c++)"), "{helper} loops over chunks only");
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

// --- Primitives 9-11: prev, toggle, escaped (carried) ---------------------------------

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

/// simdjson's escaped bytes: each byte after an even-offset bit of a run (the run's own
/// odd-offset bits, and the byte after an odd run).
fn escaped_reference(a: &[bool]) -> Vec<bool> {
    let mut run = 0usize;
    a.iter()
        .map(|&x| {
            let escaped = run % 2 == 1;
            run = if x { run + 1 } else { 0 };
            escaped
        })
        .collect()
}

#[test]
fn carried_round_trip_and_refusals() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 3\n  block 0 %2 = toggle %0\n  block 0 %3 = escaped %0\n  block 0 output %1\n  block 0 output %2\n  block 0 output %3\n";
    let p = program(rows);
    let text = rir::disassemble(&p, None);
    for line in ["%1 = prev %0, 3", "%2 = toggle %0", "%3 = escaped %0"] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert_eq!(rir::assemble(&text).expect("reassembles").canonical_digest("-", b""), p.canonical_digest("-", b""));
    for (rows, why) in [
        ("  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 0\n  block 0 output %1\n", "distance 0"),
        ("  block 0 %0 = eq 0x5c\n  block 0 %1 = prev %0, 64\n  block 0 output %1\n", "distance 64"),
        ("  block 0 %0 = toggle %0\n  block 0 output %0\n", "toggle of itself"),
        ("  block 0 %0 = escaped %1\n  block 0 output %0\n", "a later operand"),
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
fn escaped_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = escaped %0\n  block 0 output %1\n";
    check_all(rows, |bytes| vec![words_of(&escaped_reference(&bits(bytes, |b| b == b'\\')))]);
}

// --- Primitive 13: fill (carried, two operands) ---------------------------------------------

/// `f[i] = s[i] | (f[i - 1] & !e[i])`: from each start up to, not including, the next end.
fn fill_reference(s: &[bool], e: &[bool]) -> Vec<bool> {
    let mut open = false;
    s.iter().zip(e).map(|(&s, &e)| { open = s || (open && !e); open }).collect()
}

#[test]
fn fill_round_trips_and_refusals() {
    let p = program("  block 0 %0 = eq 0x22
  block 0 %1 = eq 0x5c
  block 0 %2 = fill %0, %1
  block 0 output %2
");
    let text = rir::disassemble(&p, None);
    assert!(text.contains("%2 = fill %0, %1"), "{text}");
    assert_eq!(rir::assemble(&text).expect("reassembles").canonical_digest("-", b""), p.canonical_digest("-", b""));
    let rows = "  block 0 %0 = eq 0x22
  block 0 %1 = fill %0, %1
  block 0 output %1
";
    let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
    assert!(rir::assemble(&text).is_err(), "fill of itself: accepted");
}

#[test]
fn fill_equals_reference_on_every_executor() {
    // Starts at quotes, ends at backslashes; then starts and ends the same bytes (each start
    // ends the fill coming in and begins another); then ends that are never set.
    let rows = "  block 0 %0 = eq 0x22
  block 0 %1 = eq 0x5c
  block 0 %2 = fill %0, %1
  block 0 %3 = fill %0, %0
                \x20 block 0 %4 = const 0x0000000000000000
  block 0 %5 = fill %0, %4
                \x20 block 0 output %2
  block 0 output %3
  block 0 output %5
";
    check_all(rows, |bytes| {
        let (q, b) = (bits(bytes, |x| x == b'"'), bits(bytes, |x| x == b'\\'));
        let none = vec![false; bytes.len()];
        vec![words_of(&fill_reference(&q, &b)), words_of(&fill_reference(&q, &q)), words_of(&fill_reference(&q, &none))]
    });
}

/// `fill` in one round with `toggle` and `escaped` (three scanned carries, one table), and
/// a fill whose operands are carried values.
#[test]
fn fill_with_other_carries_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x5c
  block 0 %1 = eq 0x22
  block 0 %2 = eq 0x61
  block 0 %3 = escaped %0
                \x20 block 0 %4 = toggle %1
  block 0 %5 = fill %2, %0
  block 0 %6 = fill %4, %3
                \x20 block 0 output %5
  block 0 output %6
";
    check_all(rows, |bytes| {
        let (bs, q, a) = (bits(bytes, |x| x == b'\\'), bits(bytes, |x| x == b'"'), bits(bytes, |x| x == b'a'));
        let (escaped, inside) = (escaped_reference(&bs), toggle_reference(&q));
        vec![words_of(&fill_reference(&a, &bs)), words_of(&fill_reference(&inside, &escaped))]
    });
}

#[test]
fn fill_lowers_to_straight_bit_operations() {
    check_lowering("fill", &program("  block 0 %0 = eq 0x22
  block 0 %1 = eq 0x5c
  block 0 %2 = fill %0, %1
  block 0 output %2
"));
}

/// Carried operations in a chain, each round's carry depending on the one before: the escaped
/// quotes removed before the toggle, then a shift of the result.
#[test]
fn carried_chain_equals_reference_on_every_executor() {
    let rows = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = escaped %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 %5 = prev %4, 1\n  block 0 output %4\n  block 0 output %5\n";
    check_all(rows, |bytes| {
        let escaped = escaped_reference(&bits(bytes, |b| b == b'\\'));
        let quote: Vec<bool> = bytes.iter().zip(&escaped).map(|(&b, &e)| b == b'"' && !e).collect();
        let inside = toggle_reference(&quote);
        vec![words_of(&inside), words_of(&prev_reference(&inside, 1))]
    });
}

/// The look-back's fallback: polling once, a group whose predecessor has not published leaves
/// unfinished and the launch is repeated, and the outputs are the same. The second program has a
/// carried `prev` (a halo) between two scanned carries.
#[test]
#[cfg(target_os = "macos")]
fn lookback_fallback_equals_reference() {
    use metal_program_backend::block_program::{BatchFile, ClassStrategy, Executor, LaneWidth, run_batch};
    let one = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = escaped %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 output %4\n";
    let two = "  block 0 %0 = eq 0x5c\n  block 0 %1 = eq 0x22\n  block 0 %2 = escaped %0\n  block 0 %3 = andnot %1, %2\n  block 0 %4 = toggle %3\n  block 0 %5 = prev %4, 5\n  block 0 %6 = toggle %5\n  block 0 output %4\n  block 0 output %6\n";
    for (rows, halo) in [(one, false), (two, true)] {
        let p = program(rows);
        let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
        for bytes in runs() {
            let escaped = escaped_reference(&bits(&bytes, |b| b == b'\\'));
            let quote: Vec<bool> = bytes.iter().zip(&escaped).map(|(&b, &e)| b == b'"' && !e).collect();
            let inside = toggle_reference(&quote);
            let mut want = vec![words_of(&inside)];
            if halo {
                want.push(words_of(&toggle_reference(&prev_reference(&inside, 5))));
            }
            for width in LaneWidth::ALL {
                for spins in [0, 64] {
                    for executor in [Executor::Interpreter, Executor::Generated(ClassStrategy::Planes)] {
                        let run = run_batch(&[&native], &[BatchFile { bytes: &bytes, program: 0 }], width, executor, spins).expect("runs");
                        assert_eq!(run.files[0].outputs, want, "{} bytes, {}-byte lanes, {spins} polls, {executor:?}", bytes.len(), width.bytes());
                    }
                }
            }
        }
    }
}

/// Generated MSL is one kernel, each instruction once: a few lines per instruction.
#[test]
#[cfg(target_os = "macos")]
fn generated_msl_is_one_kernel() {
    use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_with};
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    for width in LaneWidth::ALL {
        for strategy in ClassStrategy::ALL {
            let msl = generate_with(&native, width, strategy);
            assert_eq!(msl.matches("kernel void ").count(), 1, "{msl}");
            let lines = msl.lines().filter(|l| l.starts_with("  const ") || l.starts_with("  if (!")).count();
            assert!(lines < 12 * native.insts.len(), "{lines} lines for {} instructions", native.insts.len());
        }
    }
}

#[test]
fn carried_lower_to_straight_bit_operations() {
    check_lowering(
        "carried",
        &program("  block 0 %0 = eq 0x5c\n  block 0 %1 = escaped %0\n  block 0 %2 = toggle %1\n  block 0 %3 = prev %2, 1\n  block 0 output %3\n"),
    );
}

// --- simdjson's stage 1 ----------------------------------------------------------------------

/// Stage 1, written by hand in `rir` (`fixtures/simdjson-stage1/stage1.rir`).
pub(super) const STAGE1: &str = include_str!("../../../../../fixtures/simdjson-stage1/stage1.rir");

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
        // Cranelift has no lowering of `extract` rows (docs/parser-generators/20-the-ir.md).
        ("cpu interpreter".to_owned(), block.run(bytes)),
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
        // One poll: a group whose predecessor has not published its count leaves unfinished,
        // and the launch is repeated.
        #[cfg(target_os = "macos")]
        {
            use metal_program_backend::block_program::{BatchFile, ClassStrategy, Executor, LaneWidth, run_batch};
            let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
            for width in LaneWidth::ALL {
                for executor in [Executor::Interpreter, Executor::Generated(ClassStrategy::Planes)] {
                    let run = run_batch(&[&native], &[BatchFile { bytes: &bytes, program: 0 }], width, executor, 0).expect("runs");
                    assert_eq!(run.files[0].positions, want, "no polls, {}-byte lanes, {executor:?}, {} bytes", width.bytes(), bytes.len());
                }
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
            if name == "cranelift" {
                continue; // no `extract` lowering on the CPU (docs/parser-generators/20-the-ir.md)
            }
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
        // The mask passes alone, and with the extract pass (in the same launch unless a
        // look-back runs out of polls; repeated launches are timed, and counted).
        let mut masks = native.clone();
        masks.extracts.clear();
        use metal_program_backend::block_program::REPEATED_LAUNCHES;
        let before = REPEATED_LAUNCHES.load(std::sync::atomic::Ordering::Relaxed);
        for (what, program) in [("masks", &masks), ("masks + extract", &native)] {
            for strategy in ClassStrategy::ALL {
                for width in LaneWidth::ALL {
                    let s = (0..5)
                        .map(|_| run_generated_with(program, &bytes, width, strategy).expect("runs").1)
                        .fold(f64::MAX, f64::min);
                    println!(
                        "generated msl, {what}, {strategy:?}, {:2}-byte lanes: {:.3} ns per block (GPU time, best of 5)",
                        width.bytes(),
                        s / blocks * 1e9
                    );
                }
            }
        }
        let repeats = REPEATED_LAUNCHES.load(std::sync::atomic::Ordering::Relaxed) - before;
        println!("repeated launches: {repeats} over 60 runs");
    }
}

// --- Primitive 14: within, and the program's delay ------------------------------------------

/// `next(a, k)`: each byte's bit from `k` bytes after, zero past the end.
fn next_reference(a: &[bool], k: usize) -> Vec<bool> {
    (0..a.len()).map(|j| j + k < a.len() && a[j + k]).collect()
}

#[test]
fn within_and_delay_round_trip_and_refusals() {
    let p = program("  block 0 %0 = within\n  block 0 %1 = eq 0x22\n  block 0 output %1\n  block 0 delay 5\n");
    let text = rir::disassemble(&p, None);
    assert!(text.contains("%0 = within") && text.contains("block 0 delay 5"), "{text}");
    assert_eq!(rir::assemble(&text).expect("reassembles").canonical_digest("-", b""), p.canonical_digest("-", b""));
    let undelayed = program("  block 0 %0 = within\n  block 0 %1 = eq 0x22\n  block 0 output %1\n");
    assert_ne!(undelayed.canonical_digest("-", b""), p.canonical_digest("-", b""), "the delay is in the digest");
    // `within` at a delay: its own text and digest byte, `within` alone unchanged.
    let at = program("  block 0 %0 = within 5\n  block 0 output %0\n");
    let text = rir::disassemble(&at, None);
    assert!(text.contains("%0 = within 5"), "{text}");
    assert_eq!(rir::assemble(&text).expect("reassembles").canonical_digest("-", b""), at.canonical_digest("-", b""));
    assert_ne!(at.canonical_digest("-", b""), program("  block 0 %0 = within\n  block 0 output %0\n").canonical_digest("-", b""));
    assert_eq!(block::within_at(0, 3, 5), 0b1110_0000);
    assert_eq!(block::within_at(1, 70, 5), (1 << 11) - 1);
    for (rows, why) in [
        ("  block 0 %0 = within 64\n  block 0 output %0\n", "a within delay of 64"),
        ("  block 0 %0 = eq 0x22\n  block 0 output %0\n  block 0 delay 64\n", "a delay of 64"),
    ] {
        let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
        assert!(rir::assemble(&text).is_err(), "{why}: accepted\n{text}");
    }
}

/// `within` is the stream's bytes; with a delay of `d`, an output is the value `d` bytes after
/// (`next`), so a value that reads past the end must be cut by `within` first: `eq 0x00` sees
/// the padding's zero bytes, `within & eq 0x00` does not.
#[test]
fn within_and_delay_equal_reference_on_every_executor() {
    check_all("  block 0 %0 = within\n  block 0 %1 = eq 0x00\n  block 0 %2 = and %0, %1\n  block 0 output %0\n  block 0 output %2\n", |bytes| {
        vec![words_of(&vec![true; bytes.len()]), words_of(&bits(bytes, |b| b == 0))]
    });
    for d in [1usize, 2, 3, 15, 16, 17, 33, 63] {
        let rows = format!(
            "  block 0 %0 = within\n  block 0 %1 = eq 0x00\n  block 0 %2 = and %0, %1\n  block 0 %3 = eq 0x22\n  block 0 %4 = toggle %3\n\
             \x20 block 0 %5 = and %4, %0\n  block 0 output %2\n  block 0 output %3\n  block 0 output %5\n  block 0 delay {d}\n"
        );
        check_all(&rows, |bytes| {
            let nul = bits(bytes, |b| b == 0);
            let q = bits(bytes, |b| b == b'"');
            vec![words_of(&next_reference(&nul, d)), words_of(&next_reference(&q, d)), words_of(&next_reference(&toggle_reference(&q), d))]
        });
    }
}

#[test]
fn within_lowers_to_straight_bit_operations() {
    check_lowering("within", &program("  block 0 %0 = within\n  block 0 %1 = eq 0x00\n  block 0 %2 = and %0, %1\n  block 0 output %2\n  block 0 delay 3\n"));
}

// --- Cranelift's loop keeps simdjson's shape (decision 0047) ------------------------------------

/// The loop of a compiled block function's machine code: its largest block (the body over one
/// 64-byte block) and the blocks it branches to that jump back to it (the back edge). The
/// edge into the loop from a file's row is not counted: it runs once per file.
pub(super) fn machine_loop(disassembly: &str) -> Vec<String> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for line in disassembly.lines() {
        if let Some(label) = line.strip_suffix(':').filter(|l| l.starts_with("block")) {
            blocks.push((label.trim_start_matches("block").to_owned(), Vec::new()));
        } else if let Some((_, body)) = blocks.last_mut() {
            body.push(line.trim().to_owned());
        }
    }
    // A block and the edge blocks its last branch takes back to it.
    let targets = |exits: &str, label: &str| exits.split(|c: char| !c.is_alphanumeric()).any(|w| w == format!("label{label}"));
    let looped = |body: &str, lines: &[String]| -> Option<Vec<String>> {
        let exits = lines.last().cloned().unwrap_or_default();
        let back = format!("b label{body}");
        let mut out = lines.to_vec();
        let mut is_loop = targets(&exits, body);
        for (label, l) in &blocks {
            if targets(&exits, label) && l.last().is_some_and(|t| *t == back) {
                out.extend(l.iter().cloned());
                is_loop = true;
            }
        }
        is_loop.then_some(out)
    };
    // The hot loop: the largest block that loops to itself (the full blocks' body).
    blocks.iter().filter_map(|(label, l)| looped(label, l)).max_by_key(Vec::len).expect("a loop")
}

/// A loop's stack loads and stores (spills and reloads), and its vector `mvn`s.
pub(super) fn loop_costs(lines: &[String]) -> (usize, usize) {
    let stack = lines.iter().filter(|l| (l.starts_with("ldr") || l.starts_with("str") || l.starts_with("ldp") || l.starts_with("stp")) && l.contains("[sp")).count();
    let mvn = lines.iter().filter(|l| l.starts_with("mvn v")).count();
    (stack, mvn)
}

/// The function's loop-carried values, read from the Cranelift IR: the hot body's parameters
/// after its block index and its byte and output pointers.
pub(super) fn carried_params(clif: &str) -> usize {
    let line = clif.lines().find(|l| l.trim_start().starts_with("block2(")).expect("the body block");
    line.matches(": i64").count() - 3
}

/// Stage 1 compiles to a loop with no stack traffic and no vector `mvn`: every value and
/// constant in a register, its carried state packed into words.
#[test]
#[cfg(target_arch = "aarch64")]
fn stage1_loop_keeps_values_in_registers() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = machine_loop(&compiled.disassembly);
    let (stack, mvn) = loop_costs(&lines);
    eprintln!("stage 1 loop: {} instructions, {stack} stack loads or stores, {mvn} vector mvn", lines.len());
    assert_eq!((stack, mvn), (0, 0), "stage 1's loop spills or inverts vectors:\n{}", lines.join("\n"));
    let block = &p.block_programs()[0];
    // The state is packed: one carried word per 64 state bits (decision 0048).
    let words = state_bits(block).div_ceil(64);
    assert_eq!(carried_params(&compiled.clif) - sink_params(&native), words, "stage 1's carried state is not packed");
}

/// Stage 1's loop is simdjson's size (decision 0049): one reduction per 64-bit mask and one
/// lookup per table for every class, at most 150 instructions per 64 bytes.
#[test]
#[cfg(target_arch = "aarch64")]
#[ignore = "red guard (decision 0049): on at step 2, stage 1's codegen"]
fn stage1_loop_is_simdjsons_size() {
    let p = rir::assemble(STAGE1).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = machine_loop(&compiled.disassembly);
    eprintln!("stage 1 loop: {} instructions (at most 150)", lines.len());
    assert!(lines.len() <= 150, "stage 1's loop: {} instructions (at most 150):\n{}", lines.len(), lines.join("\n"));
}

/// A lab: the Cranelift loop of the program in the `.rir` file `RERAIL_RIR`, its size and its
/// opcodes, for measuring variants of a program.
#[test]
#[cfg(target_arch = "aarch64")]
#[ignore = "a lab; run by name with RERAIL_RIR"]
fn rir_loop_size() {
    let text = std::fs::read_to_string(std::env::var("RERAIL_RIR").expect("RERAIL_RIR")).expect("reads");
    let p = rir::assemble(&text).unwrap_or_else(|e| panic!("{e}"));
    let native = native_ir::lower(&p).expect("lowers").block_programs.remove(0);
    let compiled = native_cpu_block::compile(&native).expect("compiles");
    let lines = machine_loop(&compiled.disassembly);
    let mut ops: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in &lines {
        *ops.entry(l.split_whitespace().next().unwrap_or("")).or_default() += 1;
    }
    let mut ops: Vec<_> = ops.into_iter().collect();
    ops.sort_by_key(|o| std::cmp::Reverse(o.1));
    eprintln!("loop: {} instructions, {} stack; {ops:?}", lines.len(), loop_costs(&lines).0);
    if std::env::var("RERAIL_ASM").is_ok() {
        eprintln!("{}", lines.join("\n"));
    }
}

/// A program's carried state in bits (decision 0048): one bit per `toggle`, `escaped` and
/// `fill`, and for each value its `prev`s read, the top `k` bits of its previous word (`k` the
/// farthest look-back). A `toggle`'s and a `fill`'s carry is its top bit, so a look-back on one
/// costs `k - 1` more. Every executor carries these bits and no more: Cranelift packs them into
/// words, never a word per bit.
pub(super) fn state_bits(p: &block::BlockProgram) -> usize {
    use super::block::BlockOp::*;
    let mut back: std::collections::BTreeMap<u16, u8> = std::collections::BTreeMap::new();
    for op in &p.ops {
        if let Prev(a, k) = *op {
            let m = back.entry(a).or_insert(0);
            *m = (*m).max(k);
        }
    }
    let ops = p.ops.iter().filter(|op| matches!(op, Toggle(_) | Escaped(_) | Fill(..))).count();
    let looks: usize = back
        .iter()
        .map(|(&a, &k)| match p.ops[usize::from(a)] {
            Toggle(_) | Fill(..) => usize::from(k) - 1,
            _ => usize::from(k),
        })
        .sum();
    ops + looks
}

/// The values a program's `prev`s read, one carried word each, and its other carried operations.
pub(super) fn carried_words(p: &block::BlockProgram) -> usize {
    use super::block::BlockOp::*;
    let bases: std::collections::BTreeSet<u16> = p.ops.iter().filter_map(|op| if let Prev(a, _) = *op { Some(a) } else { None }).collect();
    bases.len() + p.ops.iter().filter(|op| matches!(op, Toggle(_) | Escaped(_) | Fill(..))).count()
}
