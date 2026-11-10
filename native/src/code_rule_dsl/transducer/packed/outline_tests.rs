//! Only the hot loop is inlined: the code a pass runs rarely is a function of its own, made
//! from the same program, that the pass calls. Too many instructions in one function crowd the
//! instruction cache, and code skipped still sits between the loop's.
//!
//! The general body (the first block of a file and the last ones, where `within` is not all
//! ones) runs a block or two per file: the pass calls it, with the loop's parameters through
//! a stack slot. A gated region (`native_cpu_lanes::Region`) runs only where its gate has a
//! bit: its branch calls it, with what it reads and gives through a stack slot.

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
            assert!(!pass.outlined.is_empty(), "{name} pass {k}: the general body is not outlined");
            let calls = instructions(&pass.clif).iter().filter(|l| opcode(l) == "call").count();
            // The general body's call, and each region's, once in each body that has it.
            let bodies = 1 + usize::from(pass.ascii_branch);
            assert_eq!(calls, 1 + bodies * (pass.outlined.len() - 1), "{name} pass {k}: a call each");
            let general = &pass.outlined[0].clif;
            for o in &pass.outlined {
                assert!(instructions(&o.clif).iter().all(|l| opcode(l) != "call"), "{name} pass {k}: an outlined function calls nothing");
            }
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

#[test]
fn a_gated_region_is_a_call() {
    for (name, text) in [("escapes", ESCAPES.to_owned()), ("late", LATE.to_owned()), ("many", many())] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        let gated: Vec<_> = compiled.passes.iter().filter(|p| p.gate_branches > 0).collect();
        assert!(!gated.is_empty(), "{name}: no region");
        for pass in gated {
            // Each region the body takes is a function of its own; its values are not in the
            // hot loop.
            assert!(pass.outlined.len() > 1, "{name}: the region is inline");
            let hot: Vec<String> = hot_loop(&pass.clif).into_iter().flat_map(|(_, l)| l).collect();
            assert!(hot.iter().any(|l| opcode(l) == "call"), "{name}: the hot loop does not call its region");
            for o in &pass.outlined[1..] {
                let lines = instructions(&o.clif);
                assert!(lines.iter().any(|l| l.contains("load.i8x16") || l.contains("load.i64")), "{name}: a region's function reads nothing");
            }
        }
    }
}

#[test]
fn a_called_region_equals_the_interpreter() {
    // A `\u` with and without its hex, in blocks of their own and across a block's edge.
    let unit = b"x \\u12ab z \\uzz \\\\u ab \\u0";
    for text in [ESCAPES.to_owned(), LATE.to_owned(), many()] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        for skip in [0usize, 3, 11] {
            for len in [64usize, 130, 260, 600] {
                let mut file: Vec<u8> = vec![b' '; 70];
                file.extend(unit.iter().copied().cycle().skip(skip).take(len));
                file.extend(vec![b'z'; 70]);
                assert_eq!(compiled.run_files(&[&file]), g.block.run_files(&[&file]), "skip {skip} length {len}");
            }
        }
    }
}
