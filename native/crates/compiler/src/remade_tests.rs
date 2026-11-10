//! Values made from the position alone (`phase`, `position` planes and a `prev` of one) after a
//! backward pass: stored by the first pass and loaded, and equal to the interpreter.
//!
//! Remaking them in each pass instead (doc 26 §1.10, transformation 2) was measured and not
//! taken: it saves the slots' stores and loads but adds more instructions than it saves (a
//! phase is two remainders, a `prev` of a plane its carried bit), and was no faster on the CPU
//! (`remade_positions_payoff`). Constants and `within` are remade (`native_cpu_lanes::remade`).

use crate::block_native::BlockInst;
use crate::gating_tests::small;
use crate::native_cpu_block;

/// `aligned(_, 3)` makes `phase 3 r` (3 does not divide 64): read in the first pass by
/// `aligned(d, 3)` and after the backward pass by `aligned(s, 3)`.
const ALIGNED_LATE: &str = "lexer t
class d  = '0'..'9'
class q  = '\"'
class sp = ' '
mask s  = back_fill(q, sp)
mask a  = aligned(d, 3)
mask b  = aligned(s, 3)
escape \"x\" when a
escape \"y\" when b
";

/// `position`'s planes from 6 on are `position j`: read before the backward pass (into
/// `back_fill`'s value) and after it (`end - position`), a run of backticks' length to its end.
const POSITION_LATE: &str = "lexer t
class tick = '`'
mask last = tick & ~next(tick)
number end = back_fill(last, ~tick, position)
number rem = end - position
mask long = tick & at_least(rem, 3)
escape \"long\" when long
";

fn from_position(inst: &BlockInst) -> bool {
    matches!(inst, BlockInst::Phase(..) | BlockInst::Position(_))
}





/// Files of digits, quotes and spaces at every residue mod 3, over several blocks.
fn files() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for len in [1usize, 63, 64, 65, 129, 200, 700, 3000] {
        for seed in 0..4u64 {
            let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let f: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    [b'1', b'`', b'"', b' ', b'`', b'7'][(x % 6) as usize]
                })
                .collect();
            out.push(f);
        }
    }
    out
}

#[test]
fn positions_after_a_backward_pass_equal_the_interpreter() {
    for text in [ALIGNED_LATE, POSITION_LATE] {
        let g = small(text);
        assert!(g.native.insts.iter().any(from_position), "the grammar makes no `phase` or `position`:\n{text}");
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        for f in &files() {
            assert_eq!(compiled.run_files(&[f]), g.block.run_files(&[f]), "{:?}", String::from_utf8_lossy(f));
        }
    }
}


/// The payoff, run by hand: the row's slots, the passes' machine code and their CPU time over
/// 8 MB. Measured (2026-11-10, load 42 to 73 on 18 cores), stored against remade:
/// `aligned` 7 slots, 564 instructions, 40 memory, 0.32 ms/MB against 4, 599, 34, 0.34 to 0.37;
/// `position` 100 slots, 1637, 261, 1.16 to 1.23 against 66, 1745, 193, 1.20 to 1.59.
#[test]
#[ignore = "a measurement: run by hand"]
fn remade_positions_payoff() {
    let file: Vec<u8> = files().into_iter().flatten().cycle().take(8 << 20).collect();
    for (name, text) in [("aligned", ALIGNED_LATE), ("position", POSITION_LATE)] {
        let g = small(text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        let seconds = compiled.cpu_seconds(&file, 7);
        // The passes' own machine code: instructions, memory loads and stores, stack ones.
        let code = compiled.loops_disassembly();
        let lines: Vec<&str> = code.lines().map(str::trim).filter(|l| !l.is_empty() && !l.ends_with(':')).collect();
        let memory = |l: &&str| ["ldr", "str", "ldp", "stp", "ldur", "stur"].iter().any(|m| l.starts_with(m));
        let stack = lines.iter().filter(|l| memory(l) && l.contains("[sp")).count();
        let mem = lines.iter().filter(|l| memory(l)).count() - stack;
        eprintln!("{name}: {} slots, {} instructions, {mem} memory, {stack} stack, {:.2} ms CPU per MB", g.native.passes().slots, lines.len(), seconds * 1e3 / 8.0);
    }
}
