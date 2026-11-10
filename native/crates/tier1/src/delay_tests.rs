//! A delayed output (doc 26 §1.4): Bitlex's `next(x, k)` delays the program by `k` bytes, so an
//! output at delay `d` decides bit `p` at bit `p + d`. Its stream word `i - 1` takes bits from
//! two of its words, `w[i-1]` and `w[i]`. The word is never held across the loop for that: each
//! block stores its own as it is, and the next block joins the two in memory, where it stored
//! it an iteration before (`native_cpu_block::delayed_store`).

use crate::gating_tests::{ESCAPES, small};
use crate::native_cpu_block::{self, State};
use crate::block_native::Pass;

/// Outputs at delays 1, 3 and 7, together.
const DELAYS: &str = "lexer t
class a = 'a'
class b = 'b'
mask m1 = a & next(b)
mask m3 = a & next(b, 3)
mask m7 = a & next(a, 7) & next(b, 2)
escape \"x1\" when m1
escape \"x3\" when m3
escape \"x7\" when m7
";

/// The parameters of a function's block `block`, in its Cranelift IR.
fn params(clif: &str, block: &str) -> usize {
    let line = clif.lines().map(str::trim).find(|l| l.starts_with(&format!("{block}(")) || *l == format!("{block}:")).unwrap_or_else(|| panic!("no {block}"));
    line.matches(": i64").count()
}

#[test]
fn no_delayed_output_is_held_across_the_loop() {
    for text in [DELAYS, ESCAPES] {
        let g = small(text);
        let passes = g.native.passes();
        let Some(Pass::Forward { body, .. }) = passes.passes.last().cloned() else { panic!("a forward pass last") };
        assert!(body.output_delays.iter().any(|&d| d > 0), "no delayed output");
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        // The hot loop's head (block2) takes the block, the byte and output pointers, the
        // state, the counts, the row of slots: no held word.
        let want = 3 + State::of(&body).words + native_cpu_block::counted(&body).len() + usize::from(passes.slots > 0);
        assert_eq!(params(&compiled.clif, "block2"), want, "a word held across the loop");
    }
}

#[test]
fn delayed_outputs_equal_the_interpreter() {
    let unit = b"ab aab abbb aaaaaaab ba aabab b";
    for text in [DELAYS, ESCAPES] {
        let g = small(text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        // Each block's edge: a word joined from two blocks, the first block's, the last's.
        for skip in [0usize, 5] {
            for len in [0usize, 1, 7, 62, 63, 64, 65, 66, 71, 127, 128, 129, 191, 192, 200, 513] {
                let file: Vec<u8> = unit.iter().copied().cycle().skip(skip).take(len).collect();
                assert_eq!(compiled.run_files(&[&file]), g.block.run_files(&[&file]), "skip {skip} length {len}");
            }
        }
        // A batch: no file's words reach into the next's.
        let files: Vec<Vec<u8>> = [65usize, 0, 1, 128, 70].iter().map(|&n| unit.iter().copied().cycle().skip(n % 5).take(n).collect()).collect();
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        assert_eq!(compiled.run_files(&refs), g.block.run_files(&refs), "a batch");
    }
}
