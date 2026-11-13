//! The GPU lane's space (decision 0100 §4): what a chunk keeps on the device between batches, and the joins of a
//! batch as one submission.
//!
//! **What is kept.** A join's right side, by the caller's id (`Kernel::Join::right_id`): its sort by key 0 for a
//! probe (`join::Keyed`), or its grouping by document for tiles (`tile::Resident`). A later join that names the
//! same id reads it without an upload or a sort. An entry goes when the caller forgets its id, or clears the space
//! at the chunk's end. When the kept bytes pass the limit, the entries least recently read go first, never one the
//! batch being run reads.
//!
//! **A batch's joins** take one command buffer, whatever their number, and a second only when needed:
//!
//! 1. The sorts of every right side not kept, then every probe and every tile join's count and scan. A sort is in
//!    the same serial encoder as the probes that read it, so they follow it.
//!    - A right side of at most 2,048 rows and three keys is sorted in one dispatch (`small_sort`), its raw 64-bit
//!      values read as they are; a larger one by the radix sort, narrowed first.
//!    - Every probe is in one dispatch (`probe_all`): each threadgroup counts its rows' pairs, reserves them in the
//!      batch's arena with one atomic, and writes them there.
//! 2. Only when needed: the scatters of the tile joins, sized from their counts, and the probes whose pairs did not
//!    fit the arena, each into space of its exact size.
//!
//! A probe's pairs are read from the arena block by block, in left row order. A tile join's pairs come in tile order, so they are
//! sorted into the kernel's order (left row, then right row).

use super::join::{Arena, Column, Fused, Keyed, Pass, ProbeOne, commit_wait, queue};
use super::tile::{Resident, TileJob};
use crate::kernel::{JoinBy, Keys, Kernel, Output};
use std::collections::HashMap;
use std::sync::Arc;

/// The bytes kept on the device at most (the relational share of the run's memory budget, decision 0032).
pub const LIMIT: usize = 256 << 20;

/// A kept right side.
#[derive(Clone)]
enum Entry {
    Sorted(Arc<Keyed>),
    Grouped(Arc<Resident>),
}

struct Kept {
    entry: Entry,
    rows: usize,
    bytes: usize,
    /// The batch that last read it.
    used: u64,
}

pub use crate::exec::SpaceCounts as Counts;

/// The kept right sides, by id and form.
#[derive(Default)]
pub struct Space {
    kept: HashMap<(u64, JoinBy), Kept>,
    bytes: usize,
    batch: u64,
    counts: Counts,
}

impl Space {
    pub fn forget(&mut self, id: u64) {
        for by in [JoinBy::Probe, JoinBy::Tile] {
            if let Some(k) = self.kept.remove(&(id, by)) {
                self.bytes -= k.bytes;
            }
        }
    }

    pub fn clear(&mut self) {
        self.kept.clear();
        self.bytes = 0;
    }

    pub fn counts(&self) -> Counts {
        Counts { entries: self.kept.len(), bytes: self.bytes, ..self.counts }
    }

    /// The kept entry of `id`, if its row count is `rows`; an error when the id names other rows.
    fn get(&mut self, id: u64, by: JoinBy, rows: usize) -> Result<Option<Entry>, String> {
        let batch = self.batch;
        match self.kept.get_mut(&(id, by)) {
            Some(k) if k.rows != rows => Err(format!("join: id {id} was kept with {} rows, now names {rows}", k.rows)),
            Some(k) => {
                k.used = batch;
                self.counts.hits += 1;
                Ok(Some(k.entry.clone()))
            }
            None => {
                self.counts.misses += 1;
                Ok(None)
            }
        }
    }

    fn put(&mut self, id: u64, by: JoinBy, rows: usize, bytes: usize, entry: Entry) {
        self.forget_one(id, by);
        // Least recently read first, never one this batch reads.
        while self.bytes + bytes > LIMIT {
            let Some((&key, _)) = self.kept.iter().filter(|(_, k)| k.used < self.batch).min_by_key(|(_, k)| k.used) else {
                break;
            };
            self.forget_one(key.0, key.1);
            self.counts.evicted += 1;
        }
        self.bytes += bytes;
        self.kept.insert((id, by), Kept { entry, rows, bytes, used: self.batch });
    }

    fn forget_one(&mut self, id: u64, by: JoinBy) {
        if let Some(k) = self.kept.remove(&(id, by)) {
            self.bytes -= k.bytes;
        }
    }
}

/// The columns of `keys` on the device, each as narrow as its span lets it be.
fn upload(keys: &Keys) -> Result<Vec<Column>, String> {
    keys.iter().map(|c| Column::upload(c)).collect()
}

/// A tile side: its key columns grouped by key 0's chunk.
fn group(keys: &Keys) -> Result<Resident, String> {
    let cols: Vec<&[i64]> = keys.iter().map(|c| &c[..]).collect();
    Resident::new(&cols, cols[0].len())
}

/// One join of the batch, between its passes.
enum Job {
    Probe { left: Vec<Column>, right: Arc<Keyed>, job: ProbeOne },
    Tile { left: Resident, right: Arc<Resident>, job: TileJob },
}

/// The distinct gates of a batch's staged columns.
fn gates(ks: &[&Kernel]) -> Vec<Arc<crate::col::Gate>> {
    let mut out: Vec<Arc<crate::col::Gate>> = Vec::new();
    for k in ks {
        if let Kernel::Join { left, right, .. } = k {
            for c in left.iter().chain(right.iter()) {
                if let Some(g) = c.gate() {
                    if !out.iter().any(|o| Arc::ptr_eq(o, g)) {
                        out.push(Arc::clone(g));
                    }
                }
            }
        }
    }
    out
}

/// Whether a right side takes the one-dispatch sort: at most `SMALL` rows and three keys.
fn small(right: &Keys) -> bool {
    right.len() <= 3 && right.first().is_some_and(|c| c.len() <= super::join::SMALL)
}

/// Runs a batch's joins (`ks`, each a `Kernel::Join`) in one command buffer, two only when a tile join's scatter or
/// a probe whose pairs did not fit needs one: their pairs, in `ks`' order.
///
/// When the batch has staged columns whose gate is not open (`crate::col`), the command buffer waits at its head
/// on the gates' events, and it is committed as soon as it is encoded: the host fills the columns while the commit
/// and the system's scheduling go on. A right side that takes the radix sort is narrowed from its values, so for
/// one that is staged the lane waits for its gate before encoding (the profile's `wait for fill`).
pub fn run(space: &std::sync::Mutex<Space>, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
    start(space, ks)?.finish()
}

/// A batch's joins encoded and committed, not yet waited for (`start`): the lane encodes its next submission while
/// the device runs this one, and `finish` reads it.
pub struct Started {
    command: super::join::Submitted,
    held: Vec<Arc<crate::col::Gate>>,
    committed: std::time::Instant,
    reads: Vec<Column>,
    tables: Vec<super::join::Held>,
    arena: Arena,
    jobs: Vec<Job>,
    /// This submission's scratch set (`join::scratch_set`): its second pass, if any, encodes on it.
    set: usize,
}

/// [`run`]'s first half: the batch's first command buffer encoded and committed, and its right sides kept (a sort
/// still running is waited for by whoever reads its runs). The space is not held after: the lane's next batch may
/// read what this one keeps.
pub fn start(space: &std::sync::Mutex<Space>, ks: &[&Kernel]) -> Result<Started, String> {
    let mut space = space.lock().map_err(|_| "the gpu lane's space was poisoned")?;
    // Each submission on the other scratch set from the one before: at most two are in flight.
    let set = super::join::scratch_set(0) ^ 1;
    super::join::scratch_set(set);
    space.batch += 1;
    let queue = queue()?;
    let command = queue.new_command_buffer();
    let device = &crate::gpu::shared::get().ok_or("no Metal device")?.device;
    let gates = gates(ks);
    let held: Vec<Arc<crate::col::Gate>> = crate::score::phase("encode", || gates.into_iter().filter(|g| g.hold(command, device)).collect());
    let pass = crate::score::phase("encode", || Pass::new(command));
    // The arena every probe's pairs go to: two a left row expected, at least 64 a probe.
    let (mut cap, mut blocks) = (0usize, 0usize);
    for k in ks {
        if let Kernel::Join { left, by: JoinBy::Probe, .. } = k {
            let nl = left.first().map_or(0, |c| c.len());
            cap += (2 * nl).max(64);
            blocks += super::join::probe_blocks(nl);
        }
    }
    let mut arena = crate::score::phase("encode", || Arena::new(cap, blocks))?;
    // The small sorts and every probe, described as they come and dispatched together after the radix sorts.
    let mut fused = Fused::default();
    let mut tables: Vec<super::join::Held> = Vec::new();
    let mut jobs = Vec::with_capacity(ks.len());
    // The columns the sorts read: they must outlive the first command buffer.
    let mut reads: Vec<Column> = Vec::new();
    // Built this batch: kept once the command buffer completed.
    let mut built: Vec<(u64, JoinBy, usize, usize, Entry)> = Vec::new();
    let (mut small_sorts, mut radix_sorts) = (0u64, 0u64);
    for k in ks {
        let Kernel::Join { left, right, right_id, by } = k else {
            return Err("gpu space: a launch that is not a join".into());
        };
        let (_, nr) = Kernel::join_shape(left, right)?;
        let kept = match right_id {
            Some(id) => match built.iter().find(|b| b.0 == *id && b.1 == *by) {
                Some(b) if b.2 != nr => {
                    return Err(format!("join: id {id} names {} rows and {nr} in one batch", b.2));
                }
                Some(b) => Some(b.4.clone()),
                None => space.get(*id, *by, nr)?,
            },
            None => None,
        };
        let job = match by {
            JoinBy::Probe => {
                let right_keyed = match kept {
                    Some(Entry::Sorted(r)) => r,
                    _ => {
                        let (keyed, read) = if small(right) {
                            small_sorts += 1;
                            let cols = crate::score::phase("upload", || right.iter().map(Column::raw).collect::<Result<Vec<_>, _>>())?;
                            crate::score::phase("encode", || fused.sort(cols))?
                        } else {
                            radix_sorts += 1;
                            for c in right.iter() {
                                if let Some(g) = c.gate() {
                                    crate::score::phase("wait for fill", || g.wait());
                                }
                            }
                            let cols = crate::score::phase("upload", || upload(right))?;
                            crate::score::phase("encode", || Keyed::encode(&pass, cols))?
                        };
                        reads.extend(read);
                        let keyed = Arc::new(keyed);
                        if let Some(id) = right_id {
                            built.push((*id, *by, nr, keyed.bytes, Entry::Sorted(Arc::clone(&keyed))));
                        }
                        keyed
                    }
                };
                let left = crate::score::phase("upload", || left.iter().map(Column::raw).collect::<Result<Vec<_>, _>>())?;
                let job = crate::score::phase("encode", || fused.probe(&left, &right_keyed, &mut arena))?;
                Job::Probe { left, right: right_keyed, job }
            }
            JoinBy::Tile => {
                let right = match kept {
                    Some(Entry::Grouped(r)) => r,
                    _ => {
                        let r = Arc::new(crate::score::phase("group (host)", || group(right))?);
                        if let Some(id) = right_id {
                            built.push((*id, *by, nr, r.bytes, Entry::Grouped(Arc::clone(&r))));
                        }
                        r
                    }
                };
                let left = crate::score::phase("group (host)", || group(left))?;
                let keys: Vec<usize> = (0..k_len(k)).collect();
                let job = crate::score::phase("encode", || TileJob::count(&pass, &left, &keys, &right, &keys))?;
                Job::Tile { left, right, job }
            }
        };
        jobs.push(job);
    }
    crate::score::phase("encode", || fused.dispatch(&pass, &arena, &mut tables))?;
    crate::score::phase("encode", || pass.end());
    crate::score::count("small sorts (fused)", small_sorts);
    crate::score::count("radix sorts", radix_sorts);
    let committed = std::time::Instant::now();
    let command = super::join::Submitted::commit(command)?;
    crate::score::phase("keep", || {
        for (id, by, rows, bytes, entry) in built {
            if let Entry::Sorted(k) = &entry {
                k.pending_on(&command);
            }
            space.put(id, by, rows, bytes, entry);
        }
    });
    Ok(Started { command, held, committed, reads, tables, arena, jobs, set })
}

impl Started {
    /// [`run`]'s second half: waits for the first command buffer, runs the second when needed, and reads the pairs.
    pub fn finish(self) -> Result<Vec<Output>, String> {
        let before = super::join::scratch_set(self.set);
        let r = self.finish_on_set();
        super::join::scratch_set(before);
        r
    }

    fn finish_on_set(self) -> Result<Vec<Output>, String> {
        let Started { command, held, committed, reads, tables, arena, jobs, .. } = self;
        let queue = queue()?;
        command.wait();
        // How long the host filled the staged columns after the commit: the delay the early commit hid.
        for g in &held {
            if let Some(at) = g.opened_at() {
                crate::score::add("fill after commit", 0.0, at.saturating_duration_since(committed).as_secs_f64());
            }
        }
        drop(reads);
        drop(tables);
        // The second pass, only when needed: the tiles' scatters, and the probes whose pairs did not fit the arena.
        let overflowed = arena.overflowed();
        // The probes whose pairs did not fit run again in one fused dispatch (the pipeline every probe uses, compiled
        // ahead), into one arena of their exact total.
        let spilled: Vec<bool> = jobs.iter().map(|j| matches!(j, Job::Probe { job, .. } if overflowed && job.spilled(&arena).0)).collect();
        let (mut cap2, mut blocks2) = (0usize, 0usize);
        for (j, _) in jobs.iter().zip(&spilled).filter(|(_, s)| **s) {
            if let Job::Probe { left, job, .. } = j {
                cap2 += job.spilled(&arena).1;
                blocks2 += super::join::probe_blocks(left[0].rows);
            }
        }
        let mut spill = if blocks2 > 0 { Some(Arena::new(cap2, blocks2)?) } else { None };
        let mut respill = Fused::default();
        let mut tables2: Vec<super::join::Held> = Vec::new();
        let mut second = false;
        let need = overflowed || jobs.iter().any(|j| matches!(j, Job::Tile { .. }));
        let mut done = Vec::with_capacity(jobs.len());
        if !need {
            done = jobs;
        } else {
            let command = queue.new_command_buffer();
            let pass = crate::score::phase("encode", || Pass::new(command));
            for (i, j) in jobs.into_iter().enumerate() {
                done.push(match j {
                    Job::Probe { left, right, .. } if spilled[i] => {
                        second = true;
                        let own = spill.as_mut().expect("the spills' arena");
                        let job = crate::score::phase("encode", || respill.probe(&left, &right, own))?;
                        Job::Probe { left, right, job }
                    }
                    Job::Tile { left, right, job } => {
                        let keys: Vec<usize> = (0..left.columns.len()).collect();
                        let job = crate::score::phase("encode", || job.scatter(&pass, &left, &keys, &right, &keys))?;
                        // Into the kernel's order on the device, after the scatter.
                        let job = crate::score::phase("encode", || job.sort(&pass, left.rows, right.rows))?;
                        second |= job.total > 0;
                        Job::Tile { left, right, job }
                    }
                    other => other,
                });
            }
            if let Some(own) = &spill {
                crate::score::phase("encode", || std::mem::take(&mut respill).dispatch(&pass, own, &mut tables2))?;
            }
            crate::score::phase("encode", || pass.end());
            if second {
                crate::score::count("second command buffers", 1);
                commit_wait(command)?;
            }
        }
        // Each probe's pairs as a view of its arena: no copy (decision 0100 §9).
        crate::score::phase("read pairs", || {
            done.into_iter()
                .enumerate()
                .map(|(i, j)| match j {
                    Job::Probe { job, .. } => {
                        let from = if spilled[i] { spill.as_ref().expect("the spills' arena") } else { &arena };
                        job.view(from).map(Output::View).ok_or_else(|| format!("join {i}: its pairs did not fit their exact space"))
                    }
                    Job::Tile { job, .. } => Ok(Output::Pairs(job.pairs())),
                })
                .collect()
        })
    }
}

fn k_len(k: &Kernel) -> usize {
    match k {
        Kernel::Join { left, .. } => left.len(),
        _ => 0,
    }
}
