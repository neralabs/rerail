//! The SME lane (decision 0098 §4): the bit product on the CPU's matrix unit, BMOPA in C (`kernels.c`).
//!
//! **One submission is the whole batch.** Every launch's operands are packed into 32-row panels (each distinct
//! `Bits` once: a self-product `a == b` packs one side), the work is cut into tiles (a few A panels against a group
//! of B panels small enough to stay in L2), and one worker per SME unit ([`lanes::sme_units`], one per CPU
//! cluster) enters streaming mode once and takes tiles from a shared counter until none are left. So a streaming
//! block is a worker's whole share of the batch, and the faster cluster simply takes more tiles.
//!
//! **Outputs.** Without `min`, the counts are written straight into the output matrix, the edges masked off. With
//! `min`, each 32 × 32 block is compared with the thresholds in streaming mode and written as one 32-bit mask per
//! row; the pairs are then read off the masks in `(i, j)` order. A self-product without `min` runs only the blocks
//! on and above the diagonal and writes each also transposed.
//!
//! macOS has no thread affinity: the workers ask for the interactive QoS class and the scheduler places them. Two
//! workers on one cluster share its unit (correct, only slower).

use crate::exec::Backend;
use crate::kernel::{Bits, Kernel, Output};
use crate::lanes::{self, Lane};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;

#[cfg(test)]
mod tests;

/// A tile of work, as `kernels.c`'s `rr_sme_tile`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Tile {
    ap: *const u32,
    apop: *const u32,
    bp: *const u32,
    bpop: *const u32,
    min: *const u32,
    out: *mut u32,
    k32: i64,
    ib0: i64,
    ib1: i64,
    jb0: i64,
    jb1: i64,
    nrows: i64,
    ncols: i64,
    ldc: i64,
    mode: i32,
    sym: i32,
}

const COUNTS: i32 = 0;
const MASK: i32 = 1;

unsafe extern "C" {
    fn rr_sme_worker_init();
    fn rr_sme_pack(rows: *const u64, r0: i64, r1: i64, words: i64, panels: *mut u32, pop: *mut u32);
    fn rr_sme_run(tiles: *const Tile, ntiles: i64, next: *mut i64) -> i64;
    fn rr_sme_mask_count(masks: *const u32, n: i64) -> i64;
    fn rr_sme_mask_pairs(masks: *const u32, r0: i64, r1: i64, npb: i64, out: *mut u32) -> i64;
}

/// The pairs a mask buffer holds, in `(i, j)` order, read off in C (two u32s a pair, the layout checked).
fn pairs(rows: usize, npb: usize, masks: &[u32]) -> Vec<(u32, u32)> {
    const { assert!(std::mem::size_of::<(u32, u32)>() == 8 && std::mem::offset_of!((u32, u32), 0) == 0 && std::mem::offset_of!((u32, u32), 1) == 4) };
    let masks = &masks[..rows * npb];
    let n = unsafe { rr_sme_mask_count(masks.as_ptr(), masks.len() as i64) } as usize;
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(n);
    let got = unsafe { rr_sme_mask_pairs(masks.as_ptr(), 0, rows as i64, npb as i64, out.as_mut_ptr().cast()) } as usize;
    assert_eq!(got, n);
    unsafe { out.set_len(n) };
    out
}

/// The tiles point into buffers the submission owns and outlives; workers write disjoint parts of the outputs.
struct Tiles(Vec<Tile>);
unsafe impl Send for Tiles {}
unsafe impl Sync for Tiles {}

/// This lane's backend, when the CPU has SME2.
pub fn backend() -> Option<Arc<dyn Backend>> {
    let units = lanes::sme_units();
    (lanes::sme2() && units > 0).then(|| Arc::new(Sme { units }) as Arc<dyn Backend>)
}

/// The SME lane: `units` workers, one per cluster.
pub struct Sme {
    pub units: usize,
}

/// A bit matrix in panels: `np` panels of 32 rows × `k32` words, and each padded row's popcount.
struct Packed {
    np: usize,
    k32: usize,
    panels: Vec<u32>,
    pop: Vec<u32>,
    /// `pop - 32 k32`, mod 2^32: the seed of the rows of A.
    popm: Vec<u32>,
}

fn pack(b: &Bits) -> Packed {
    let (np, k32) = (b.rows.div_ceil(32), b.words * 2);
    let mut panels = vec![0u32; np * 32 * k32];
    let mut pop = vec![0u32; np * 32];
    // Jobs of a few panels each, written in place.
    let per = 4usize;
    let job = |c: usize, p: &mut [u32], q: &mut [u32]| {
        let r0 = c * per * 32;
        let r1 = (r0 + per * 32).min(b.rows);
        unsafe { rr_sme_pack(b.data.as_ptr().add(r0 * b.words), 0, (r1 - r0) as i64, b.words as i64, p.as_mut_ptr(), q.as_mut_ptr()) };
    };
    if np <= per {
        // One job: on this thread, without a trip through the pool.
        job(0, &mut panels, &mut pop);
    } else {
        panels.par_chunks_mut(per * 32 * k32).zip(pop.par_chunks_mut(per * 32)).enumerate().for_each(|(c, (p, q))| job(c, p, q));
    }
    let base = (32 * k32) as u32;
    let popm = pop.iter().map(|&x| x.wrapping_sub(base)).collect();
    Packed { np, k32, panels, pop, popm }
}

/// The identity of a `Bits` within a submission: its buffer and shape.
fn key(b: &Bits) -> (usize, usize, usize) {
    (b.data.as_ptr() as usize, b.rows, b.words)
}

/// What a launch writes, while its tiles run.
enum Out {
    /// Done without the unit (no rows, or no words).
    Ready(Output),
    Counts { rows: usize, cols: usize, data: Vec<u32> },
    /// One 32-bit mask per padded row of A and panel of B; `min` padded with `u32::MAX`.
    Mask { rows: usize, npb: usize, masks: Vec<u32>, thr: Vec<u32> },
}

/// The B panels per group (a group stays in L2 while A panels stream past), and the A panels per tile.
fn tiling(npa: usize, npb: usize, k32: usize, units: usize) -> (usize, usize) {
    const GROUP_BYTES: usize = 2 << 20;
    let panel = 32 * k32 * 4;
    let g = (GROUP_BYTES / panel.max(1)).clamp(1, npb.max(1));
    let groups = npb.div_ceil(g);
    // About eight tiles per worker over the batch's launch, at most 8 A panels a tile.
    let a = (npa * groups).div_ceil(8 * units.max(1)).div_ceil(groups).clamp(1, 8);
    (g, a)
}

impl Sme {
    /// The submission, and the BMOPAs it issued (for the timing test).
    fn run(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, i64), String> {
        self.run_timed(ks).map(|(o, ops, _)| (o, ops))
    }

    /// [`Sme::run`], with its phases: packing (encode), the streaming workers (device; the spawned workers' CPU as
    /// `worker_cpu`), and the masks turned into pairs (decode).
    fn run_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, i64, crate::exec::Phases), String> {
        let mut ph = crate::exec::Phases::default();
        let t = std::time::Instant::now();
        // Validate and pack every distinct operand once.
        let mut packed: HashMap<(usize, usize, usize), Packed> = HashMap::new();
        for (n, k) in ks.iter().enumerate() {
            let Kernel::BitProduct { a, b, min } = k else {
                return Err(format!("launch {n}: the SME lane runs only the bit product, not {}", k.form().name()));
            };
            if a.words != b.words {
                return Err(format!("launch {n}: bit product of rows of {} and {} words", a.words, b.words));
            }
            if min.as_ref().is_some_and(|m| m.len() != b.rows) {
                return Err(format!("launch {n}: {} thresholds for {} rows of b", min.as_ref().unwrap().len(), b.rows));
            }
            if a.rows == 0 || b.rows == 0 || a.words == 0 {
                continue;
            }
            for x in [a, b] {
                packed.entry(key(x)).or_insert_with(|| pack(x));
            }
        }

        // Outputs, then tiles pointing into them.
        let mut outs: Vec<Out> = ks
            .iter()
            .map(|k| {
                let Kernel::BitProduct { a, b, min } = k else { unreachable!() };
                if a.rows == 0 || b.rows == 0 || a.words == 0 {
                    return Out::Ready(match min {
                        None => Output::Counts { rows: a.rows, cols: b.rows, data: vec![0; a.rows * b.rows] },
                        Some(m) => Output::Pairs(
                            (0..a.rows as u32).flat_map(|i| (0..b.rows as u32).filter(|&j| m[j as usize] == 0).map(move |j| (i, j))).collect(),
                        ),
                    });
                }
                match min {
                    None => Out::Counts { rows: a.rows, cols: b.rows, data: vec![0; a.rows * b.rows] },
                    Some(m) => {
                        let (npa, npb) = (a.rows.div_ceil(32), b.rows.div_ceil(32));
                        let mut mm = vec![u32::MAX; npb * 32];
                        mm[..b.rows].copy_from_slice(m);
                        Out::Mask { rows: a.rows, npb, masks: vec![0; npa * 32 * npb], thr: mm }
                    }
                }
            })
            .collect();
        let mut tiles = Vec::new();
        for (k, o) in ks.iter().zip(outs.iter_mut()) {
            let Kernel::BitProduct { a, b, .. } = k else { unreachable!() };
            let (pa, pb) = match o {
                Out::Ready(_) => continue,
                _ => (&packed[&key(a)], &packed[&key(b)]),
            };
            let sym = matches!(o, Out::Counts { .. }) && key(a) == key(b);
            let mut t = Tile {
                ap: pa.panels.as_ptr(),
                apop: pa.popm.as_ptr(),
                bp: pb.panels.as_ptr(),
                bpop: pb.pop.as_ptr(),
                min: std::ptr::null(),
                out: std::ptr::null_mut(),
                k32: pa.k32 as i64,
                ib0: 0,
                ib1: 0,
                jb0: 0,
                jb1: 0,
                nrows: a.rows as i64,
                ncols: b.rows as i64,
                ldc: b.rows as i64,
                mode: COUNTS,
                sym: sym as i32,
            };
            match o {
                Out::Counts { data, .. } => t.out = data.as_mut_ptr(),
                Out::Mask { npb, masks, thr, .. } => {
                    (t.out, t.min, t.ldc, t.mode) = (masks.as_mut_ptr(), thr.as_ptr(), *npb as i64, MASK);
                }
                Out::Ready(_) => unreachable!(),
            }
            let (g, per) = tiling(pa.np, pb.np, pa.k32, self.units);
            for jb0 in (0..pb.np).step_by(g) {
                for ib0 in (0..pa.np).step_by(per) {
                    let (ib1, jb1) = ((ib0 + per).min(pa.np), (jb0 + g).min(pb.np));
                    // A self-product runs only the blocks on and above the diagonal.
                    if sym && jb1 <= ib0 {
                        continue;
                    }
                    tiles.push(Tile { ib0: ib0 as i64, ib1: ib1 as i64, jb0: jb0 as i64, jb1: jb1 as i64, ..t });
                }
            }
        }

        // The workers: one per unit, one streaming block each. The calling thread (the executor's thread for this
        // lane) is one of them, so a submission of one tile starts no thread.
        ph.encode = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let tiles = Tiles(tiles);
        let next = AtomicI64::new(0);
        let workers = self.units.min(tiles.0.len());
        let (tiles, next) = (&tiles, &next);
        let work = move || unsafe {
            rr_sme_worker_init();
            rr_sme_run(tiles.0.as_ptr(), tiles.0.len() as i64, next.as_ptr())
        };
        let (ops, worker_cpu): (i64, f64) = crate::exec::scoped(|s| {
            let hs: Vec<_> = (1..workers)
                .map(|_| {
                    s.spawn(move || {
                        let c = crate::exec::thread_cpu();
                        let n = work();
                        (n, crate::exec::thread_cpu() - c)
                    })
                })
                .collect();
            let mine = if workers > 0 { work() } else { 0 };
            hs.into_iter().map(|h| h.join().expect("an SME worker panicked")).fold((mine, 0.0), |(a, c), (n, d)| (a + n, c + d))
        });
        drop(packed);
        (ph.device, ph.worker_cpu) = (Some(t.elapsed().as_secs_f64()), worker_cpu);
        let t = std::time::Instant::now();

        let outs = outs
            .into_iter()
            .map(|o| match o {
                Out::Ready(o) => o,
                Out::Counts { rows, cols, data } => Output::Counts { rows, cols, data },
                Out::Mask { rows, npb, masks, .. } => Output::Pairs(pairs(rows, npb, &masks)),
            })
            .collect();
        ph.decode = t.elapsed().as_secs_f64();
        Ok((outs, ops, ph))
    }
}

impl Backend for Sme {
    fn lane(&self) -> Lane {
        Lane::Sme
    }

    /// The bit product, any shape (rows are padded to the panel, words to nothing); its operands must agree.
    fn can(&self, k: &Kernel) -> bool {
        match k {
            Kernel::BitProduct { a, b, min } => a.words == b.words && min.as_ref().is_none_or(|m| m.len() == b.rows),
            _ => false,
        }
    }

    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        self.run(ks).map(|(o, _)| o)
    }
    fn submit_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, crate::exec::Phases), String> {
        self.run_timed(ks).map(|(o, _, p)| (o, p))
    }
}
