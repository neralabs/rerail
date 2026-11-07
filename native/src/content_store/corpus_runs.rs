//! Corpus runs' roots and the bounded history the store keeps of them (decision 0051).
//!
//! Each run of a corpus root records a manifest: the sources it read, the grammar version and tape format of their
//! tapes, and its memo plan. Collection keeps
//!
//! 1. each root's last `protected_runs` manifests;
//! 2. older manifests, newest first, while the bytes they alone hold (the history) stay under a budget: no limit
//!    below the high-water mark, the low-water mark once the history passes it;
//!
//! and drops a manifest whose sources all appear in newer kept ones, and every manifest of a root idle past the
//! backstop. What the kept manifests name is `corpus_live`, which the collector's tape and source queries honour;
//! the memo rows of plans no kept manifest names are deleted in batches. The bound is by size, not by age: a branch
//! can be churned whole many times a day.

use super::{CollectionReport, ContentStore, MAX_COLLECTION_BATCH, SourceId, StoreError, StoreResult};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What collection keeps of corpus runs.
#[derive(Clone, Copy, Debug)]
pub struct CorpusPolicy {
    /// Each root's last runs, always kept.
    pub protected_runs: usize,
    /// Above this many bytes of history, a pass evicts.
    pub high_water: u64,
    /// Eviction keeps history up to this many bytes.
    pub low_water: u64,
    /// A root not run for this many seconds keeps nothing, its last runs included.
    pub backstop_secs: i64,
}

impl Default for CorpusPolicy {
    fn default() -> Self {
        CorpusPolicy { protected_runs: 2, high_water: 512 << 20, low_water: 384 << 20, backstop_secs: 30 * 86_400 }
    }
}

/// What one decision kept and dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpusKept {
    pub manifests: usize,
    pub evicted: usize,
    /// Bytes the protected manifests hold.
    pub protected_bytes: u64,
    /// Bytes the other kept manifests hold alone.
    pub history_bytes: u64,
    /// Entries of `corpus_live` removed.
    pub unlived: usize,
}

/// One pass: the decision (when one was due), memo rows of dead plans deleted, and the store's collection.
#[derive(Debug, Default)]
pub struct CorpusPass {
    pub kept: Option<CorpusKept>,
    pub plan_rows: usize,
    pub collection: CollectionReport,
}

impl CorpusPass {
    /// Whether the pass deleted anything (another may find more).
    pub fn did_work(&self) -> bool {
        let c = &self.collection;
        self.kept.as_ref().is_some_and(|k| k.evicted > 0 || k.unlived > 0) || self.plan_rows > 0 || c.tapes > 0 || c.sources > 0 || c.snapshots > 0 || c.snapshot_files > 0
    }
}

/// Memo rows of dead plans deleted per pass.
const PLAN_ROWS_PER_PASS: i64 = 4096;

struct Manifest {
    id: i64,
    root: i64,
    last_time: i64,
    grammar_version: i64,
    tape_format: i64,
    plan: Vec<u8>,
    sources: Vec<i64>,
}

/// A file's stat when its source was read: what the stat index keeps of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub size: i64,
    pub mtime_ns: i64,
    pub inode: i64,
}

impl FileStat {
    /// The stat of `metadata`.
    #[cfg(unix)]
    pub fn of(metadata: &std::fs::Metadata) -> FileStat {
        use std::os::unix::fs::MetadataExt as _;
        FileStat { size: metadata.len() as i64, mtime_ns: metadata.mtime().saturating_mul(1_000_000_000).saturating_add(metadata.mtime_nsec()), inode: metadata.ino() as i64 }
    }
}

/// How long before its recording an entry's mtime must be for the entry to be trusted (git's racy rule, with margin
/// for file systems whose mtime is coarse).
pub const RACY_NS: i64 = 2_000_000_000;

/// A root's stat index as loaded: by path, the stat recorded and its source, for entries a run may trust.
#[derive(Debug, Default)]
pub struct StatIndex {
    entries: HashMap<String, (FileStat, SourceId)>,
}

impl StatIndex {
    /// The source of the file at `path` when its stat is `now` and the index holds the same: else `None`, and the file
    /// is to be read.
    pub fn source(&self, path: &str, now: FileStat) -> Option<SourceId> {
        self.entries.get(path).filter(|(stat, _)| *stat == now).map(|(_, id)| *id)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ContentStore {
    /// The stat index of `root`: entries whose mtime is over [`RACY_NS`] before their recording and whose source is
    /// still stored.
    pub fn corpus_stats(&self, root: &str) -> StoreResult<StatIndex> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT st.path, st.size, st.mtime_ns, st.inode, st.source_id FROM corpus_stats st \
             JOIN corpus_roots r ON r.root_id=st.root_id JOIN source_objects s ON s.source_id=st.source_id \
             WHERE r.path=?1 AND st.mtime_ns < st.recorded_ns - ?2",
        )?;
        let rows = statement.query_map(params![root, RACY_NS], |r| {
            Ok((r.get::<_, String>(0)?, FileStat { size: r.get(1)?, mtime_ns: r.get(2)?, inode: r.get(3)? }, SourceId(r.get(4)?)))
        })?;
        let mut entries = HashMap::new();
        for row in rows {
            let (path, stat, id) = row?;
            entries.insert(path, (stat, id));
        }
        Ok(StatIndex { entries })
    }

    /// Records in `root`'s stat index each (path, stat, source) of `read`, the files a run read, at `now_ns`.
    pub fn record_corpus_stats(&self, root: &str, read: &[(String, FileStat, SourceId)], now_ns: i64) -> StoreResult<()> {
        if read.is_empty() {
            return Ok(());
        }
        self.immediate_transaction(|tx| {
            tx.execute("INSERT OR IGNORE INTO corpus_roots(path) VALUES (?1)", [root])?;
            let root_id: i64 = tx.query_row("SELECT root_id FROM corpus_roots WHERE path=?1", [root], |r| r.get(0))?;
            let mut put = tx.prepare_cached(
                "INSERT INTO corpus_stats(root_id, path, size, mtime_ns, inode, source_id, recorded_ns) VALUES (?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(root_id, path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns, inode=excluded.inode, \
                 source_id=excluded.source_id, recorded_ns=excluded.recorded_ns",
            )?;
            for (path, stat, id) in read {
                put.execute(params![root_id, path, stat.size, stat.mtime_ns, stat.inode, id.0, now_ns])?;
            }
            Ok(())
        })
    }

    /// Records a run of the corpus at `root` that read `sources` (tapes of `grammar_version` and `tape_format`) under
    /// memo plan `plan`, at unix second `now`. True if the manifest is new: a run matching an earlier manifest of its
    /// root only moves that one's last run.
    pub fn record_corpus_run(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], now: i64) -> StoreResult<bool> {
        let mut ids: Vec<i64> = sources.iter().map(|s| s.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let blob = encode_ids(&ids);
        let mut hash = Sha256::new();
        hash.update(b"rerail.corpus-manifest/v1\0");
        hash.update(grammar_version.to_le_bytes());
        hash.update(tape_format.to_le_bytes());
        hash.update(plan);
        hash.update(&blob);
        let digest: [u8; 32] = hash.finalize().into();
        self.immediate_transaction(|tx| {
            tx.execute("INSERT OR IGNORE INTO corpus_roots(path) VALUES (?1)", [root])?;
            let root_id: i64 = tx.query_row("SELECT root_id FROM corpus_roots WHERE path=?1", [root], |r| r.get(0))?;
            let run: i64 = tx.query_row("UPDATE corpus_state SET run=run+1 WHERE singleton=1 RETURNING run", [], |r| r.get(0))?;
            let moved = tx.execute("UPDATE corpus_manifests SET last_run=?1, last_time=?2 WHERE root_id=?3 AND digest=?4", params![run, now, root_id, &digest[..]])?;
            tx.execute("INSERT INTO corpus_plans(plan_digest, live) VALUES (?1, 1) ON CONFLICT(plan_digest) DO UPDATE SET live=1", [&plan[..]])?;
            if moved > 0 {
                return Ok(false);
            }
            // The sources the root's newest manifest of the same tapes already made live need no row again.
            let previous: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT sources FROM corpus_manifests WHERE root_id=?1 AND grammar_version=?2 AND tape_format=?3 ORDER BY last_run DESC LIMIT 1",
                    params![root_id, grammar_version, tape_format],
                    |r| r.get(0),
                )
                .optional()?;
            let before: HashSet<i64> = match previous {
                Some(b) => decode_ids(&b)?.into_iter().collect(),
                None => HashSet::new(),
            };
            tx.execute(
                "INSERT INTO corpus_manifests(root_id, last_run, last_time, grammar_version, tape_format, plan_digest, digest, sources) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![root_id, run, now, grammar_version, tape_format, &plan[..], &digest[..], blob],
            )?;
            let mut live = tx.prepare_cached("INSERT OR IGNORE INTO corpus_live(source_id, grammar_version, tape_format) VALUES (?1,?2,?3)")?;
            for id in ids.iter().filter(|id| !before.contains(id)) {
                live.execute(params![id, grammar_version, tape_format])?;
            }
            tx.execute("UPDATE corpus_state SET due=1 WHERE singleton=1", [])?;
            Ok(true)
        })
    }

    /// Whether corpus collection has work: a new manifest to decide on, or a decision's rows not yet collected.
    pub fn corpus_collection_due(&self) -> StoreResult<bool> {
        Ok(self.corpus_due()? != 0)
    }

    /// `corpus_state.due`: 0 nothing, 1 a new manifest (a decision is due), 2 a decision's rows being collected.
    fn corpus_due(&self) -> StoreResult<i64> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        Ok(connection.query_row("SELECT due FROM corpus_state WHERE singleton=1", [], |r| r.get(0))?)
    }

    /// Decides which manifests to keep under `policy` at unix second `now`, and applies it: evicted manifests go,
    /// `corpus_live` becomes what the kept ones name, and plans no kept manifest names are marked dead.
    pub fn keep_corpus_runs(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusKept> {
        let (manifests, sizes, live) = {
            let connection = self.connection.lock().expect("content store connection unavailable");
            let manifests = connection
                .prepare("SELECT manifest_id, root_id, last_time, grammar_version, tape_format, plan_digest, sources FROM corpus_manifests ORDER BY last_run DESC")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get::<_, Vec<u8>>(5)?, r.get::<_, Vec<u8>>(6)?)))?
                .collect::<Result<Vec<(i64, i64, i64, i64, i64, Vec<u8>, Vec<u8>)>, _>>()?
                .into_iter()
                .map(|(id, root, last_time, grammar_version, tape_format, plan, blob)| {
                    Ok(Manifest { id, root, last_time, grammar_version, tape_format, plan, sources: decode_ids(&blob)? })
                })
                .collect::<StoreResult<Vec<_>>>()?;
            // Bytes of each live source, and of its tape of the live grammar version and format.
            let mut sizes = Sizes::default();
            let mut live = HashSet::new();
            let mut rows = connection.prepare(
                "SELECT l.source_id, l.grammar_version, l.tape_format, coalesce(length(s.frame), 0), \
                 coalesce((SELECT length(t.frame) FROM tape_objects t WHERE t.source_id=l.source_id \
                           AND t.grammar_version=l.grammar_version AND t.tape_format_version=l.tape_format), 0) \
                 FROM corpus_live l LEFT JOIN source_objects s ON s.source_id=l.source_id",
            )?;
            let mut q = rows.query([])?;
            while let Some(r) = q.next()? {
                let key: (i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
                sizes.source.insert(key.0, r.get::<_, i64>(3)? as u64);
                sizes.tape.insert(key, r.get::<_, i64>(4)? as u64);
                live.insert(key);
            }
            (manifests, sizes, live)
        };
        let decided = decide(&manifests, &sizes, policy, now);
        let kept_ids: HashSet<i64> = decided.kept.iter().copied().collect();
        let mut keep_live: HashSet<(i64, i64, i64)> = HashSet::new();
        let mut keep_plans: BTreeSet<Vec<u8>> = BTreeSet::new();
        for m in manifests.iter().filter(|m| kept_ids.contains(&m.id)) {
            keep_live.extend(m.sources.iter().map(|&s| (s, m.grammar_version, m.tape_format)));
            keep_plans.insert(m.plan.clone());
        }
        let unlive: Vec<&(i64, i64, i64)> = live.iter().filter(|k| !keep_live.contains(k)).collect();
        let evicted: Vec<i64> = manifests.iter().map(|m| m.id).filter(|id| !kept_ids.contains(id)).collect();
        self.immediate_transaction(|tx| {
            let mut gone = tx.prepare_cached("DELETE FROM corpus_manifests WHERE manifest_id=?1")?;
            for id in &evicted {
                gone.execute([id])?;
            }
            let mut unlived = tx.prepare_cached("DELETE FROM corpus_live WHERE source_id=?1 AND grammar_version=?2 AND tape_format=?3")?;
            for (s, g, f) in &unlive {
                unlived.execute(params![s, g, f])?;
            }
            // A plan a kept manifest names is live; any other recorded plan is dead.
            let plans: Vec<Vec<u8>> = tx.prepare("SELECT plan_digest FROM corpus_plans")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            let mut mark = tx.prepare_cached("UPDATE corpus_plans SET live=?1 WHERE plan_digest=?2")?;
            for plan in plans {
                mark.execute(params![i64::from(keep_plans.contains(&plan)), plan])?;
            }
            // Collection stays due until a pass finds nothing (`collect_corpus_pass`), unless a new manifest came first.
            tx.execute("UPDATE corpus_state SET due=2, history_bytes=?1 WHERE singleton=1 AND due<=1", [decided.history as i64])?;
            Ok(())
        })?;
        Ok(CorpusKept { manifests: decided.kept.len(), evicted: evicted.len(), protected_bytes: decided.protected, history_bytes: decided.history, unlived: unlive.len() })
    }

    /// One bounded pass: the keep decision when one is due, up to `PLAN_ROWS_PER_PASS` memo rows of dead plans, and
    /// the store's collection (`collect`).
    pub fn collect_corpus_pass(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusPass> {
        let mut pass = CorpusPass::default();
        if self.corpus_due()? == 1 {
            pass.kept = Some(self.keep_corpus_runs(policy, now)?);
        }
        pass.plan_rows = self.immediate_transaction(|tx| {
            let dead: Option<Vec<u8>> = tx.query_row("SELECT plan_digest FROM corpus_plans WHERE live=0 LIMIT 1", [], |r| r.get(0)).optional()?;
            let Some(plan) = dead else { return Ok(0) };
            let n = tx.execute(
                "DELETE FROM facts_results WHERE (source_id, fact_schema, path, plan_digest) IN (\
                 SELECT source_id, fact_schema, path, plan_digest FROM facts_results WHERE plan_digest=?1 LIMIT ?2)",
                params![plan, PLAN_ROWS_PER_PASS],
            )?;
            if (n as i64) < PLAN_ROWS_PER_PASS {
                tx.execute("DELETE FROM corpus_plans WHERE plan_digest=?1 AND live=0", [&plan])?;
            }
            Ok(n)
        })?;
        pass.collection = self.collect(now, MAX_COLLECTION_BATCH)?;
        if !pass.did_work() {
            self.immediate_transaction(|tx| Ok(tx.execute("UPDATE corpus_state SET due=0 WHERE singleton=1 AND due=2", [])?))?;
        }
        Ok(pass)
    }

    /// Converts a store made before decision 0051 (`auto_vacuum` never took) to incremental vacuum: a full `VACUUM`,
    /// which rewrites the file. True if it converted. For maintenance that holds the store alone (`rerail-engine gc`).
    pub fn convert_to_incremental_vacuum(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        let mode: i64 = connection.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        if mode == 2 {
            return Ok(false);
        }
        connection.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
        Ok(true)
    }
}

#[derive(Default)]
struct Sizes {
    source: HashMap<i64, u64>,
    tape: HashMap<(i64, i64, i64), u64>,
}

struct Decided {
    kept: Vec<i64>,
    protected: u64,
    history: u64,
}

/// The manifests to keep, `manifests` newest first (the module's rules).
fn decide(manifests: &[Manifest], sizes: &Sizes, policy: &CorpusPolicy, now: i64) -> Decided {
    let walk = |cap: Option<u64>| -> Decided {
        let mut per_root: BTreeMap<i64, usize> = BTreeMap::new();
        let (mut seen_sources, mut seen_tapes) = (HashSet::new(), HashSet::new());
        let mut out = Decided { kept: Vec::new(), protected: 0, history: 0 };
        for m in manifests {
            let rank = per_root.entry(m.root).or_default();
            let protected = *rank < policy.protected_runs;
            *rank += 1;
            if now - m.last_time > policy.backstop_secs {
                continue;
            }
            // The bytes this manifest holds that no newer kept manifest does.
            let mut own = 0;
            let mut fresh = Vec::new();
            for &s in &m.sources {
                let tape = (s, m.grammar_version, m.tape_format);
                if !seen_tapes.contains(&tape) {
                    own += sizes.tape.get(&tape).copied().unwrap_or(0);
                    fresh.push(tape);
                }
                if !seen_sources.contains(&s) {
                    own += sizes.source.get(&s).copied().unwrap_or(0);
                }
            }
            let keep = if protected {
                out.protected += own;
                true
            } else if fresh.is_empty() {
                false
            } else if cap.is_none_or(|cap| out.history + own <= cap) {
                out.history += own;
                true
            } else {
                false
            };
            if keep {
                out.kept.push(m.id);
                seen_sources.extend(m.sources.iter().copied());
                seen_tapes.extend(fresh);
            }
        }
        out
    };
    let all = walk(None);
    if all.history > policy.high_water { walk(Some(policy.low_water)) } else { all }
}

/// Sorted ids as delta varints.
fn encode_ids(ids: &[i64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ids.len() * 2);
    let mut last = 0i64;
    for &id in ids {
        let mut d = (id - last) as u64;
        last = id;
        loop {
            let byte = (d & 0x7f) as u8;
            d >>= 7;
            if d == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    out
}

fn decode_ids(bytes: &[u8]) -> StoreResult<Vec<i64>> {
    let mut out = Vec::new();
    let (mut last, mut d, mut shift) = (0i64, 0u64, 0u32);
    for &b in bytes {
        if shift > 63 {
            return Err(StoreError::Corrupt("a corpus manifest's source ids overflow".into()));
        }
        d |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            last += d as i64;
            out.push(last);
            (d, shift) = (0, 0);
        } else {
            shift += 7;
        }
    }
    if shift != 0 {
        return Err(StoreError::Corrupt("a corpus manifest's source ids are truncated".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
