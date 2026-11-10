//! Only the hot loop is inlined: the code a pass runs rarely is a function of its own, made
//! from the same program, that the pass calls. Too many instructions in one function crowd the
//! instruction cache, and code skipped still sits between the loop's.
//!
//! The general body (the first block of a file and the last ones, where `within` is not all
//! ones) runs a block or two per file: the pass calls it, with the loop's parameters through
//! a stack slot.

use super::block_tests::{hot_loop, opcode};
use super::gating_tests::{ESCAPES, LATE, many, small};
use super::native_cpu_block;

/// The instructions of a function's IR, as text lines.
fn instructions(clif: &str) -> Vec<&str> {
    clif.lines().map(str::trim).filter(|t| !t.is_empty() && !t.ends_with(':') && !t.starts_with("function") && !t.starts_with('}') && !t.starts_with("sig") && !t.starts_with("fn") && !t.starts_with("ss") && !t.starts_with("const")).collect()
}

#[test]
fn the_general_body_is_a_call_not_a_copy() {
    for (name, text) in [("escapes", ESCAPES.to_owned()), ("late", LATE.to_owned()), ("many", many())] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        for (k, pass) in compiled.passes.iter().enumerate().filter(|(_, p)| !p.back) {
            assert_eq!(pass.outlined.len(), 1, "{name} pass {k}: the general body is not outlined");
            let calls = instructions(&pass.clif).iter().filter(|l| opcode(l) == "call").count();
            assert_eq!(calls, 1, "{name} pass {k}: one call, the general body's");
            let general = &pass.outlined[0].clif;
            assert!(instructions(general).iter().all(|l| opcode(l) != "call"), "{name} pass {k}: the general body calls nothing");
            // Outside the hot loop the pass holds only the loops' scaffolding: fewer
            // instructions than the body it calls.
            let hot: usize = hot_loop(&pass.clif).iter().map(|(_, lines)| lines.len()).sum();
            let rest = instructions(&pass.clif).len() - hot;
            assert!(rest < instructions(general).len(), "{name} pass {k}: {rest} instructions outside the hot loop, the general body {}", instructions(general).len());
        }
    }
}

#[test]
fn the_called_general_body_equals_the_interpreter() {
    // Lengths at each block's edge: the first block, the last, a file of one block, none.
    let unit = b"ab \\u12z4 \\u00e9 zz \\\\u \\uzz12 z";
    for text in [ESCAPES.to_owned(), LATE.to_owned(), many()] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        for len in [0usize, 1, 5, 63, 64, 65, 127, 128, 129, 200, 300] {
            let file: Vec<u8> = unit.iter().copied().cycle().take(len).collect();
            assert_eq!(compiled.run_files(&[&file]), g.block.run_files(&[&file]), "length {len}");
        }
        let files: Vec<Vec<u8>> = [3usize, 70, 130].iter().map(|&n| unit.iter().copied().cycle().skip(n % 7).take(n).collect()).collect();
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert_eq!(compiled.run_files(&refs), g.block.run_files(&refs), "a batch");
    }
}
