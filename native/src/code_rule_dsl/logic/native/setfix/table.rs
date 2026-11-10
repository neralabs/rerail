//! The `count table` form (decision 0098 §1): a count step answered from a table of counts per key, not by walking
//! the matching rows at every probe.
//!
//! A count step (`$c = count { r(k1, …, kn, _) }`) walks, at each binding that reaches it, the rows of `r` that
//! match its key, and counts them. When many bindings reach it, the walks add up to more than reading `r` once.
//! The count table reads `r` once instead:
//!
//! - **The histogram.** The relation's rows, by their key's value, are given a 16-bit code within a partition (0095
//!   §6's local codes; the partitions are by a hash of the key, so each holds at most 2^16 distinct keys), and the
//!   executor's `Histogram` kernel counts each partition's codes. A probe looks its key up and reads its count. The
//!   table belongs to the relation, as its indexes do: every later step that counts it by the same columns reads the
//!   same table, in any stratum.
//! - **Counts per row.** When the key comes whole from the join's first step, a scan, every row of the scan has its
//!   count before the walk: the scan's keys against the relation's, one `CountTable` launch (counts only, no pairs),
//!   and a probe reads its row's count by position.
//!
//! **When.** A step starts by walking. Once its walks have read as many rows as building the table reads (the
//! relation's rows, and the scan's for counts per row), it builds the table and reads it from then on. So it never
//! spends more than about twice what walking alone would: the rule a planner without estimates can keep, and the
//! one the estimates refine (the tables a chunk found worth building are built ahead in the next chunk's device
//! waves: `prefetch`).
//!
//! Both forms are exact: the count of a key is the number of live rows with that key, as the walk counts them.

use super::{InRel, Key, Src, Step, is_dead};
use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
use rerail_accel::{Batch, Kernel, Launch, Output};
use rustc_hash::{FxHashMap as HashMap, FxHasher};
use std::cell::{Cell, OnceCell};
use std::collections::BTreeSet;
use std::hash::Hasher;
use std::sync::{Arc, LazyLock, Mutex};

/// A step's walks must read at least this many rows before a table is built for it.
const FLOOR: u64 = 4096;
/// Distinct keys a partition is planned for: well under the 2^16 its codes can name.
const PER_PART: usize = 4096;
/// The code a key absent from the relation is given in the counts per row (none of the relation's keys has it).
const ABSENT: u16 = u16::MAX;

/// A relation's counts by the values of some of its columns.
pub(super) struct Table {
    /// Each key's place in `counts`.
    slot: HashMap<Box<[i64]>, u32>,
    counts: Vec<u32>,
}

impl Table {
    pub(super) fn count(&self, key: &[i64]) -> i64 {
        self.slot.get(key).map_or(0, |&s| i64::from(self.counts[s as usize]))
    }
}

/// A count step's state in one join: the rows its walks read, and its counts per row once built.
#[derive(Default)]
pub(super) struct StepForm {
    spent: Cell<u64>,
    tried: Cell<bool>,
    /// Whether the relation's tables were looked at for one by this step's columns (once: another step's, or a
    /// wave's, is read as this step's own).
    looked: Cell<bool>,
    table: OnceCell<Arc<Table>>,
    per_row: OnceCell<Vec<u32>>,
}

/// The tables a run found worth building, by relation (its table's name) and columns: the next chunk builds them ahead,
/// in its levels' waves.
static PAID: LazyLock<Mutex<BTreeSet<(String, Vec<usize>)>>> = LazyLock::new(Default::default);

/// What the forms did in this process, for the profile.
#[derive(Default, Clone, Copy)]
pub(super) struct Built {
    pub(super) histograms: u64,
    pub(super) per_row: u64,
    pub(super) rows: u64,
}
pub(super) static BUILT: Mutex<Built> = Mutex::new(Built { histograms: 0, per_row: 0, rows: 0 });

fn hash(key: &[i64]) -> u64 {
    let mut h = FxHasher::default();
    for v in key {
        h.write_i64(*v);
    }
    h.finish()
}

#[cfg(test)]
thread_local! {
    /// Tests: build a step's table at its first probe, whatever its walks cost.
    pub(super) static AT_ONCE: Cell<bool> = const { Cell::new(false) };
}
#[cfg(not(test))]
fn at_once() -> bool { false }
#[cfg(test)]
fn at_once() -> bool { AT_ONCE.with(Cell::get) }

fn off_on() -> bool {
    std::env::var("RERAIL_COUNT_TABLE").is_ok_and(|v| v == "off")
}

/// A relation's keys coded for the kernels: the key of each of its live rows by `cols`, its partition and code.
struct Coded {
    parts: usize,
    /// Each distinct key's (partition, code).
    codes: HashMap<Box<[i64]>, (u32, u16)>,
    /// Per partition, its distinct keys.
    sizes: Vec<u32>,
    /// The relation's live rows' (partition, code).
    rows: Vec<(u32, u16)>,
}

/// Codes `rel`'s live rows by `cols`: `None` when a partition would hold too many keys for 16-bit codes (never, at
/// the planned load; then a larger partition count is tried).
fn code(rel: &InRel, cols: &[usize]) -> Coded {
    let n = rel.len();
    let mut parts = n.div_ceil(PER_PART).max(1);
    'retry: loop {
        let mut codes: HashMap<Box<[i64]>, (u32, u16)> = HashMap::default();
        let mut sizes = vec![0u32; parts];
        let mut rows = Vec::with_capacity(n);
        let mut key = vec![0i64; cols.len()];
        for id in 0..n {
            if is_dead(&rel.dead, id as u32) {
                continue;
            }
            let row = rel.at(id).read();
            for (k, &c) in key.iter_mut().zip(cols) {
                *k = row.at(c);
            }
            let pc = match codes.get(key.as_slice()) {
                Some(&pc) => pc,
                None => {
                    let p = (hash(&key) % parts as u64) as usize;
                    // One code is kept for keys the relation lacks (`ABSENT`).
                    if sizes[p] as usize >= usize::from(ABSENT) {
                        parts *= 4;
                        continue 'retry;
                    }
                    let pc = (p as u32, sizes[p] as u16);
                    sizes[p] += 1;
                    codes.insert(key.clone().into_boxed_slice(), pc);
                    pc
                }
            };
            rows.push(pc);
        }
        return Coded { parts, codes, sizes, rows };
    }
}

/// Codes grouped by partition: the offsets (`parts + 1`), the codes, and each item's position in the grouped order.
fn group(parts: usize, items: impl Iterator<Item = (u32, u16)> + Clone) -> (Vec<u32>, Vec<u16>, Vec<u32>) {
    let mut off = vec![0u32; parts + 1];
    for (p, _) in items.clone() {
        off[p as usize + 1] += 1;
    }
    for p in 0..parts {
        off[p + 1] += off[p];
    }
    let mut at = off.clone();
    let total = off[parts] as usize;
    let mut codes = vec![0u16; total];
    let mut pos = Vec::with_capacity(total);
    for (p, c) in items {
        let i = at[p as usize];
        at[p as usize] += 1;
        codes[i as usize] = c;
        pos.push(i);
    }
    (off, codes, pos)
}

/// A histogram's launch over `rel` by `cols`, and how to make its table from the kernel's counts.
pub(super) struct Pending {
    coded: Coded,
    width: u32,
}

pub(super) fn histogram_launch(rel: &InRel, cols: &[usize]) -> (Pending, Launch) {
    let coded = code(rel, cols);
    let width = coded.sizes.iter().copied().max().unwrap_or(0).max(1);
    let (off, codes, _) = group(coded.parts, coded.rows.iter().copied());
    let k = Kernel::Histogram { off: off.into(), codes: codes.into(), width };
    (Pending { coded, width }, Launch::new(k))
}

pub(super) fn histogram_table(p: Pending, out: Output) -> Result<Table, String> {
    let Output::Counts { data, .. } = out else { return Err("a histogram's launch returned no counts".into()) };
    let width = p.width;
    let slot = p.coded.codes.into_iter().map(|(k, (part, code))| (k, part * width + u32::from(code))).collect();
    let mut b = BUILT.lock().unwrap();
    b.histograms += 1;
    b.rows += p.coded.rows.len() as u64;
    Ok(Table { slot, counts: data })
}

/// `rel`'s table by `cols`, built now (one launch) unless it is already.
fn table(rel: &InRel, cols: &[usize]) -> Result<Arc<Table>, String> {
    if let Some(t) = rel.tables.lock().unwrap().get(cols) {
        return Ok(Arc::clone(t));
    }
    let (p, launch) = {
        let _h = crate::code_rule_dsl::logic::lanes::prof::host("count table (histogram)", "encode: code the keys, group by partition");
        histogram_launch(rel, cols)
    };
    let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run("count table (histogram)", Batch(vec![launch]))?;
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("count table (histogram)", "decode: the slot map of key to count");
    let t = Arc::new(histogram_table(p, done.remove(0).out)?);
    rel.tables.lock().unwrap().entry(cols.to_vec()).or_insert_with(|| Arc::clone(&t));
    Ok(t)
}

/// The key columns of a count step over an input, when it can read a table: every key bound, no column tests.
fn cols_of(src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)]) -> Option<(Arc<InRel>, Vec<usize>)> {
    let Src::In(rel, _) = src else { return None };
    if !tests.is_empty() || keys.is_empty() {
        return None;
    }
    Some((Arc::clone(rel), keys.iter().map(|&(c, _)| c).collect()))
}

/// The count at step `i`, from its table, when it has one: `key` the key's values, `at0` the row of the join's first
/// step.
#[inline]
pub(super) fn count(form: &StepForm, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)], key: &[i64], at0: u32) -> Option<i64> {
    if let Some(per) = form.per_row.get() {
        return Some(i64::from(per[at0 as usize]));
    }
    if let Some(t) = form.table.get() {
        return Some(t.count(key));
    }
    if form.looked.get() {
        return None;
    }
    form.looked.set(true);
    let (rel, cols) = cols_of(src, keys, tests)?;
    let t = rel.tables.lock().unwrap().get(&cols).cloned()?;
    let n = t.count(key);
    let _ = form.table.set(t);
    Some(n)
}

/// Notes that step `i` of `steps` walked `walked` rows for one probe, and builds its table once walking has cost as
/// much as building it would (see the module's comment).
pub(super) fn spent(form: &StepForm, steps: &[Step], i: usize, walked: u64) {
    if form.tried.get() {
        return;
    }
    let Step::Scalar { src, keys, tests, kind: ScalarKind::Count, col: None, .. } = &steps[i] else { return };
    let Some((rel, cols)) = cols_of(src, keys, tests) else {
        form.tried.set(true);
        return;
    };
    let spent = form.spent.get() + walked + 1;
    form.spent.set(spent);
    let left = per_row_left(steps, i);
    let cost = rel.len() as u64 + left.as_ref().map_or(0, |(l, _)| l.len() as u64) + FLOOR;
    if spent < cost && !at_once() {
        return;
    }
    form.tried.set(true);
    crate::code_rule_dsl::logic::lanes::prof::count(if left.is_some() { "count per row" } else { "count table (histogram)" }, "rows walked by probes before the build paid", spent);
    if off_on() {
        return;
    }
    // A table the relation already has (another step's) is read as it is.
    if let Some(t) = rel.tables.lock().unwrap().get(&cols) {
        let _ = form.table.set(Arc::clone(t));
        return;
    }
    let built = match left {
        Some((l, of_key)) => per_row(&rel, &cols, &l, &of_key, keys).map(|per| {
            crate::code_rule_dsl::logic::profile::decision("count table (decision 0098 §1)", || {
                format!("{} by columns {cols:?}: counts per row of the join's first scan", rel.name.get().map_or("an input", String::as_str))
            });
            let _ = form.per_row.set(per);
        }),
        None => table(&rel, &cols).map(|t| {
            crate::code_rule_dsl::logic::profile::decision("count table (decision 0098 §1)", || {
                format!("{} by columns {cols:?}: a histogram, once its count's walks read as many rows", rel.name.get().map_or("an input", String::as_str))
            });
            let _ = form.table.set(t);
            if let Some(name) = rel.name.get() {
                PAID.lock().unwrap().insert((name.clone(), cols.clone()));
            }
        }),
    };
    // A lane's failure is an error of the run's executor; the walk is exact without the table, so the step walks on
    // and the failure is in the profile.
    if let Err(e) = built {
        crate::code_rule_dsl::logic::profile::decision("count table (decision 0098)", || format!("not built: {e}"));
    }
}

/// When step `i`'s key comes whole from step 0, a scan of an input: the scan's relation, and per key column, the scan's
/// column holding it (`None` for a constant).
fn per_row_left(steps: &[Step], i: usize) -> Option<(Arc<InRel>, Vec<Option<usize>>)> {
    let Step::Atom { src: Src::In(l, _), neg: false, keys: k0, binds, .. } = steps.first()? else { return None };
    if !k0.is_empty() || i == 0 {
        return None;
    }
    let Step::Scalar { keys, kind: ScalarKind::Count, .. } = &steps[i] else { return None };
    let mut of_key = Vec::with_capacity(keys.len());
    for &(_, k) in keys {
        match k {
            Key::Const(_) => of_key.push(None),
            Key::Slot(s) => of_key.push(Some(binds.iter().find(|&&(_, b)| b == s)?.0)),
        }
    }
    Some((Arc::clone(l), of_key))
}

/// The count, for every row of `left`, of `rel`'s rows whose `cols` equal the row's key (`of_key`, with `keys`'
/// constants): one `CountTable` launch.
fn per_row(rel: &InRel, cols: &[usize], left: &InRel, of_key: &[Option<usize>], keys: &[(usize, Key)]) -> Result<Vec<u32>, String> {
    let h = crate::code_rule_dsl::logic::lanes::prof::host("count per row", "encode: code both sides, group by partition");
    let coded = code(rel, cols);
    let (roff, right, _) = group(coded.parts, coded.rows.iter().copied());
    // Each left row's key, coded as the relation's are; a key it lacks is `ABSENT` (no right row has that code).
    let mut key = vec![0i64; cols.len()];
    let lefts: Vec<(u32, u16)> = (0..left.len())
        .map(|id| {
            let row = left.at(id).read();
            for ((k, of), &(_, kk)) in key.iter_mut().zip(of_key).zip(keys) {
                *k = match (of, kk) {
                    (Some(c), _) => row.at(*c),
                    (None, Key::Const(v)) => v,
                    (None, Key::Slot(_)) => unreachable!("a slot's key has its column"),
                };
            }
            match coded.codes.get(key.as_slice()) {
                Some(&pc) => pc,
                None => ((hash(&key) % coded.parts as u64) as u32, ABSENT),
            }
        })
        .collect();
    let (loff, lcodes, pos) = group(coded.parts, lefts.iter().copied());
    let k = Kernel::CountTable { loff: loff.into(), left: lcodes.into(), roff: roff.into(), right: right.into(), emit: false };
    drop(h);
    let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run("count per row", Batch(vec![Launch::new(k)]))?;
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("count per row", "decode: counts back in row order");
    let Output::PerRow(counts) = done.remove(0).out else { return Err("a count table's launch returned no counts per row".into()) };
    let mut b = BUILT.lock().unwrap();
    b.per_row += 1;
    b.rows += (coded.rows.len() + left.len()) as u64;
    Ok(pos.iter().map(|&p| counts[p as usize]).collect())
}

/// The device wave of a level (0095 §5): the tables its strata will read that an earlier chunk found worth building,
/// built now in one batch, so each lane gets one submission for the level. `wants`: the level's count reads, by
/// relation and columns; `held`: the relation's rows, if this run holds them. Returns (launches, distinct relations).
pub(super) fn prefetch(wants: &BTreeSet<(String, Vec<usize>)>, held: impl Fn(&str) -> Option<Arc<InRel>>) -> Result<(usize, usize), String> {
    if off_on() {
        return Ok((0, 0));
    }
    let paid = PAID.lock().unwrap().clone();
    let mut pending = Vec::new();
    let mut launches = Vec::new();
    for (name, cols) in wants.intersection(&paid) {
        let Some(rel) = held(name) else { continue };
        if rel.tables.lock().unwrap().contains_key(cols) {
            continue;
        }
        let _h = crate::code_rule_dsl::logic::lanes::prof::host("count table (level wave)", "encode: code the keys, group by partition");
        let (p, l) = histogram_launch(&rel, cols);
        pending.push((rel, cols.clone(), p));
        launches.push(l);
    }
    if launches.is_empty() {
        return Ok((0, 0));
    }
    let n = launches.len();
    let rels: BTreeSet<*const InRel> = pending.iter().map(|(r, _, _)| Arc::as_ptr(r)).collect();
    let (done, _) = crate::code_rule_dsl::logic::lanes::prof::run("count table (level wave)", Batch(launches))?;
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("count table (level wave)", "decode: the slot map of key to count");
    for ((rel, cols, p), d) in pending.into_iter().zip(done) {
        let t = Arc::new(histogram_table(p, d.out)?);
        rel.tables.lock().unwrap().entry(cols).or_insert(t);
    }
    Ok((n, rels.len()))
}

/// What the forms built so far in this process.
pub(super) fn built() -> Built {
    *BUILT.lock().unwrap()
}
