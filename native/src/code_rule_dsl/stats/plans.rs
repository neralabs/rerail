//! Compiled plans, the statistics their decisions read, run-time feedback and SQLite's own
//! statistics (docs/final-shape/03 §6, 02 "stat-driven compiler"; 05 Phase 3 row 8). Written new:
//! no source has these. The rules come from PIAT spec §4.9 and §11 (notes/t1-piat-spec.md):
//! - **`compiled_plan`** keeps each plan under its identity (`identity::CompiledPlanId`, N's
//!   `SemanticId` in its own domain) with its decisions and the program they produced.
//!   Every statistic a decision read is stored with its validity interval: the log2 bucket of the
//!   value read (§4.9's "log2 buckets of every statistic read"). A plan stays valid while every
//!   statistic stays in its bucket, so a refresh re-plans only when a decision could change (§11
//!   :273), and nothing counts rows at run time (DLRE trap T18).
//! - **Validity is re-checked in place** (§11 :283): [`Plans::valid`] reads the intervals of many
//!   plans where they are kept and compares them with the values the caller read in its own one
//!   batched statement (`inputs::Statistics::reads`).
//! - **`plan_feedback`** holds, per plan and backend, an EWMA of the observed selectivity and cost
//!   per root (§2.5b, §4.8: "sel from `plan_feedback` EWMA, else the prior"). It also answers H4:
//!   a regex's matched-term count enters the estimate after its first fill.
//! - **`sqlite_stat1`** is written from `fact_stats` for fact relations held as SQL tables, never
//!   by `ANALYZE` over the data (02 "SQL join order"); `ANALYZE sqlite_schema` only creates the
//!   table and reloads the planner's copy.
//!
//! The three are kept facts of the store (t33 step 6, decisions 0104 and 0456), where three SQLite
//! tables held them: one space, a partition per plan, `compiled_plan(p0..p3, compiler, decisions
//! bytes, program bytes)`, `compiled_plan_bytes(p0..p3, at, word)` (both byte strings, eight bytes a
//! word, read where they lie), `compiled_plan_read(p0..p3, stat, lo, hi)` and `plan_feedback(p0..p3,
//! backend, selectivity, ns per root, samples, at)`, a statistic's and a backend's name kept as its
//! hash. A read takes a space opened before it (`Plans::open`), and a write is staged until it is
//! flushed (`Plans::flush`), so either may run inside a candidate transaction, which holds the
//! store's connection; the write is queued behind the run.
//!
//! Compiled programs also live in memory, in the bounded cache copied from Data-layer-rule-
//! expressivity (`cache.rs`), keyed the same way and by the store they were read from or written
//! to: the store is their durable copy, so a plan another store holds is read from this one or
//! compiled into it, never answered from memory alone.

use super::cache::Cache;
use crate::content_store::kept_facts::{BlockFlat, KeptPut, KeptSpace, KeptWhat};
use crate::content_store::{ContentStore, StoreResult};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use rusqlite::{Connection, OptionalExtension, Result, params};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The version of the decisions and programs `compiled_plan` holds; a plan compiled by another is
/// not read.
pub const COMPILER_VERSION: i64 = 2;
/// Bytes of compiled programs kept in memory.
const MEMORY: usize = 16 * 1024 * 1024;
/// Weight of the newest observation in `plan_feedback`.
const FEEDBACK_WEIGHT: f64 = 0.25;

const PLAN: &str = "compiled_plan";
const BYTES: &str = "compiled_plan_bytes";
const READ: &str = "compiled_plan_read";
const FEEDBACK: &str = "plan_feedback";

/// The bytes of a plan's `identity::CompiledPlanId`.
pub type PlanId = [u8; 32];

/// The interval of values over which a decision that read `value` stays valid: its log2 bucket.
pub fn interval(value: u64) -> (u64, u64) {
    if value == 0 {
        return (0, 0);
    }
    let low = 1u64 << value.ilog2();
    (low, low.saturating_sub(1).saturating_add(low))
}

/// A compiled plan: its decisions (with the statistic values they read, for `explain`) and the
/// program they produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compiled {
    pub decisions: String,
    pub program: Vec<u8>,
}

/// A plan in memory: the store's file (its main database) and the plan.
type Key = (std::path::PathBuf, PlanId);

fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.compiled-plans-space/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.compiled-plans/v1\0{rel}").as_bytes()).into()
}

fn words(plan: &PlanId) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(plan[i * 8..i * 8 + 8].try_into().expect("8 bytes")))
}

fn part(plan: &PlanId) -> i64 {
    words(plan)[0] & i64::MAX
}

/// A name's hash, as a statistic or a backend is kept.
fn name(text: &str) -> i64 {
    i64::from_le_bytes(Sha256::digest(text.as_bytes())[..8].try_into().expect("8 bytes"))
}

/// The rows of `plan`'s partition in `rel`, of `plan` alone (its first four columns).
fn rows_of(kept: &KeptSpace, rel: &str, plan: &PlanId) -> Vec<Vec<i64>> {
    let chain = kept.view();
    let Some(entry) = chain.get(&FactKey { rel, ident: &ident(rel), part: part(plan) }) else { return Vec::new() };
    let w = words(plan);
    (0..entry.rows.len()).map(|i| entry.row(i)).filter(|r| r[..4] == w).collect()
}

/// `plan`'s partition of `rel`: `n` rows of `width` words, which `fill` gives row by row, written into a block of
/// their own as they are given (`BlockFlat`, never a heap vector) and digested there; gone when `n` is 0.
fn put(rel: &str, plan: &PlanId, width: usize, n: usize, fill: impl FnOnce(&mut dyn FnMut(i64))) -> KeptPut {
    let (what, digest) = if n == 0 {
        (KeptWhat::Gone, 0)
    } else {
        let mut block = BlockFlat::new(n, width, true);
        let mut h = Sha256::new();
        {
            let mut w = block.writer();
            fill(&mut |v| {
                h.update(v.to_le_bytes());
                w.push(v);
            });
            debug_assert_eq!(w.written(), n * width, "{rel}: whole rows");
        }
        (KeptWhat::Picked(Box::new(block)), u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes")))
    };
    KeptPut { rel: rel.to_owned(), ident: ident(rel), part: part(plan), digest, reads: Vec::new(), types: vec![b'i'; width], what }
}

/// `bytes` as words, eight bytes each, the last padded with zeros.
fn to_words(bytes: &[u8]) -> impl Iterator<Item = i64> + '_ {
    bytes.chunks(8).map(|c| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        i64::from_le_bytes(w)
    })
}

struct Held {
    compiled: Arc<Compiled>,
    bounds: Vec<(i64, u64, u64)>,
}

fn holds(bounds: &[(i64, u64, u64)], current: &BTreeMap<i64, u64>) -> bool {
    bounds.iter().all(|(stat, low, high)| current.get(stat).is_some_and(|value| (low..=high).contains(&value)))
}

/// `plan` as kept: its decisions and program, and its statistics' intervals; `None` when none is
/// kept, or one of another compiler.
fn read(kept: &KeptSpace, plan: &PlanId) -> Option<Held> {
    let head = rows_of(kept, PLAN, plan).pop()?;
    if head[4] != COMPILER_VERSION {
        return None;
    }
    let (decisions, program) = (usize::try_from(head[5]).ok()?, usize::try_from(head[6]).ok()?);
    let mut bytes: Vec<u8> = Vec::with_capacity(decisions + program + 8);
    let mut words = rows_of(kept, BYTES, plan);
    words.sort_by_key(|r| r[4]);
    for r in &words {
        bytes.extend_from_slice(&r[5].to_le_bytes());
    }
    if bytes.len() < decisions + program {
        return None;
    }
    let decisions_text = String::from_utf8(bytes[..decisions].to_vec()).ok()?;
    let program_bytes = bytes[decisions..decisions + program].to_vec();
    let bounds = rows_of(kept, READ, plan).into_iter().map(|r| (r[4], r[5] as u64, r[6] as u64)).collect();
    Some(Held { compiled: Arc::new(Compiled { decisions: decisions_text, program: program_bytes }), bounds })
}

/// Compiled plans, in memory and in the store.
pub struct Plans {
    memory: Cache<Key, Held>,
    /// The kept writes of plans `put` since the last `flush`.
    pending: Vec<KeptPut>,
}

impl Default for Plans {
    fn default() -> Self {
        Self { memory: Cache::new(MEMORY), pending: Vec::new() }
    }
}

impl Plans {
    /// The store's kept plans, opened for reads (`valid`) that may run where the store's connection is held.
    pub fn open(store: &ContentStore) -> StoreResult<Option<KeptSpace>> {
        store.kept_space(&space())
    }

    /// The plans among `plans` whose stored decisions still hold for `current` (statistic →
    /// value, as read now), with their programs: from memory, else as `kept` (`open`) holds them.
    pub fn valid(&mut self, store: &std::path::Path, kept: Option<&KeptSpace>, plans: &[PlanId], current: &BTreeMap<String, u64>) -> BTreeMap<PlanId, Arc<Compiled>> {
        let current: BTreeMap<i64, u64> = current.iter().map(|(k, v)| (name(k), *v)).collect();
        let mut valid = BTreeMap::new();
        for plan in plans {
            let key = (store.to_path_buf(), *plan);
            if let Some(entry) = self.memory.get(&key) {
                if holds(&entry.bounds, &current) {
                    valid.insert(*plan, Arc::clone(&entry.compiled));
                }
                continue;
            }
            let Some(entry) = kept.and_then(|kept| read(kept, plan)) else { continue };
            if holds(&entry.bounds, &current) {
                valid.insert(*plan, Arc::clone(&entry.compiled));
            }
            let bytes = entry.compiled.program.len() + entry.compiled.decisions.len();
            self.memory.insert(key, entry, bytes);
        }
        valid
    }

    /// Records `plan`, compiled from statistics `read` (statistic → value), replacing any earlier
    /// compilation of the store at `store`: in memory now, in the store once `flush`ed. Touches no
    /// connection, so it may run inside a candidate transaction.
    pub fn put(&mut self, store: &std::path::Path, plan: PlanId, compiled: Compiled, read: &BTreeMap<String, u64>) -> Arc<Compiled> {
        let w = words(&plan);
        let head = put(PLAN, &plan, 7, 1, |push| {
            w.iter().copied().chain([COMPILER_VERSION, compiled.decisions.len() as i64, compiled.program.len() as i64]).for_each(push)
        });
        let joined: Vec<u8> = compiled.decisions.as_bytes().iter().chain(&compiled.program).copied().collect();
        let bytes = put(BYTES, &plan, 6, joined.len().div_ceil(8), |push| {
            for (at, word) in to_words(&joined).enumerate() {
                w.iter().copied().chain([at as i64, word]).for_each(&mut *push);
            }
        });
        let bounds: Vec<(i64, u64, u64)> = read
            .iter()
            .map(|(stat, &value)| {
                let (low, high) = interval(value);
                (name(stat), low, high.min(i64::MAX as u64))
            })
            .collect();
        let reads = put(READ, &plan, 7, bounds.len(), |push| {
            for &(stat, low, high) in &bounds {
                w.iter().copied().chain([stat, low as i64, high as i64]).for_each(&mut *push);
            }
        });
        self.pending.extend([head, bytes, reads]);
        let size = compiled.program.len() + compiled.decisions.len();
        let compiled = Arc::new(compiled);
        self.memory.insert((store.to_path_buf(), plan), Held { compiled: Arc::clone(&compiled), bounds }, size);
        compiled
    }

    /// Queues the plans `put` since the last flush, behind the run: outside any candidate transaction.
    pub fn flush(&mut self, store: &ContentStore) {
        store.queue_kept(space(), std::mem::take(&mut self.pending));
    }
}

/// A plan's observed behaviour on one backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feedback {
    pub selectivity: f64,
    pub ns_per_root: f64,
    pub samples: i64,
}

/// Folds one run of `plan` on `backend` into its feedback, behind the run.
pub fn observe(store: &ContentStore, plan: PlanId, backend: &str, selectivity: f64, ns_per_root: f64) -> StoreResult<()> {
    let backend = name(backend);
    let mut rows = match store.kept_space(&space())? {
        Some(kept) => rows_of(&kept, FEEDBACK, &plan),
        None => Vec::new(),
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    match rows.iter_mut().find(|r| r[4] == backend) {
        Some(r) => {
            let (s, n) = (f64::from_bits(r[5] as u64), f64::from_bits(r[6] as u64));
            r[5] = (s + FEEDBACK_WEIGHT * (selectivity - s)).to_bits() as i64;
            r[6] = (n + FEEDBACK_WEIGHT * (ns_per_root - n)).to_bits() as i64;
            r[7] += 1;
            r[8] = now;
        }
        None => {
            let mut r = words(&plan).to_vec();
            r.extend([backend, selectivity.to_bits() as i64, ns_per_root.to_bits() as i64, 1, now]);
            rows.push(r);
        }
    }
    let feedback = put(FEEDBACK, &plan, 9, rows.len(), |push| rows.iter().flatten().copied().for_each(push));
    store.queue_kept(space(), vec![feedback]);
    Ok(())
}

/// `plan`'s feedback on `backend`, as kept.
pub fn feedback(store: &ContentStore, plan: PlanId, backend: &str) -> StoreResult<Option<Feedback>> {
    let Some(kept) = store.kept_space(&space())? else { return Ok(None) };
    let backend = name(backend);
    Ok(rows_of(&kept, FEEDBACK, &plan).into_iter().find(|r| r[4] == backend).map(|r| Feedback {
        selectivity: f64::from_bits(r[5] as u64),
        ns_per_root: f64::from_bits(r[6] as u64),
        samples: r[7],
    }))
}

/// What the store keeps of its compiled plans: each plan with its decisions and program, the
/// statistics their decisions read, and each feedback row's samples (a test's view).
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Default)]
pub struct KeptPlans {
    pub plans: Vec<(PlanId, Compiled)>,
    pub reads: usize,
    pub samples: Vec<i64>,
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn kept_plans(store: &ContentStore) -> KeptPlans {
    let mut out = KeptPlans::default();
    let Ok(Some(kept)) = Plans::open(store) else { return out };
    let chain = kept.view();
    for entry in chain.of(PLAN) {
        for i in 0..entry.rows.len() {
            let r = entry.row(i);
            let mut plan = [0u8; 32];
            for (c, w) in r.iter().take(4).enumerate() {
                plan[c * 8..c * 8 + 8].copy_from_slice(&w.to_le_bytes());
            }
            if let Some(held) = read(&kept, &plan) {
                out.reads += held.bounds.len();
                out.plans.push((plan, (*held.compiled).clone()));
            }
        }
    }
    for entry in chain.of(FEEDBACK) {
        out.samples.extend((0..entry.rows.len()).map(|i| entry.row(i)[7]));
    }
    out.plans.sort_by_key(|p| p.0);
    out
}

/// Writes `sqlite_stat1` rows for `indexes` ((table, index, columns)) from `fact_stats`, and makes
/// the planner read them. An index's k-th number is the rows an equality on its first k columns
/// selects: the smallest postings-weighted fanout (`sq / rows`) among them, at least 1.
pub fn write_stat1(db: &Connection, indexes: &[(&str, &str, &[&str])]) -> Result<()> {
    db.execute_batch("ANALYZE sqlite_schema")?;
    let mut column = db.prepare_cached("SELECT rows, sq FROM fact_stats WHERE tbl = ?1 AND col = ?2")?;
    for (table, index, columns) in indexes {
        let mut rows = 0;
        let mut stat = Vec::with_capacity(columns.len());
        let mut fanout = u64::MAX;
        for col in *columns {
            let (n, sq) = column
                .query_row(params![table, col], |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)))
                .optional()?
                .unwrap_or((0, 0));
            rows = rows.max(n);
            fanout = fanout.min(sq.div_ceil(n.max(1)).max(1));
            stat.push(fanout.to_string());
        }
        db.execute("DELETE FROM sqlite_stat1 WHERE tbl = ?1 AND idx = ?2", params![table, index])?;
        db.execute(
            "INSERT INTO sqlite_stat1(tbl, idx, stat) VALUES (?1, ?2, ?3)",
            params![table, index, format!("{} {}", rows.max(1), stat.join(" "))],
        )?;
    }
    // Reloads every table's statistics; reads no table but the schema.
    db.execute_batch("ANALYZE sqlite_schema")
}

// cratify: `tests` is rerail-engine's (it mounts it)
