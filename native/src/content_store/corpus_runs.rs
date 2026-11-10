//! Corpus runs' roots and the bounded history the store keeps of them (decision 0053).
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

use super::{CollectionReport, ContentStore, MAX_COLLECTION_BATCH, STRING_OWNER, SourceId, StoreError, StoreResult};
use super::rows::intern_string;
use super::bounded_insert;

/// `object_pins`' kind for a memo plan a run holds (O95): its id is the plan digest's first eight bytes.
const OBJECT_PLAN: i64 = 3;

fn plan_object(plan: &[u8]) -> i64 {
    let mut first = [0u8; 8];
    first.copy_from_slice(&plan[..8]);
    i64::from_le_bytes(first)
}
use rusqlite::{OptionalExtension, params, types::Value};
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
    /// A root whose directory is gone and that has not run for this many seconds keeps nothing: no run can be warm
    /// there again (a test's or a replay's temporary roots).
    pub missing_secs: i64,
    /// With no root past its protected runs, a decision waits for this many runs since the last one…
    pub decide_every_runs: i64,
    /// …or this many seconds (for the backstop and gone roots).
    pub decide_every_secs: i64,
}

impl Default for CorpusPolicy {
    fn default() -> Self {
        CorpusPolicy {
            protected_runs: 2,
            high_water: 512 << 20,
            low_water: 384 << 20,
            backstop_secs: 30 * 86_400,
            missing_secs: 3_600,
            decide_every_runs: 256,
            decide_every_secs: 600,
        }
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

/// The corpus memo's schemas before 4 (logic/plan/corpus/memo.rs): their rows were written before decision 0053, under
/// plans no manifest names, so only a sweep collects them. Theirs alone: the slice's memo is schema 1 and a prepared
/// program's entries 101 to 103, all under other numbers, and a content-pure row (empty path) is never the corpus's.
pub const RETIRED_CORPUS_MEMO_SCHEMAS: &[u32] = &[2, 3];

fn lapsed_plan_hold(connection: &rusqlite::Connection, now: i64) -> StoreResult<bool> {
    Ok(connection
        .query_row("SELECT 1 FROM object_pins WHERE object_kind_id=?1 AND lease_until<?2 LIMIT 1", params![OBJECT_PLAN, now], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some())
}

/// Memo rows of dead plans deleted per pass.
const PLAN_ROWS_PER_PASS: i64 = 4096;

#[derive(Clone)]
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

    /// The stat of `metadata` where there are no inode numbers (Windows, wasm32): size and modification time.
    #[cfg(not(unix))]
    pub fn of(metadata: &std::fs::Metadata) -> FileStat {
        let mtime_ns = metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
        FileStat { size: metadata.len() as i64, mtime_ns, inode: 0 }
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
            // A few statements whatever the files' number.
            bounded_insert::insert(
                tx,
                "INSERT INTO corpus_stats(root_id, path, size, mtime_ns, inode, source_id, recorded_ns)",
                "ON CONFLICT(root_id, path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns, inode=excluded.inode, \
                 source_id=excluded.source_id, recorded_ns=excluded.recorded_ns",
                read.iter().map(|(path, stat, id)| {
                    [Value::Integer(root_id), Value::Text(path.clone()), Value::Integer(stat.size), Value::Integer(stat.mtime_ns), Value::Integer(stat.inode), Value::Integer(id.0), Value::Integer(now_ns)]
                }),
            )?;
            Ok(())
        })
    }

    /// Keeps memo plan `plan`'s statistics (decision 0096 §2): one blob, written by the run that counted them.
    pub fn put_stat_blob(&self, plan: [u8; 32], blob: &[u8]) -> StoreResult<()> {
        self.immediate_transaction(|tx| {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS stat_blob(plan BLOB PRIMARY KEY, blob BLOB NOT NULL) WITHOUT ROWID")?;
            tx.execute("INSERT OR REPLACE INTO stat_blob(plan, blob) VALUES (?1, ?2)", params![&plan[..], blob])?;
            Ok(())
        })
    }

    /// `put_stat_blob`, queued behind the run (decision 0102).
    pub fn queue_stat_blob(&self, plan: [u8; 32], blob: Vec<u8>) {
        self.write_back("the kept statistics", blob.len(), vec![(stat_blob_key(&plan), None)], Box::new(move |store| store.put_stat_blob(plan, &blob)));
    }

    /// Memo plan `plan`'s kept statistics (`put_stat_blob`), if a run kept them; a queued write of them first.
    pub fn stat_blob(&self, plan: [u8; 32]) -> StoreResult<Option<Vec<u8>>> {
        self.settle(&stat_blob_key(&plan));
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let kept: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='stat_blob'", [], |r| r.get(0))?;
        if kept == 0 {
            return Ok(None);
        }
        Ok(connection.query_row("SELECT blob FROM stat_blob WHERE plan=?1", [&plan[..]], |r| r.get(0)).optional()?)
    }

    /// Records the relations a memo plan's entries hold rows of, each with its declared columns (JSON).
    pub fn record_corpus_relations(&self, plan: [u8; 32], relations: &[(String, String)]) -> StoreResult<()> {
        self.immediate_transaction(|tx| {
            bounded_insert::insert(
                tx,
                "INSERT OR REPLACE INTO corpus_plan_relations(plan_digest, relation, columns)",
                "",
                relations.iter().map(|(relation, columns)| [Value::Blob(plan.to_vec()), Value::Text(relation.clone()), Value::Text(columns.clone())]),
            )?;
            Ok(())
        })
    }

    /// The newest run of `root` (as recorded, canonical): its memo plan, the sources it read, and the relations
    /// its plan's entries hold rows of with their declared columns (JSON). `None` if the root has no run.
    pub fn corpus_run_of(&self, root: &str) -> StoreResult<Option<([u8; 32], Vec<i64>, Vec<(String, String)>)>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let run: Option<(Vec<u8>, Vec<u8>)> = connection
            .query_row(
                "SELECT m.plan_digest, m.sources FROM corpus_manifests m JOIN corpus_roots r USING (root_id) \
                 WHERE r.path = ?1 ORDER BY m.last_run DESC LIMIT 1",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((plan, sources)) = run else { return Ok(None) };
        let plan: [u8; 32] = plan.try_into().map_err(|_| StoreError::Corrupt("a corpus manifest's plan is not 32 bytes".into()))?;
        let relations = connection
            .prepare_cached("SELECT relation, columns FROM corpus_plan_relations WHERE plan_digest = ?1 ORDER BY relation")?
            .query_map([&plan[..]], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((plan, decode_ids(&sources)?, relations)))
    }

    /// Records a run of the corpus at `root` that read `sources` (tapes of `grammar_version` and `tape_format`) under
    /// memo plan `plan`, at unix second `now`. True if the manifest is new: a run matching an earlier manifest of its
    /// root only moves that one's last run.
    pub fn record_corpus_run(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], now: i64) -> StoreResult<bool> {
        self.record_corpus_run_naming(root, sources, grammar_version, tape_format, plan, &[], now)
    }

    /// `record_corpus_run`, the manifest also naming `also`: other memo plans the run stored entries under (its
    /// prepared program's, decision 0073). They are live while a kept manifest names them, as its memo plan is, and
    /// their rows are collected once none does.
    #[allow(clippy::too_many_arguments)]
    pub fn record_corpus_run_naming(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], now: i64) -> StoreResult<bool> {
        let mut also: Vec<[u8; 32]> = also.iter().copied().filter(|p| *p != plan).collect();
        also.sort_unstable();
        also.dedup();
        let mut ids: Vec<i64> = sources.iter().map(|s| s.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let blob = encode_ids(&ids);
        let mut hash = Sha256::new();
        hash.update(b"rerail.corpus-manifest/v1\0");
        hash.update(grammar_version.to_le_bytes());
        hash.update(tape_format.to_le_bytes());
        hash.update(plan);
        // A manifest naming no other plan keeps the digest it had before `also` existed.
        for p in &also {
            hash.update(p);
        }
        hash.update(&blob);
        let digest: [u8; 32] = hash.finalize().into();
        self.immediate_transaction(|tx| {
            tx.execute("INSERT OR IGNORE INTO corpus_roots(path) VALUES (?1)", [root])?;
            let root_id: i64 = tx.query_row("SELECT root_id FROM corpus_roots WHERE path=?1", [root], |r| r.get(0))?;
            let run: i64 = tx.query_row("UPDATE corpus_state SET run=run+1 WHERE singleton=1 RETURNING run", [], |r| r.get(0))?;
            let moved = tx.execute("UPDATE corpus_manifests SET last_run=?1, last_time=?2 WHERE root_id=?3 AND digest=?4", params![run, now, root_id, &digest[..]])?;
            bounded_insert::insert(
                tx,
                "INSERT INTO corpus_plans(plan_digest, live)",
                "ON CONFLICT(plan_digest) DO UPDATE SET live=1",
                std::iter::once(&plan).chain(&also).map(|p| [Value::Blob(p.to_vec()), Value::Integer(1)]),
            )?;
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
            let manifest = tx.last_insert_rowid();
            bounded_insert::insert(
                tx,
                "INSERT OR IGNORE INTO corpus_manifest_plans(manifest_id, plan_digest)",
                "",
                also.iter().map(|p| [Value::Integer(manifest), Value::Blob(p.to_vec())]),
            )?;
            // The sources new to the root's manifests: a few statements whatever their number.
            bounded_insert::insert(
                tx,
                "INSERT OR IGNORE INTO corpus_live(source_id, grammar_version, tape_format)",
                "",
                ids.iter().filter(|id| !before.contains(id)).map(|id| [Value::Integer(*id), Value::Integer(grammar_version), Value::Integer(tape_format)]),
            )?;
            tx.execute("UPDATE corpus_state SET due=1 WHERE singleton=1", [])?;
            Ok(true)
        })
    }

    /// Holds memo plan `plan` for `owner` until unix second `lease_until`, and makes it live, in one transaction
    /// (O95). A run writes its memo rows before it records its manifest, and a plan an earlier, evicted manifest
    /// named may be dead, its rows being deleted. Held, it stays live and keeps its rows until the run's manifest
    /// names it. `release_pins(owner)` ends the hold; a run that dies leaves it to lapse with its lease.
    pub fn hold_corpus_plan(&self, plan: [u8; 32], owner: &str, lease_until: i64) -> StoreResult<()> {
        if owner.is_empty() || lease_until <= 0 {
            return Err(StoreError::Invalid("object pin is invalid".to_owned()));
        }
        self.immediate_transaction(|tx| {
            let owner_id = intern_string(tx, STRING_OWNER, owner)?;
            tx.execute(
                "INSERT INTO object_pins(object_kind_id,object_id,owner_string_id,lease_until) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(object_kind_id,object_id,owner_string_id) DO UPDATE SET lease_until=excluded.lease_until",
                params![OBJECT_PLAN, plan_object(&plan), owner_id, lease_until],
            )?;
            tx.execute("INSERT INTO corpus_plans(plan_digest, live) VALUES (?1, 1) ON CONFLICT(plan_digest) DO UPDATE SET live=1", [&plan[..]])?;
            Ok(())
        })
    }

    /// Whether corpus collection has work: a new manifest to decide on, or a decision's rows not yet collected.
    pub fn corpus_collection_due(&self) -> StoreResult<bool> {
        Ok(self.corpus_due()? != 0)
    }

    /// Whether a pass under `policy` at unix second `now` has work: a decision's rows being collected, or a decision
    /// due that can change something (`corpus_decision_worth`). A due decision that cannot is cleared here, so a run
    /// of fresh roots neither decides nor sweeps the store after each run.
    pub fn corpus_pass_wanted(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<bool> {
        if self.retired_memo_rows_left()? {
            return Ok(true);
        }
        let due = match self.corpus_due()? {
            0 if self.lapsed_plan_hold(now)? => 1,
            due => due,
        };
        match due {
            0 => Ok(false),
            1 if !self.corpus_decision_worth(policy, now)? => {
                self.immediate_transaction(|tx| Ok(tx.execute("UPDATE corpus_state SET due=0 WHERE singleton=1 AND due=1", [])?))?;
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    /// Whether a keep decision can change anything now: a root recorded since the last decision has more manifests
    /// than its protected runs (history grows only so), or `decide_every_runs` runs or `decide_every_secs` seconds
    /// have passed since it (the backstop and gone roots). Reads the recent manifests through `last_run`'s index.
    fn corpus_decision_worth(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        let (run, decided_run, decided_time): (i64, i64, i64) = connection.query_row(
            "SELECT s.run, coalesce(d.run, 0), coalesce(d.time, 0) FROM corpus_state s LEFT JOIN corpus_decided d ON d.singleton=1 WHERE s.singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if run - decided_run >= policy.decide_every_runs || now - decided_time >= policy.decide_every_secs {
            return Ok(true);
        }
        // A plan hold that lapsed: the plan may be dead now (a memo whose writer holds its plans, the slice's).
        if lapsed_plan_hold(&connection, now)? {
            return Ok(true);
        }
        let displaced: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM corpus_manifests m WHERE m.last_run > ?1 \
                 AND (SELECT count(*) FROM corpus_manifests o WHERE o.root_id=m.root_id) > ?2 LIMIT 1",
                params![decided_run, policy.protected_runs as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(displaced.is_some())
    }

    /// Whether a plan hold has lapsed (`object_pins` of `OBJECT_PLAN` past their lease).
    fn lapsed_plan_hold(&self, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        lapsed_plan_hold(&connection, now)
    }

    /// Whether the retired corpus memo schemas' rows are still being swept.
    fn retired_memo_rows_left(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        Ok(connection.query_row("SELECT 1 FROM corpus_swept WHERE singleton=1 AND done=0", [], |r| r.get::<_, i64>(0)).optional()?.is_some())
    }

    /// `corpus_state.due`: 0 nothing, 1 a new manifest (a decision is due), 2 a decision's rows being collected.
    fn corpus_due(&self) -> StoreResult<i64> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        Ok(connection.query_row("SELECT due FROM corpus_state WHERE singleton=1", [], |r| r.get(0))?)
    }

    /// Decides which manifests to keep under `policy` at unix second `now`, and applies it: evicted manifests go,
    /// `corpus_live` becomes what the kept ones name, and plans no kept manifest names are marked dead.
    /// Defers with an empty report while a reader or another collector holds the collection lease.
    pub fn keep_corpus_runs(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusKept> {
        let Some(_lease) = self.try_collect()? else { return Ok(CorpusKept::default()) };
        self.keep_corpus_runs_held(policy, now)
    }

    fn keep_corpus_runs_held(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusKept> {
        // Read, decided and applied in one transaction (O95): a manifest or plan another run records meanwhile waits
        // for the write lock, so a decision never drops what it did not see.
        self.immediate_transaction(|tx| {
            let manifests = tx
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
            {
                let mut rows = tx.prepare(
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
            }
            // A root whose directory is gone keeps nothing once it has been idle `missing_secs`: its manifests are
            // left out of the decision (so evicted), and its stat index and row go with them.
            let gone_roots: HashSet<i64> = {
                let mut last: HashMap<i64, i64> = HashMap::new();
                for m in &manifests {
                    let t = last.entry(m.root).or_insert(m.last_time);
                    *t = (*t).max(m.last_time);
                }
                let roots: Vec<(i64, String)> = tx.prepare("SELECT root_id, path FROM corpus_roots")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                roots
                    .into_iter()
                    .filter(|(id, path)| last.get(id).is_none_or(|&t| now - t >= policy.missing_secs) && !std::path::Path::new(path).exists())
                    .map(|(id, _)| id)
                    .collect()
            };
            let present: Vec<Manifest>;
            let considered: &[Manifest] = if gone_roots.is_empty() {
                &manifests
            } else {
                present = manifests.iter().filter(|m| !gone_roots.contains(&m.root)).map(Manifest::clone).collect();
                &present
            };
            let decided = decide(considered, &sizes, policy, now);
            let kept_ids: HashSet<i64> = decided.kept.iter().copied().collect();
            let mut keep_live: HashSet<(i64, i64, i64)> = HashSet::new();
            let mut keep_plans: BTreeSet<Vec<u8>> = BTreeSet::new();
            for m in manifests.iter().filter(|m| kept_ids.contains(&m.id)) {
                keep_live.extend(m.sources.iter().map(|&s| (s, m.grammar_version, m.tape_format)));
                keep_plans.insert(m.plan.clone());
            }
            // The other plans kept manifests name (`record_corpus_run_naming`).
            {
                let mut named = tx.prepare_cached("SELECT plan_digest FROM corpus_manifest_plans WHERE manifest_id=?1")?;
                for m in manifests.iter().filter(|m| kept_ids.contains(&m.id)) {
                    for p in named.query_map([m.id], |r| r.get::<_, Vec<u8>>(0))? {
                        keep_plans.insert(p?);
                    }
                }
            }
            let unlive: Vec<&(i64, i64, i64)> = live.iter().filter(|k| !keep_live.contains(k)).collect();
            let evicted: Vec<i64> = manifests.iter().map(|m| m.id).filter(|id| !kept_ids.contains(id)).collect();
            // Each set in one statement, whatever its size.
            let evicted_json = serde_json::to_string(&evicted).expect("integers");
            tx.prepare_cached("DELETE FROM corpus_manifest_plans WHERE manifest_id IN (SELECT value FROM json_each(?1))")?.execute([&evicted_json])?;
            tx.prepare_cached("DELETE FROM corpus_manifests WHERE manifest_id IN (SELECT value FROM json_each(?1))")?.execute([&evicted_json])?;
            tx.prepare_cached(
                "DELETE FROM corpus_live WHERE (source_id, grammar_version, tape_format) IN \
                 (SELECT json_extract(value,'$[0]'), json_extract(value,'$[1]'), json_extract(value,'$[2]') FROM json_each(?1))",
            )?
            .execute([serde_json::to_string(&unlive.iter().map(|(s, g, f)| [*s, *g, *f]).collect::<Vec<_>>()).expect("integers")])?;
            // A plan a kept manifest names, or a run holds (`hold_corpus_plan`), is live; any other recorded plan is dead.
            let held: HashSet<i64> = tx
                .prepare("SELECT object_id FROM object_pins WHERE object_kind_id=?1 AND lease_until>=?2")?
                .query_map(params![OBJECT_PLAN, now], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let plans: Vec<Vec<u8>> = tx.prepare("SELECT plan_digest FROM corpus_plans")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            // Every plan's liveness in one statement: the live ones listed (hex), the rest dead.
            let live_plans: Vec<String> = plans.iter().filter(|plan| keep_plans.contains(*plan) || (plan.len() >= 8 && held.contains(&plan_object(plan)))).map(|p| hex(p)).collect();
            tx.prepare_cached("UPDATE corpus_plans SET live = (plan_digest IN (SELECT unhex(value) FROM json_each(?1)))")?
                .execute([serde_json::to_string(&live_plans).expect("text")])?;
            let gone_json = serde_json::to_string(&gone_roots).expect("integers");
            tx.prepare_cached("DELETE FROM corpus_stats WHERE root_id IN (SELECT value FROM json_each(?1))")?.execute([&gone_json])?;
            tx.prepare_cached("DELETE FROM corpus_roots WHERE root_id IN (SELECT value FROM json_each(?1))")?.execute([&gone_json])?;
            tx.execute("UPDATE corpus_decided SET run=(SELECT run FROM corpus_state WHERE singleton=1), time=?1 WHERE singleton=1", [now])?;
            // Collection stays due until a pass finds nothing (`collect_corpus_pass`), unless a new manifest came first.
            tx.execute("UPDATE corpus_state SET due=2, history_bytes=?1 WHERE singleton=1 AND due<=1", [decided.history as i64])?;
            Ok(CorpusKept { manifests: decided.kept.len(), evicted: evicted.len(), protected_bytes: decided.protected, history_bytes: decided.history, unlived: unlive.len() })
        })
    }

    /// One bounded pass: the keep decision when one is due, up to `PLAN_ROWS_PER_PASS` memo rows of dead plans, and
    /// the store's collection (`collect`).
    /// Active readers defer the pass without clearing the pending collection flag.
    pub fn collect_corpus_pass(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusPass> {
        let Some(_lease) = self.try_collect()? else { return Ok(CorpusPass::default()) };
        let mut pass = CorpusPass::default();
        // Rows of the corpus memo's retired schemas (before decision 0053 named plans): no plan collects them.
        let swept = self.immediate_transaction(|tx| {
            let left: Option<i64> = tx.query_row("SELECT 1 FROM corpus_swept WHERE singleton=1 AND done=0", [], |r| r.get(0)).optional()?;
            if left.is_none() {
                return Ok(0);
            }
            let schemas = RETIRED_CORPUS_MEMO_SCHEMAS.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            let n = tx.execute(
                &format!(
                    "DELETE FROM facts_results WHERE (source_id, fact_schema, path, plan_digest) IN (\
                     SELECT source_id, fact_schema, path, plan_digest FROM facts_results WHERE fact_schema IN ({schemas}) AND path<>'' LIMIT ?1)"
                ),
                [PLAN_ROWS_PER_PASS],
            )?;
            if (n as i64) < PLAN_ROWS_PER_PASS {
                tx.execute("UPDATE corpus_swept SET done=1 WHERE singleton=1", [])?;
            }
            Ok(n)
        })?;
        pass.plan_rows = swept;
        let due = match self.corpus_due()? {
            0 if self.lapsed_plan_hold(now)? => 1,
            due => due,
        };
        if due == 1 {
            if !self.corpus_decision_worth(policy, now)? {
                // Nothing a decision could change: no root went past its protected runs, and the last decision is
                // recent. The sweep waits for the next decision too; a new manifest makes collection due again.
                self.immediate_transaction(|tx| Ok(tx.execute("UPDATE corpus_state SET due=0 WHERE singleton=1 AND due=1", [])?))?;
                if swept > 0 {
                    pass.collection = self.collect_held(now, MAX_COLLECTION_BATCH)?;
                }
                return Ok(pass);
            }
            pass.kept = Some(self.keep_corpus_runs_held(policy, now)?);
        }
        pass.plan_rows += self.immediate_transaction(|tx| {
            let dead: Option<Vec<u8>> = tx.query_row("SELECT plan_digest FROM corpus_plans WHERE live=0 LIMIT 1", [], |r| r.get(0)).optional()?;
            let Some(plan) = dead else { return Ok(0) };
            let n = tx.execute(
                "DELETE FROM facts_results WHERE (source_id, fact_schema, path, plan_digest) IN (\
                 SELECT source_id, fact_schema, path, plan_digest FROM facts_results WHERE plan_digest=?1 LIMIT ?2)",
                params![plan, PLAN_ROWS_PER_PASS],
            )?;
            if (n as i64) < PLAN_ROWS_PER_PASS {
                tx.execute("DELETE FROM corpus_plans WHERE plan_digest=?1 AND live=0", [&plan])?;
                tx.execute("DELETE FROM corpus_plan_relations WHERE plan_digest=?1", [&plan])?;
            }
            Ok(n)
        })?;
        pass.collection = self.collect_held(now, MAX_COLLECTION_BATCH)?;
        if !pass.did_work() {
            self.immediate_transaction(|tx| Ok(tx.execute("UPDATE corpus_state SET due=0 WHERE singleton=1 AND due=2", [])?))?;
        }
        Ok(pass)
    }

    /// Converts a store made before decision 0053 (`auto_vacuum` never took) to incremental vacuum: a full `VACUUM`,
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

/// `bytes` as lower-case hex, as SQLite's `unhex` reads it.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a write of plan `plan`'s statistics is queued under (`write_back`).
fn stat_blob_key(plan: &[u8; 32]) -> super::write_back::Key {
    super::write_back::Key::of(&[b"stat_blob", plan])
}
