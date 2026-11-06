//! Block programs, built up one primitive at a time (`docs/parser-generators/12-what-is-wanted.md`).
//! Each primitive is a tiny transducer program written in `rir`, checked on every executor (the
//! CPU interpreter, Cranelift, and the Metal interpreter and generated MSL at every lane width)
//! against a byte-by-byte reference that exists only here, and checked for what it lowers to:
//! straight bit operations.

use super::*;

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
        use metal_program_backend::block_program::{LaneWidth, interpret, run_generated};
        for width in LaneWidth::ALL {
            let w = width.bytes();
            out.push((format!("metal interpreter, {w}-byte lanes"), interpret(block, bytes, width).expect("runs")));
            out.push((format!("generated msl, {w}-byte lanes"), run_generated(&native, bytes, width).expect("runs")));
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
        let msl = metal_program_backend_generate(&native, width);
        println!("== {name}: generated MSL, {width}-byte lanes ==\n{msl}");
        let kernel = &msl[msl.find("kernel void").expect("a kernel")..];
        for word in ["for (", "while", "switch", "goto", "?", "if (", "simd_"] {
            assert!(!kernel.contains(word), "{name}: generated MSL has `{word}`");
        }
    }
}

#[cfg(target_os = "macos")]
fn metal_program_backend_generate(native: &native_ir::BlockNative, width: usize) -> String {
    use metal_program_backend::block_program::{LaneWidth, generate};
    let width = LaneWidth::ALL.into_iter().find(|w| w.bytes() == width).expect("a width");
    generate(native, width)
}

#[cfg(not(target_os = "macos"))]
fn metal_program_backend_generate(_native: &native_ir::BlockNative, _width: usize) -> String {
    "kernel void tempo_block() {}".into()
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
    let metal = include_str!("metal_block_program.metal");
    assert_eq!(metal.matches("for (").count(), 1, "the Metal interpreter's one loop is over operations");
    assert!(metal.contains("for (uint at = 0u; at < p.ops; at++)"));
    for word in ["while", "if (", "simd_ballot"] {
        assert!(!metal.contains(word), "the Metal interpreter has `{word}`");
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
