//! The GPU lane's space (decision 0100 §4): what a chunk keeps on the device between batches, and the joins of a
//! batch as one submission.
//!
//! **What is kept.** A join's right side, by the caller's id (`Kernel::Join::right_id`): its sort by key 0 for a
//! probe (`join::Keyed`), or its grouping by document for tiles (`tile::Resident`). A later join that names the
//! same id reads it without an upload or a sort. An entry goes when the caller forgets its id, or clears the space
//! at the chunk's end. When the kept bytes pass the limit, the entries least recently read go first, never one the
//! batch being run reads.
//!
//! **A batch's joins** take at most two command buffers, whatever their number:
//!
//! 1. The sorts of every right side not kept, then every probe's count, scan and scatter, and every tile join's
//!    count and scan. A sort is in the same serial encoder as the probes that read it, so they follow it.
//! 2. Only when needed: the scatters of the tile joins, sized from their counts, and the probes whose pairs
//!    outgrew the space expected for them.
//!
//! The pairs are each join's own buffers, read after completion. A tile join's pairs come in tile order, so they are
//! sorted into the kernel's order (left row, then right row).

use super::join::{Column, Keyed, Pass, ProbeJob, commit_wait, queue};
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
            let Some((&key, _)) = self.kept.iter().filter(|(_, k)| k.used < self.batch).min_by_key(|(_, k)| k.used) else { break };
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
    Probe { left: Vec<Column>, right: Arc<Keyed>, job: ProbeJob },
    Tile { left: Resident, right: Arc<Resident>, job: TileJob },
}

/// Runs a batch's joins (`ks`, each a `Kernel::Join`) in at most two command buffers: their pairs, in `ks`' order.
pub fn run(space: &std::sync::Mutex<Space>, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
    let mut space = space.lock().map_err(|_| "the gpu lane's space was poisoned")?;
    space.batch += 1;
    let queue = queue()?;
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let mut jobs = Vec::with_capacity(ks.len());
    // The columns the sorts read: they must outlive the first command buffer.
    let mut reads: Vec<Column> = Vec::new();
    // Built this batch: kept once the command buffer completed.
    let mut built: Vec<(u64, JoinBy, usize, usize, Entry)> = Vec::new();
    for k in ks {
        let Kernel::Join { left, right, right_id, by } = k else { return Err("gpu space: a launch that is not a join".into()) };
        let (_, nr) = Kernel::join_shape(left, right)?;
        let kept = match right_id {
            Some(id) => match built.iter().find(|b| b.0 == *id && b.1 == *by) {
                Some(b) if b.2 != nr => return Err(format!("join: id {id} names {} rows and {nr} in one batch", b.2)),
                Some(b) => Some(b.4.clone()),
                None => space.get(*id, *by, nr)?,
            },
            None => None,
        };
        let job = match by {
            JoinBy::Probe => {
                let right = match kept {
                    Some(Entry::Sorted(r)) => r,
                    _ => {
                        let (keyed, read) = Keyed::encode(&pass, upload(right)?)?;
                        reads.extend(read);
                        let keyed = Arc::new(keyed);
                        if let Some(id) = right_id {
                            built.push((*id, *by, nr, keyed.bytes, Entry::Sorted(Arc::clone(&keyed))));
                        }
                        keyed
                    }
                };
                let left = upload(left)?;
                let job = ProbeJob::encode(&pass, &left, &right, 0)?;
                Job::Probe { left, right, job }
            }
            JoinBy::Tile => {
                let right = match kept {
                    Some(Entry::Grouped(r)) => r,
                    _ => {
                        let r = Arc::new(group(right)?);
                        if let Some(id) = right_id {
                            built.push((*id, *by, nr, r.bytes, Entry::Grouped(Arc::clone(&r))));
                        }
                        r
                    }
                };
                let left = group(left)?;
                let keys: Vec<usize> = (0..k_len(k)).collect();
                let job = TileJob::count(&pass, &left, &keys, &right, &keys)?;
                Job::Tile { left, right, job }
            }
        };
        jobs.push(job);
    }
    pass.end();
    commit_wait(command)?;
    drop(reads);
    // The second pass: the tiles' scatters and the probes that overflowed.
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let mut second = false;
    let mut done = Vec::with_capacity(ks.len());
    for j in jobs {
        done.push(match j {
            Job::Probe { left, right, job } if job.overflowed() => {
                second = true;
                let job = job.rescatter(&pass, &left, &right)?;
                Job::Probe { left, right, job }
            }
            Job::Tile { left, right, job } => {
                let keys: Vec<usize> = (0..left.columns.len()).collect();
                let job = job.scatter(&pass, &left, &keys, &right, &keys)?;
                second |= job.total > 0;
                Job::Tile { left, right, job }
            }
            other => other,
        });
    }
    pass.end();
    if second {
        commit_wait(command)?;
    }
    for (id, by, rows, bytes, entry) in built {
        if let Entry::Sorted(k) = &entry {
            k.settle();
        }
        space.put(id, by, rows, bytes, entry);
    }
    Ok(done
        .into_iter()
        .map(|j| match j {
            Job::Probe { job, .. } => Output::Pairs(job.pairs()),
            Job::Tile { job, .. } => {
                let mut p = job.pairs();
                p.sort_unstable();
                Output::Pairs(p)
            }
        })
        .collect())
}

fn k_len(k: &Kernel) -> usize {
    match k {
        Kernel::Join { left, .. } => left.len(),
        _ => 0,
    }
}
