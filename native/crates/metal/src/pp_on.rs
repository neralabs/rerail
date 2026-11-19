//! The preprocessing words on the device (decision 0348, stage 5): the words' compiled code as
//! MSL (`native_cpu_pp::msl_source`, the same IR as the CPU's), over a batch of TUs, one a
//! worker, each taken by ticket.
//!
//! Each TU's machine is laid out as on the CPU (`pp_words::prepare`), its buffers placed in one
//! arena block a batch, at GPU addresses, which the code reads as the CPU's reads pointers. When
//! the batch is done, each buffer the words write is copied back and the machine finished
//! (`pp_words::complete`), as the CPU's run finishes.
//!
//! - **Rooms by use (stage L1):** every room is pages the device takes, so each TU runs first at
//!   the smallest of `Bounds::rung`'s rungs. A run refused past a room's bound runs again at the
//!   next, in the next batch, up to the job's own bounds (the last rung, the CPU's).
//! - **A batch past the device's budget** is split in halves, down to one TU.

use crate::Error;
use metal::MTLSize;
use rerail_compiler::pp_words::{self, Bounds, Buf, Env, Expanded, File, RUNGS, Refused};
use rerail_compiler::program::Words;

/// A TU to run: the words, its files, its environment and its bounds (its last rung's).
pub type Job<'a> = (&'a Words, &'a [File<'a>], &'a Env<'a>, &'a Bounds);

/// A batch's run: each TU's result, in order, and the batch's GPU seconds.
pub struct Ran {
    pub results: Vec<Result<Expanded, Refused>>,
    pub gpu_seconds: f64,
    /// The largest arena's bytes a dispatch's buffers took.
    pub arena_bytes: usize,
    /// The dispatches, and the TUs whose run ended at each rung.
    pub dispatches: usize,
    pub rungs: [usize; RUNGS],
}

/// A buffer the batch copies back: where it is in the host's memory and in the arena, its bytes.
struct Back {
    host: *mut u8,
    at: usize,
    bytes: usize,
}

/// The arena's bytes past which a batch is split.
const MAX_ARENA: usize = 256 << 20;

/// `jobs` run by `source`'s kernel `pp_batch`, on `workers` threads (at most the jobs), each from
/// the smallest rung up.
pub fn run_batch(source: &str, jobs: &[Job<'_>], workers: usize) -> Result<Ran, Error> {
    let pipeline = crate::runtime::pipeline_named(source, "pp_batch")?;
    if std::env::var_os("RERAIL_PP_DEVICE_TRACE").is_some() {
        let toks: usize = jobs.iter().map(|j| j.1.iter().map(|f| f.toks.len()).sum::<usize>()).sum();
        eprintln!("pp_on: {} jobs, {} tokens, footprint {} MB", jobs.len(), toks, rerail_blocks::footprint() >> 20);
    }
    let n = jobs.len();
    let mut results: Vec<Option<Result<Expanded, Refused>>> = (0..n).map(|_| None).collect();
    let mut rung = vec![0usize; n];
    let mut pending: Vec<usize> = (0..n).collect();
    let mut all = Ran { results: Vec::new(), gpu_seconds: 0.0, arena_bytes: 0, dispatches: 0, rungs: [0; RUNGS] };
    while !pending.is_empty() {
        let bounds: Vec<Bounds> = pending.iter().map(|&i| if rung[i] + 1 < RUNGS { Bounds::rung(jobs[i].1, rung[i]).within(jobs[i].3) } else { jobs[i].3.clone() }).collect();
        let these: Vec<Job<'_>> = pending.iter().zip(&bounds).map(|(&i, b)| (jobs[i].0, jobs[i].1, jobs[i].2, b)).collect();
        let ran = fitted(&pipeline, &these, workers)?;
        (all.gpu_seconds, all.arena_bytes, all.dispatches) = (all.gpu_seconds + ran.gpu_seconds, all.arena_bytes.max(ran.arena_bytes), all.dispatches + ran.dispatches);
        let mut next = Vec::new();
        for (&i, r) in pending.iter().zip(ran.results) {
            match r {
                Err(refused) if Bounds::is_room(refused.why) && rung[i] + 1 < RUNGS => {
                    rung[i] += 1;
                    next.push(i);
                }
                r => {
                    all.rungs[rung[i]] += 1;
                    results[i] = Some(r);
                }
            }
        }
        pending = next;
    }
    all.results = results.into_iter().map(|r| r.expect("each job ran")).collect();
    if std::env::var_os("RERAIL_PP_DEVICE_TRACE").is_some() {
        eprintln!("pp_on: footprint {} MB", rerail_blocks::footprint() >> 20);
    }
    Ok(all)
}

/// The arena bytes `jobs` take laid out (`laid_out`'s placement, read-only buffers once,
/// nothing copied): the first arena's size, so that a batch of small jobs leases a small arena.
fn need(jobs: &[Job<'_>]) -> usize {
    let mut used = 0usize;
    let mut shared: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let (mut slots, mut runnable) = (0, 0);
    {
        let mut place = |b: Buf| -> u64 {
            if b.write || shared.insert((b.ptr as usize, b.len)) {
                used = used.next_multiple_of(16) + b.room;
            }
            0
        };
        for &(program, files, env, bounds) in jobs {
            if let Ok(p) = pp_words::prepare(program, files, env, bounds, &mut place) {
                slots = p.ctx_bytes().len() / 8;
                runnable += 1;
            }
        }
    }
    used.next_multiple_of(16) + 8 * slots * runnable + 64
}

/// `jobs` in arenas that fit: one of the batch's need, twice as large while it is full, else
/// the jobs split in halves. A batch whose need is past `MAX_ARENA` is split before any arena is
/// leased (a lease is pages made: a lease that cannot fit is waste).
fn fitted(pipeline: &metal::ComputePipelineState, jobs: &[Job<'_>], workers: usize) -> Result<Ran, Error> {
    // The batch's own need, a power of two from 1 MB.
    let need = need(jobs);
    let mut size = if need > MAX_ARENA && jobs.len() > 1 { 2 * MAX_ARENA } else { need.next_power_of_two().max(1 << 20) };
    loop {
        let laid = if size > MAX_ARENA { Err(Error::BackendCapacity) } else { laid_out(pipeline, jobs, workers, size) };
        match laid {
            Ok(Some(ran)) => return Ok(ran),
            Ok(None) => size *= 2,
            Err(Error::BackendCapacity) if jobs.len() > 1 => {
                let (a, b) = jobs.split_at(jobs.len() / 2);
                let (mut a, b) = (fitted(pipeline, a, workers)?, fitted(pipeline, b, workers)?);
                a.results.extend(b.results);
                (a.gpu_seconds, a.arena_bytes, a.dispatches) = (a.gpu_seconds + b.gpu_seconds, a.arena_bytes.max(b.arena_bytes), a.dispatches + b.dispatches);
                return Ok(a);
            }
            Err(e) => return Err(e),
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
    let table_at;
    {
        // Each buffer at the next 16 bytes, its written part copied in; its address the GPU's. A
        // read-only buffer is placed once a batch: the jobs that read the same slice (a header's
        // text, which every element of a unit reads) read one copy.
        let mut shared: std::collections::HashMap<(usize, usize), u64> = std::collections::HashMap::new();
        let mut place = |b: Buf| -> u64 {
            if !b.write
                && let Some(&at) = shared.get(&(b.ptr as usize, b.len))
            {
                return at;
            }
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
            } else {
                shared.insert((b.ptr as usize, b.len), gpu + at as u64);
            }
            gpu + at as u64
        };
        for &(program, files, env, bounds) in jobs {
            prepared.push(pp_words::prepare(program, files, env, bounds, &mut place));
        }
        // The contexts across the batch (0408, L2): the table's row `j` is each context's 8 bytes
        // at `8 j`, a context a column, so a SIMD group's lanes read one field as adjacent words.
        // The TUs `prepare` refused never run: the columns are the others'.
        let runnable: Vec<&pp_words::Prepared<'_>> = prepared.iter().filter_map(|p| p.as_ref().ok()).collect();
        let slots = runnable.first().map_or(0, |p| p.ctx_bytes().len() / 8);
        let mut table = vec![0u64; slots * runnable.len()];
        for (k, p) in runnable.iter().enumerate() {
            for (j, w) in p.ctx_bytes().chunks_exact(8).enumerate() {
                table[j * runnable.len() + k] = u64::from_le_bytes(w.try_into().expect("8 bytes"));
            }
        }
        let bytes = table.len() * 8;
        table_at = place(Buf { ptr: table.as_mut_ptr().cast(), len: bytes, room: bytes, write: false });
    }
    if full {
        return Ok(None);
    }
    let n = jobs.len();
    let runnable: Vec<usize> = (0..n).filter(|&i| prepared[i].is_ok()).collect();
    let stride = 8 * runnable.len() as u64;
    let ctxs: Vec<u64> = (0..runnable.len() as u64).map(|k| table_at + 8 * k).chain(std::iter::once(0)).collect();
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
        // The contexts' table's row bytes: a context's slot `j` is `8 j` rows on.
        encoder.set_bytes(6, 8, (&raw const stride).cast());
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
    let (table_at, columns) = ((table_at - gpu) as usize, runnable.len());
    // SAFETY: the table, as the code left it, within the arena.
    let table: &[u64] = unsafe { std::slice::from_raw_parts(base.add(table_at).cast::<u64>(), stride as usize / 8 * prepared.iter().filter_map(|p| p.as_ref().ok()).next().map_or(0, |p| p.ctx_bytes().len() / 8)) };
    let mut k = 0;
    let results = prepared
        .into_iter()
        .map(|p| {
            let mut p = p?;
            let column: Vec<u8> = (0..p.ctx_bytes().len() / 8).flat_map(|j| table[j * columns + k].to_le_bytes()).collect();
            let r = words[k];
            k += 1;
            // SAFETY: the context as the code left it, then the buffers it wrote, within their rooms.
            unsafe {
                p.ctx_from(&column);
                pp_words::complete(p, r, rerail_compiler::native_cpu_pp::REASONS)
            }
        })
        .collect();
    Ok(Some(Ran { results, gpu_seconds, arena_bytes: used, dispatches: 1, rungs: [0; RUNGS] }))
}
