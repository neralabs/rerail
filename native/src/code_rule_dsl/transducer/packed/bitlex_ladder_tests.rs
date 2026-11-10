//! The Bitlex ladder: thirteen languages, from one keyword to a data language with nested
//! records, lists, texts, escapes, numbers and two kinds of comment, a step at a time. Each is
//! parsed by its Bitlex file alone (`bitlex_ladder/`), with no Rust between the bytes and the
//! nodes.
//!
//! The languages are made up, so that no step is a known format: records are `< >`, lists
//! `( )`, items are separated by `;`, an entry is `#key# = value`, texts are between `#`, a caret
//! escapes, the booleans are `vrai` and `fausse`, nothing is `nul`, a number has a tilde for minus
//! and a decimal comma (`~3,14e~2`), and comments are `-- …` and `{- … -}`.
//!
//! Each step adds one construct to the one before. No step's test code knows its language: the
//! documents are drawn from the step's own tree grammar, its tokens' keywords and checks, and
//! then mutated a byte at a time.
//!
//! For each step and document, the same nodes (or the same refusal, where the file goes to
//! tree-sitter) from:
//!
//! 1. the reference: the evaluator and tree parser of `tools/bitlex`, which run Bitlex without
//!    the compiler;
//! 2. the interpreters: tier 1's block program, then tier 2's words;
//! 3. the compiled path, `native_cpu_stream::Parser`: one Cranelift function per tier, tier 2
//!    reading tier 1's buffers as written.
//!
//! Tier 1 on Cranelift and on Metal (generated MSL) equals its interpreter bit for bit. And each
//! step has the shape the compiler promises: tier 1's body has no branch but the loop's and the
//! extracts' flattening (it is branchless per block), and tier 2 has one loop, over the tokens
//! (and one over 64-byte blocks inside each `check`); neither calls anything.

//!
//! The checks are the compiler crate's (`rerail_compiler::ladder`), and the ladder's test
//! there (`ladder_tests`); here are the Metal backend's.

pub(super) use rerail_compiler::ladder::*;
#[allow(unused_imports)]
use super::*;

/// Every step's tier 1 on Metal, generated MSL (one lane width, the class planes), equals the
/// CPU interpreter on every document: outputs and extracted positions.
#[test]
#[cfg(target_os = "macos")]
fn every_step_on_metal_equals_the_interpreter() {
    use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, run_split};
    let steps = steps_in(DIR);
    let compiled: Vec<Compiled> = steps.iter().map(|(n, s)| compile_step(n, s)).collect();
    let docs: Vec<Vec<Vec<u8>>> = compiled.iter().zip(&steps).map(|(c, (n, _))| documents(&Lang::of(&c.stmts), seed(n), &|b| reference(c, b).is_some()).0).collect();
    let forms: Vec<&native_ir::BlockNative> = compiled.iter().map(|c| &c.native.block_programs[0]).collect();
    let files: Vec<block::BatchFile<'_>> = docs.iter().enumerate().flat_map(|(p, ds)| ds.iter().map(move |b| block::BatchFile { bytes: b, program: p })).collect();
    let run = run_split(&forms, &files, LaneWidth::W32, Executor::Generated(ClassStrategy::Planes), 64).unwrap_or_else(|e| panic!("metal: {e:?}"));
    let mut differ = Vec::new();
    for (f, got) in files.iter().zip(&run.files) {
        let want = compiled[f.program].program.block_programs[0].run_files(&[f.bytes]).remove(0);
        if got.outputs != want.outputs || got.positions != want.positions {
            differ.push(format!("{}: {:?}", steps[f.program].0, String::from_utf8_lossy(f.bytes)));
        }
    }
    eprintln!("{} files over {} steps on Metal", files.len(), steps.len());
    assert!(differ.is_empty(), "{} files differ:\n{}", differ.len(), differ.join("\n"));
}
