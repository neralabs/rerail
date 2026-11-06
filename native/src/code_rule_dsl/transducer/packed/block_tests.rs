//! Block programs, built up one primitive at a time (`docs/parser-generators/12-what-is-wanted.md`).
//! Each primitive is a tiny transducer program written in `rir`, checked on every executor (the
//! CPU interpreter, Cranelift, the Metal interpreter, generated MSL) against a byte-by-byte
//! reference that exists only here, and checked for what it lowers to: straight bit operations.

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
fn executors(p: &Program, bytes: &[u8]) -> Vec<(&'static str, Vec<Vec<u64>>)> {
    let block = &p.block_programs()[0];
    let native = native_ir::lower(p).expect("lowers").block_programs.remove(0);
    let mut out = vec![
        ("cpu interpreter", block.interpret(bytes)),
        ("cranelift", native_cpu_block::compile(&native).expect("compiles").run(bytes)),
    ];
    #[cfg(target_os = "macos")]
    {
        out.push(("metal interpreter", metal_program_backend::block_program::interpret(block, bytes).expect("runs")));
        out.push(("generated msl", metal_program_backend::block_program::run_generated(&native, bytes).expect("runs")));
    }
    out
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

/// The opcodes a block body may contain: loads and stores of the block, bit operations, the
/// block's address arithmetic, and the one branch back to the loop's head.
const BODY_OPCODES: &[&str] = &[
    "load", "store", "icmp", "icmp_imm", "vhigh_bits", "uextend", "ishl_imm", "bor", "band", "bxor",
    "bnot", "band_not", "iadd", "iadd_imm", "iconst", "brif",
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

    let msl = metal_program_backend_generate(&native);
    println!("== {name}: generated MSL ==\n{msl}");
    let kernel = &msl[msl.find("kernel void").expect("a kernel")..];
    for word in ["for (", "while", "switch", "goto", "?"] {
        assert!(!kernel.contains(word), "{name}: generated MSL has `{word}`");
    }
    // The one `if` is the lane-0 store of each SIMD group's ballot.
    assert_eq!(kernel.matches("if (").count(), 1, "{name}: one store guard");
    assert!(kernel.contains("if (lane == 0u)"));
}

#[cfg(target_os = "macos")]
fn metal_program_backend_generate(native: &native_ir::BlockNative) -> String {
    metal_program_backend::block_program::generate(native)
}

#[cfg(not(target_os = "macos"))]
fn metal_program_backend_generate(_native: &native_ir::BlockNative) -> String {
    "kernel void tempo_block() { if (lane == 0u) {} }".into()
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
}
