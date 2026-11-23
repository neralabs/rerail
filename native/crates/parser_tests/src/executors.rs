//! Every executor of a block program, for checks that a program runs alike on all of them: the
//! CPU interpreter, Cranelift and, on a Mac, Metal's forward carry in segments of 3 and of 29
//! blocks. And the inputs those checks run over: every byte value at every
//! position of a block, and runs long enough to carry across lanes, SIMD groups and threadgroups.
//!
//! Public so the engine's tests of its own block programs (`packed::regex_block`) use them too.

use rerail_compiler::program::Words as Program;
use rerail_compiler::{block, native_ir};
use rerail_cpu_block::native_cpu_block;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use rerail_metal_programs as metal_program_backend;

/// No device here that runs a block program: none at all (a Linux host without a GPU), or one whose SIMD groups are
/// not 32 lanes (`block_program::ready`). A test's device runs are left out there; its CPU's are still held.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn no_device() -> bool {
    metal_program_backend::block_program::ready().is_err()
}

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
            let byte = b"\\\"a ,\\"[next(6) as usize];
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

/// Each executor's outputs on a file, by its name.
pub type Runs = Vec<(String, Vec<Vec<u64>>)>;

/// A program's first block program, lowered and compiled once, for runs over many inputs.
pub struct Executors<'a> {
    block: &'a block::BlockProgram,
    /// The program lowered, which the device's executors run.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    native: rerail_compiler::block_native::BlockNative,
    cranelift: native_cpu_block::CompiledBlock,
}

impl<'a> Executors<'a> {
    pub fn new(p: &'a Program) -> Self {
        let native = native_ir::lower(p, 0, 0).expect("lowers").block_programs.remove(0);
        let cranelift = native_cpu_block::compile(&native).expect("compiles");
        Executors {
            block: &p.block_programs[0],
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            native,
            cranelift,
        }
    }

    /// Every executor's outputs over `bytes`, with the executor's name.
    pub fn every(&self, bytes: &[u8]) -> Runs {
        self.run(bytes, true)
    }

    /// Every executor's outputs over `bytes` (the device's when `metal`). A device run past the
    /// launch budget is the hard failure (decision 0601), never skipped.
    pub fn run(&self, bytes: &[u8], metal: bool) -> Runs {
        if metal {
            return self.every_of(&[bytes.to_vec()]).remove(0);
        }
        vec![("cpu interpreter".to_owned(), self.block.interpret(bytes)), ("cranelift".to_owned(), self.cranelift.run(bytes))]
    }

    /// [`every`](Self::every) over each of `inputs`, the device's executors each over all of them in one batch.
    pub fn every_of(&self, inputs: &[Vec<u8>]) -> Vec<Runs> {
        Self::every_of_all(&[self], inputs).remove(0)
    }

    /// [`every_of`](Self::every_of) for several programs at once: each program's runs over each of `inputs`. The
    /// device's executor runs every program over every input as one batch (decision 0066), so a test of eleven
    /// programs builds its kernels once, with the eleven in it, not eleven times (decision 0576 §24: on an Arc 140V a
    /// build is 0.3-3 s).
    pub fn every_of_all(programs: &[&Executors<'_>], inputs: &[Vec<u8>]) -> Vec<Vec<Runs>> {
        let mut out: Vec<Vec<Runs>> = programs
            .iter()
            .map(|ex| inputs.iter().map(|bytes| vec![("cpu interpreter".to_owned(), ex.block.interpret(bytes)), ("cranelift".to_owned(), ex.cranelift.run(bytes))]).collect())
            .collect();
        Self::device_of(programs, inputs, &mut out);
        out
    }

    /// The device's executor, the forward carry (`metal_block_serial`), over every program and input in one batch,
    /// into each program's runs of each input: segments of three blocks, so that a file of a few blocks has segments
    /// that speculate their start and run again, and tier 1's 29.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_of(programs: &[&Executors<'_>], inputs: &[Vec<u8>], out: &mut [Vec<Runs>]) {
        use metal_program_backend::block_program::serial::{Segments, run_serial_split};
        use rerail_compiler::Error;
        let files: Vec<block::BatchFile<'_>> =
            (0..programs.len()).flat_map(|program| inputs.iter().map(move |bytes| block::BatchFile { bytes, program })).collect();
        let natives: Vec<_> = programs.iter().map(|ex| &ex.native).collect();
        for blocks in [3, 29] {
            let name = format!("device forward carry, segments of {blocks}");
            match run_serial_split(&natives, &files, Segments { blocks, warm: 1 }) {
                Ok(r) => {
                    assert_eq!(r.run.files.len(), files.len(), "{name}: a run per program and input");
                    for (runs, f) in out.iter_mut().flat_map(|p| p.iter_mut()).zip(r.run.files) {
                        runs.push((name.clone(), f.outputs));
                    }
                }
                // A device whose SIMD groups are neither 16 nor 32 lanes runs no block program (`block_program::ready`).
                Err(Error::UnsupportedBackend) if no_device() => {}
                Err(e) => panic!("{name} runs: {e:?}"),
            }
        }
    }

    /// No device executors off the device layer's targets.
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    fn device_of(_programs: &[&Executors<'_>], _inputs: &[Vec<u8>], _out: &mut [Vec<Runs>]) {}
}

