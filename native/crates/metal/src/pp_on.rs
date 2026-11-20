//! The preprocessing words on the device (decisions 0348 stage 5, 0440 step D): the words' compiled code as UKL
//! (`native_cpu_pp::ukl_source`, the same IR as the CPU's), on the process's device (Metal, OpenCL, the CPU's
//! backend), over a batch of TUs, one a worker, each taken by ticket.
//!
//! Each TU's machine is laid out as on the CPU (`pp_words::prepare`), its buffers placed in one arena block a batch,
//! at the arena's addresses, which the code reads as the CPU's reads pointers: the device's address of the arena's
//! first byte and up, or, on a device whose buffers have none (OpenCL 1.2), a base of its own (`NO_ADDRESS`): the code
//! reads each as an offset from it. When the batch is done, each buffer the words write is copied back and the machine
//! finished (`pp_words::complete`), as the CPU's run finishes.
//!
//! - **Rooms by use (stage L1):** every room is pages the device takes, so each TU runs first at
//!   the smallest of `Bounds::rung`'s rungs. A run refused past a room's bound runs again at the
//!   next, in the next batch, up to the job's own bounds (the last rung, the CPU's).
//! - **A batch past the device's budget** is split in halves, down to one TU.

use crate::Error;
use crate::lane::{self, Kernel};
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

/// The arena's bytes a batch starts with; a batch that needs more is laid out again in twice,
/// up to `MAX_ARENA`, past which it is split.
const FIRST_ARENA: usize = 64 << 20;
const MAX_ARENA: usize = 256 << 20;

/// The arena's base on a device whose buffers have no address of the device's: any 16-aligned value the words never
/// take for another, its offsets the arena's. Not 0, which the words read as no pointer.
const NO_ADDRESS: u64 = 1 << 40;

/// The kernel's launch: `PpBatch` (`clif_ukl`).
#[repr(C)]
#[derive(Clone, Copy)]
struct PpBatch {
    hb: u64,
    cs: u64,
    count: u32,
    pad: u32,
}
// SAFETY: plain integers, no padding.
unsafe impl rerail_device::Pod for PpBatch {}

/// `jobs` run by `source`'s kernel `pp_batch`, on `workers` threads (at most the jobs), each from
/// the smallest rung up.
pub fn run_batch(source: &str, jobs: &[Job<'_>], workers: usize) -> Result<Ran, Error> {
    let kernel = lane::kernel(source, "pp_batch", None);
    let n = jobs.len();
    let mut results: Vec<Option<Result<Expanded, Refused>>> = (0..n).map(|_| None).collect();
    let mut rung = vec![0usize; n];
    let mut pending: Vec<usize> = (0..n).collect();
    let mut all = Ran { results: Vec::new(), gpu_seconds: 0.0, arena_bytes: 0, dispatches: 0, rungs: [0; RUNGS] };
    while !pending.is_empty() {
        let bounds: Vec<Bounds> = pending.iter().map(|&i| if rung[i] + 1 < RUNGS { Bounds::rung(jobs[i].1, rung[i]) } else { jobs[i].3.clone() }).collect();
        let these: Vec<Job<'_>> = pending.iter().zip(&bounds).map(|(&i, b)| (jobs[i].0, jobs[i].1, jobs[i].2, b)).collect();
        let ran = fitted(&kernel, &these, workers)?;
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
    Ok(all)
}

/// `jobs` in arenas that fit: one, twice as large while it is full, else the jobs split in halves.
fn fitted(kernel: &Kernel, jobs: &[Job<'_>], workers: usize) -> Result<Ran, Error> {
    let mut size = FIRST_ARENA;
    loop {
        let laid = if size > MAX_ARENA { Err(Error::BackendCapacity) } else { laid_out(kernel, jobs, workers, size) };
        match laid {
            Ok(Some(ran)) => return Ok(ran),
            Ok(None) => size *= 2,
            Err(Error::BackendCapacity) if jobs.len() > 1 => {
                let (a, b) = jobs.split_at(jobs.len() / 2);
                let (mut a, b) = (fitted(kernel, a, workers)?, fitted(kernel, b, workers)?);
                a.results.extend(b.results);
                (a.gpu_seconds, a.arena_bytes, a.dispatches) = (a.gpu_seconds + b.gpu_seconds, a.arena_bytes.max(b.arena_bytes), a.dispatches + b.dispatches);
                return Ok(a);
            }
            Err(e) => return Err(e),
        }
    }
}

/// The batch in an arena of `size` bytes, or `None` where it does not fit.
fn laid_out(kernel: &Kernel, jobs: &[Job<'_>], workers: usize, size: usize) -> Result<Option<Ran>, Error> {
    let arena = crate::pool::lease(size)?;
    let (base, hb) = (arena.host(), arena.device_buffer().device_address().unwrap_or(NO_ADDRESS));
    let mut used = 0usize;
    let mut full = false;
    let mut backs: Vec<Back> = Vec::new();
    let mut prepared = Vec::with_capacity(jobs.len());
    let table_at;
    {
        // Each buffer at the next 16 bytes, its written part copied in; its address the arena's.
        let mut place = |b: Buf| -> u64 {
            let at = used.next_multiple_of(16);
            if at + b.room + 16 > size {
                full = true;
                return hb;
            }
            used = at + b.room;
            // SAFETY: `b.len` bytes at `b.ptr` are written (`prepare`'s), and the arena has room.
            unsafe { std::ptr::copy_nonoverlapping(b.ptr, base.add(at), b.len) };
            if b.write {
                backs.push(Back { host: b.ptr, at, bytes: b.room });
            }
            hb + at as u64
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
    // Its launch budget on this thread (0344: no process-wide lock; the buffers here are its own).
    let _scope = crate::runtime::evaluation_scope();
    let threads = workers.clamp(1, runnable.len().max(1));
    let group = threads.min(32);
    let groups = threads.div_ceil(group);
    let mut command = lane::Command::new("Rerail pp_batch");
    command
        .launch(kernel, "pp_batch")
        .buffer("ctxs", order.device_buffer())
        .buffer("results", results.device_buffer())
        .buffer("ticket", ticket.device_buffer())
        .buffer("heap", arena.device_buffer())
        // The contexts' table's row bytes (`cs`): a context's slot `j` is `8 j` rows on.
        .value("p", &PpBatch { hb, cs: stride, count, pad: 0 })
        .groups([groups as u64, 1, 1], [group as u64, 1, 1]);
    let gpu_seconds = command.run()?.device;
    let words: Vec<u64> = results.read(runnable.len().max(1));
    // The written buffers back where they were, then each machine finished.
    for b in &backs {
        // SAFETY: each is a buffer the words write, its room reserved on the host (`prepare`).
        unsafe { std::ptr::copy_nonoverlapping(base.add(b.at), b.host, b.bytes) };
    }
    let (table_at, columns) = ((table_at - hb) as usize, runnable.len());
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
