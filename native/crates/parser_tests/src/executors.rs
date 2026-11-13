//! Every executor of a block program, for checks that a program runs alike on all of them: the
//! CPU interpreter, Cranelift and, on a Mac, Metal's interpreter and generated MSL at every lane
//! width and the forward carry. And the inputs those checks run over: every byte value at every
//! position of a block, and runs long enough to carry across lanes, SIMD groups and threadgroups.
//!
//! Public so the engine's tests of its own block programs (`packed::regex_block`) use them too.

use rerail_compiler::program::Words as Program;
use rerail_compiler::{Error, block, block_native, native_cpu_block, native_ir};
#[cfg(target_os = "macos")]
use rerail_metal as metal_program_backend;

/// Inputs that put every byte value at every position of a block, and lengths around every
/// block edge.
pub fn inputs() -> Vec<Vec<u8>> {
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
pub fn runs() -> Vec<Vec<u8>> {
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

/// Every executor's outputs for the program's first block program, with the executor's name.
pub fn executors(p: &Program, bytes: &[u8]) -> Vec<(String, Vec<Vec<u64>>)> {
    let (out, skipped) = Executors::new(p).run(bytes, true);
    assert!(skipped.is_empty(), "over the launch budget: {skipped:?}");
    out
}

/// A program's first block program, lowered and compiled once, for runs over many inputs.
pub struct Executors<'a> {
    block: &'a block::BlockProgram,
    native: block_native::BlockNative,
    cranelift: native_cpu_block::CompiledBlock,
}

impl<'a> Executors<'a> {
    pub fn new(p: &'a Program) -> Self {
        let native = native_ir::lower(p, 0, 0).expect("lowers").block_programs.remove(0);
        let cranelift = native_cpu_block::compile(&native).expect("compiles");
        Executors { block: &p.block_programs[0], native, cranelift }
    }

    /// Every executor's outputs over `bytes` (the Metal ones when `metal`), without a Metal
    /// executor whose run would take more launches than the budget (`metal_launch_budget`):
    /// those are named in the second list.
    pub fn run(&self, bytes: &[u8], metal: bool) -> (Vec<(String, Vec<Vec<u64>>)>, Vec<String>) {
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
            // The forward carry (`metal_block_serial`): segments of three blocks, so that a file of
            // a few blocks has segments that speculate their start and run again.
            use metal_program_backend::block_program::serial::{Segments, run_serial_split};
            let file = [block::BatchFile { bytes, program: 0 }];
            keep("metal forward carry, segments of 3".to_owned(), run_serial_split(&[&self.native], &file, Segments { blocks: 3, warm: 1 }).map(|mut r| r.run.files.remove(0).outputs));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = metal;
        (out, skipped)
    }
}

