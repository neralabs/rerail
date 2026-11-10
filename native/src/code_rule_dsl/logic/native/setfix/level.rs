//! A level's first joins as one batch (decision 0100 §5, §6): the seed joins of every stratum of a level, sent to
//! the executor when the level's first stratum runs, and each one's pairs taken when its stratum runs.
//!
//! **What qualifies.** A seed join (a rule with no recursive atom, run once) whose first two steps are positive
//! atoms of native relations (`r:`, held by earlier levels), the second keyed on slots the first binds, and whose
//! every atom reads a native relation. Such a join reads nothing the level makes, so its pairs are known when the
//! level starts. A join too small to repay the batch is left alone (`MIN_LEFT`).
//!
//! **The launch.** The left rows are the first atom's live rows that pass its tests; the left keys are their values
//! of the slots the second atom is keyed by. The right is every row of the second atom's relation, in its key
//! columns, named by the relation's id and those columns, so a lane keeps its sort for every later join of the
//! chunk that reads it so (the executor's space, decision 0100 §4).
//!
//! **The pairs.** When the seed runs, it looks for its pairs by its spec (`JoinRule::spec`). When both relations
//! are the ones the launch read (by id and length), it runs from its third step over the pairs: each pair's left
//! row binds the first atom's slots, its right row is checked (live, every key, every test) and binds the second's
//! (`run_join_in`'s `paired`). Otherwise it runs as before. The pairs are the same set the host's index walk finds,
//! so the seed's rows are the same.

use super::spec::{JoinItem, JoinSpec, Source};
use super::{InRel, Interner, Key, Result, Src, Step, err, is_dead};
use rerail_accel::kernel::{JoinBy, Keys};
use rerail_accel::{Batch, Kernel, Launch, Pairs};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// A relation's id for the run: what the executor's space keeps its forms by.
#[derive(Debug)]
pub struct Uid(pub u64);

impl Default for Uid {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Uid(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A join's left rows below this are run by the host's walk alone: the batch's copies cost more than they save.
/// `RERAIL_LEVEL_JOIN_MIN` overrides it; `RERAIL_LEVEL_JOINS=off` turns the batch off.
const MIN_LEFT: usize = 4096;

fn min_left() -> usize {
    static MIN: OnceLock<usize> = OnceLock::new();
    *MIN.get_or_init(|| std::env::var("RERAIL_LEVEL_JOIN_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(MIN_LEFT))
}

/// Whether a level's seed joins are batched.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RERAIL_LEVEL_JOINS").map_or(true, |v| v != "off"))
}

/// A level's batch of seed joins, running until the first seed that reads it waits.
struct LevelBatch {
    running: Mutex<Option<rerail_accel::exec::Running>>,
    /// Each launch's pairs, taken once by its seed: a view of the lane's buffer when the GPU ran it, so nothing is
    /// copied (decision 0100 §9).
    outs: OnceLock<std::result::Result<Vec<Mutex<Option<Pairs>>>, String>>,
}

impl LevelBatch {
    fn out(&self, i: usize) -> Result<Pairs> {
        let outs = self.outs.get_or_init(|| {
            let running = self.running.lock().unwrap().take();
            match running {
                Some(r) => r.wait().map(|(done, _)| done.into_iter().map(|d| Mutex::new(d.out.into_pairs())).collect()),
                None => Err("a level's joins were already collected".into()),
            }
        });
        match outs {
            Ok(o) => o[i].lock().unwrap().take().ok_or_else(|| err("a level's join gave no pairs, or they were taken")),
            Err(e) => Err(err(format!("a level's joins: {e}"))),
        }
    }
}

/// A seed's pairs, pending: the relations they were made from and its left rows.
struct Pending {
    batch: Arc<LevelBatch>,
    at: usize,
    left: (u64, usize, usize),
    right: (u64, usize, usize),
    lids: Vec<u32>,
}

/// Pending pairs by the seed's spec.
static PENDING: LazyLock<Mutex<HashMap<u64, Pending>>> = LazyLock::new(Default::default);

/// A seed's pairs, taken: its first atom's row ids, and the pairs (an index into them, the second atom's row id).
pub struct Paired {
    pub lids: Vec<u32>,
    pub pairs: Pairs,
}

/// The digest of a join's spec: the key its pairs are kept by.
pub fn digest(spec: &JoinSpec) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    spec.hash(&mut h);
    h.finish()
}

fn fx(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    s.hash(&mut h);
    h.finish()
}

/// The relation's identity, as the pairs were made from it: its id, rows and dead rows.
fn ident(r: &InRel) -> (u64, usize, usize) {
    (r.uid.0, r.len(), r.ndead)
}

/// The first two steps of a join that qualifies: the relations, the first atom's tests and binds, and the second's
/// slot keys (column, slot).
fn qualifies(j: &super::JoinRule) -> Option<(&Arc<InRel>, &Arc<InRel>, &[(usize, usize)], Vec<(usize, usize)>)> {
    if j.fused.first().is_some_and(Option::is_some) {
        return None;
    }
    let (Some(Step::Atom { src: Src::In(r0, _), neg: false, keys: k0, binds: b0, tests: t0 }), Some(Step::Atom { src: Src::In(r1, _), neg: false, keys: k1, binds: b1, .. })) = (j.steps.first(), j.steps.get(1)) else {
        return None;
    };
    if !k0.is_empty() || b0.is_empty() || b1.is_empty() {
        return None;
    }
    let bound0: Vec<usize> = b0.iter().map(|&(_, s)| s).collect();
    let slot_keys: Vec<(usize, usize)> = k1.iter().filter_map(|&(c, k)| match k { Key::Slot(s) if bound0.contains(&s) => Some((c, s)), _ => None }).collect();
    if slot_keys.is_empty() {
        return None;
    }
    let _ = t0;
    Some((r0, r1, b0, slot_keys))
}

/// Sends the seed joins of a level (`specs`, each a seed's join spec) as one batch: the launches it sent.
pub fn prefetch(conn: &rusqlite::Connection, specs: &[Arc<JoinSpec>]) -> Result<usize> {
    if !on() {
        return Ok(0);
    }
    let symbols = super::symbols(conn)?;
    let mut syms = Interner::for_loading(&symbols);
    let mut launches = Vec::new();
    let mut pending = Vec::new();
    let (gate, opener) = rerail_accel::Gate::new();
    let mut fills: Vec<Fill> = Vec::new();
    for spec in specs {
        // Every atom a native relation's: the spec reads nothing the level makes, nor an input not loaded yet.
        let atoms = spec.items.iter().filter_map(|i| match i {
            JoinItem::Atom { src, .. } | JoinItem::Count { src, .. } | JoinItem::Extreme { src, .. } | JoinItem::StrJoin { src, .. } => Some(src),
            _ => None,
        });
        if atoms.clone().next().is_none() || !atoms.into_iter().all(|s| matches!(s, Source::Native(_))) {
            continue;
        }
        let key = digest(spec);
        // Sent already: twice in this batch, or by the level before (a chained join, decision 0100 §11).
        if pending.iter().any(|(k, _): &(u64, Pending)| *k == key) || PENDING.lock().unwrap().contains_key(&key) {
            continue;
        }
        let Ok(j) = super::join_rule(conn, spec, &mut syms) else { continue };
        let Some((r0, r1, b0, slot_keys)) = qualifies(&j) else { continue };
        let Step::Atom { tests: t0, .. } = &j.steps[0] else { continue };
        // The left: the first atom's live rows passing its tests; per key, the value its slot takes.
        let lids: Vec<u32> = (0..r0.len() as u32)
            .filter(|&id| !is_dead(&r0.dead, id) && {
                let row = r0.at(id as usize).read();
                t0.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
            })
            .collect();
        if lids.len() < min_left() || r1.len() == 0 {
            continue;
        }
        let col_of = |s: usize| b0.iter().find(|&&(_, b)| b == s).map(|&(c, _)| c).expect("a slot the first atom binds");
        // Staged: the columns' sizes are known now, their values are written after the batch is begun (below).
        let mut stage = |n: usize, cols: Vec<(Arc<InRel>, Option<Vec<u32>>, usize)>| -> Result<Keys> {
            let mut out = Vec::with_capacity(cols.len());
            for (rel, ids, c) in cols {
                let (col, w) = gate.column(n).map_err(err)?;
                fills.push(Fill { w, rel, ids, c });
                out.push(col);
            }
            Ok(out.into())
        };
        let left: Keys = stage(lids.len(), slot_keys.iter().map(|&(_, s)| (Arc::clone(r0), Some(lids.clone()), col_of(s))).collect())?;
        let right: Keys = stage(r1.len(), slot_keys.iter().map(|&(c, _)| (Arc::clone(r1), None, c)).collect())?;
        // The right's id in the space: the relation and its key columns.
        let right_id = fx(&format!("{} {:?}", r1.uid.0, slot_keys.iter().map(|&(c, _)| c).collect::<Vec<_>>()));
        launches.push(Launch::new(Kernel::Join { left, right, right_id: Some(right_id), by: JoinBy::Probe }));
        pending.push((key, Pending { batch: Arc::new(LevelBatch { running: Mutex::new(None), outs: OnceLock::new() }), at: launches.len() - 1, left: ident(r0), right: ident(r1), lids }));
    }
    if launches.is_empty() {
        return Ok(0);
    }
    let n = launches.len();
    SENT.fetch_add(n as u64, Ordering::Relaxed);
    let ex = crate::code_rule_dsl::logic::lanes::executor();
    let running = ex.begin_as("level joins", Batch(launches)).map_err(err)?;
    // The values, while the lanes commit and schedule: then the gate opens, and the device starts (decision 0100
    // §10). An error or a panic here opens it too (the opener's drop), so no lane waits for ever.
    ex.phase(rerail_accel::Lane::Cpu, "level joins: fill (after begin)", || {
        for f in &mut fills {
            let v = f.w.values();
            match &f.ids {
                Some(ids) => ids.iter().zip(v.iter_mut()).for_each(|(&id, o)| *o = f.rel.at(id as usize).read().at(f.c)),
                None => v.iter_mut().enumerate().for_each(|(id, o)| *o = f.rel.at(id).read().at(f.c)),
            }
        }
    });
    drop(fills);
    opener.open();
    let batch = Arc::new(LevelBatch { running: Mutex::new(Some(running)), outs: OnceLock::new() });
    let mut all = PENDING.lock().unwrap();
    for (key, mut p) in pending {
        p.batch = Arc::clone(&batch);
        all.insert(key, p);
    }
    Ok(n)
}

/// A staged column's values to write: rows `ids` of `rel` (all its rows when `None`), column `c`.
struct Fill {
    w: rerail_accel::col::Writer,
    rel: Arc<InRel>,
    ids: Option<Vec<u32>>,
    c: usize,
}

/// The pairs of `j`, if a level's batch made them from the relations it reads now.
pub fn take(j: &super::JoinRule) -> Result<Option<Paired>> {
    let Some(p) = PENDING.lock().unwrap().remove(&j.spec) else { return Ok(None) };
    let Some((r0, r1, ..)) = qualifies(j) else { return Ok(None) };
    let pairs = p.batch.out(p.at)?;
    if ident(r0) != p.left || ident(r1) != p.right {
        return Ok(None);
    }
    Ok(Some(Paired { lids: p.lids, pairs }))
}

/// Drops pairs no seed took (a stratum that ran in SQL, or declined), and what the lanes keep: the chunk's end.
pub fn clear() {
    let gone: Vec<Pending> = PENDING.lock().unwrap().drain().map(|(_, p)| p).collect();
    // A batch still running is waited for, so its lanes' buffers are not freed under it.
    for p in &gone {
        let _ = p.batch.out(p.at);
    }
    crate::code_rule_dsl::logic::lanes::executor().clear();
}

/// What the batches did so far: launches sent, seeds run from their pairs.
pub fn counts() -> (u64, u64) {
    (SENT.load(Ordering::Relaxed), USED.load(Ordering::Relaxed))
}

pub(super) static SENT: AtomicU64 = AtomicU64::new(0);
pub(super) static USED: AtomicU64 = AtomicU64::new(0);

/// Checks a qualifying spec's shape the first two steps need (for a test).
#[cfg(test)]
pub fn qualifying(j: &super::JoinRule) -> bool {
    qualifies(j).is_some()
}
