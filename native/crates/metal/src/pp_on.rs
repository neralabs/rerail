//! The preprocessing words on the device (decision 0348, stage 5): the words' compiled code as
//! MSL (`native_cpu_pp::msl_source`, the same IR as the CPU's), over a batch of TUs, one a
//! worker, each taken by ticket.
//!
//! Each TU's machine is laid out as on the CPU (`pp_words::prepare`), its buffers placed in one
//! arena block a batch, at GPU addresses, which the code reads as the CPU's reads pointers. When
//! the batch is done, each buffer the words write is copied back and the machine finished
//! (`pp_words::complete`), as the CPU's run finishes.

use crate::Error;
use metal::MTLSize;
use rerail_compiler::pp_words::{self, Bounds, Buf, Env, Expanded, File, Refused};
use rerail_compiler::program::Words;

/// A TU to run: the words, its files, its environment and its bounds.
pub type Job<'a> = (&'a Words, &'a [File<'a>], &'a Env<'a>, &'a Bounds);

/// A batch's run: each TU's result, in order, and the batch's GPU seconds.
pub struct Ran {
    pub results: Vec<Result<Expanded, Refused>>,
    pub gpu_seconds: f64,
    /// The arena's bytes the batch's buffers took.
    pub arena_bytes: usize,
}

/// A buffer the batch copies back: where it is in the host's memory and in the arena, its bytes.
struct Back {
    host: *mut u8,
    at: usize,
    bytes: usize,
}

/// The arena's bytes a batch starts with; a batch that needs more is laid out again in twice.
const FIRST_ARENA: usize = 64 << 20;

/// `jobs` run by `source`'s kernel `pp_batch`, on `workers` threads (at most the jobs).
pub fn run_batch(source: &str, jobs: &[Job<'_>], workers: usize) -> Result<Ran, Error> {
    let pipeline = crate::runtime::pipeline_named(source, "pp_batch")?;
    let mut size = FIRST_ARENA;
    loop {
        match laid_out(&pipeline, jobs, workers, size)? {
            Some(ran) => return Ok(ran),
            None => size *= 2,
        }
    }
}

/// The batch in an arena of `size` bytes, or `None` where it does not fit.
fn laid_out(pipeline: &metal::ComputePipelineState, jobs: &[Job<'_>], workers: usize, size: usize) -> Result<Option<Ran>, Error> {
    let arena = crate::pool::lease(size)?;
    let (base, gpu) = (arena.host(), arena.buffer().gpu_address());
    let mut used = 0usize;
    let mut full = false;
    let mut backs: Vec<Back> = Vec::new();
    let mut prepared = Vec::with_capacity(jobs.len());
    let mut ctx_at = Vec::with_capacity(jobs.len());
    {
        // Each buffer at the next 16 bytes, its written part copied in; its address the GPU's.
        let mut place = |b: Buf| -> u64 {
            let at = used.next_multiple_of(16);
            if at + b.room + 16 > size {
                full = true;
                return gpu;
            }
            used = at + b.room;
            // SAFETY: `b.len` bytes at `b.ptr` are written (`prepare`'s), and the arena has room.
            unsafe { std::ptr::copy_nonoverlapping(b.ptr, base.add(at), b.len) };
            if b.write {
                backs.push(Back { host: b.ptr, at, bytes: b.room });
            }
            gpu + at as u64
        };
        for &(program, files, env, bounds) in jobs {
            match pp_words::prepare(program, files, env, bounds, &mut place) {
                Ok(p) => {
                    let bytes = p.ctx_bytes();
                    let at = place(Buf { ptr: bytes.as_ptr().cast_mut(), len: bytes.len(), room: bytes.len(), write: false });
                    ctx_at.push(Some(at));
                    prepared.push(Ok(p));
                }
                Err(r) => {
                    ctx_at.push(None);
                    prepared.push(Err(r));
                }
            }
        }
    }
    if full {
        return Ok(None);
    }
    let n = jobs.len();
    // The TUs `prepare` refused never run: the contexts are the others'.
    let runnable: Vec<usize> = (0..n).filter(|&i| ctx_at[i].is_some()).collect();
    let ctxs: Vec<u64> = runnable.iter().map(|&i| ctx_at[i].expect("placed")).chain(std::iter::once(0)).collect();
    let order = crate::pool::upload(&ctxs)?;
    let results = crate::pool::zeroed::<u64>(runnable.len().max(1))?;
    let ticket = crate::pool::zeroed::<u32>(1)?;
    let count = runnable.len() as u32;
    let queue = crate::queue()?;
    let _guard = crate::runtime::metal_execution_guard()?;
    let gpu_seconds = metal::objc::rc::autoreleasepool(|| -> Result<f64, Error> {
        let command = queue.new_command_buffer();
        command.set_label("Rerail pp_batch");
        let encoder = command.new_compute_command_encoder();
        // The checked dispatch records the kernel by its encoder's label.
        encoder.set_label("pp_batch");
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_buffer(0, Some(order.buffer()), 0);
        encoder.set_buffer(1, Some(results.buffer()), 0);
        encoder.set_buffer(2, Some(ticket.buffer()), 0);
        encoder.set_bytes(3, 4, (&raw const count).cast());
        // The arena, and its first byte's GPU address: each access is the arena's pointer at an offset.
        encoder.set_buffer(4, Some(arena.buffer()), 0);
        encoder.set_bytes(5, 8, (&raw const gpu).cast());
        let threads = workers.clamp(1, runnable.len().max(1));
        let group = threads.min(pipeline.max_total_threads_per_threadgroup() as usize).min(32);
        let groups = threads.div_ceil(group);
        use crate::launch::CheckedComputeEncoder;
        encoder.checked_dispatch_thread_groups(command, MTLSize::new(groups as u64, 1, 1), MTLSize::new(group as u64, 1, 1));
        encoder.end_encoding();
        if let Err(refused) = crate::launch::commit_checked(command) {
            crate::launch::release_refused(command);
            return Err(refused);
        }
        crate::launch::wait_checked(command)?;
        Ok(crate::launch::gpu_seconds(command))
    })?;
    let words: Vec<u64> = results.read(runnable.len().max(1));
    // The written buffers back where they were, then each machine finished.
    for b in &backs {
        // SAFETY: each is a buffer the words write, its room reserved on the host (`prepare`).
        unsafe { std::ptr::copy_nonoverlapping(base.add(b.at), b.host, b.bytes) };
    }
    let mut r_of = vec![0u64; n];
    for (k, &i) in runnable.iter().enumerate() {
        r_of[i] = words[k];
    }
    let results = prepared
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let mut p = p?;
            let at = (ctx_at[i].expect("placed") - gpu) as usize;
            // SAFETY: the context as the code left it, then the buffers it wrote, within their rooms.
            unsafe {
                p.ctx_from(std::slice::from_raw_parts(base.add(at), std::mem::size_of_val(p.ctx_bytes())));
                pp_words::complete(p, r_of[i], rerail_compiler::native_cpu_pp::REASONS)
            }
        })
        .collect();
    Ok(Some(Ran { results, gpu_seconds, arena_bytes: used }))
}
