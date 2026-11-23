//! Capacity limits, object pins, and collection of unreachable rows.

use super::connection::{BusyRetries, retry_busy};
use super::rows::{generation, intern_string};
use super::snapshots::GIT_TREE_BATCH_ENTRIES;
use super::{
    CapacityState, CollectionReport, ContentStore, HashKey, MAX_COLLECTION_BATCH, STRING_OWNER,
    SourceId, StoreCapacity, StoreError, StoreResult, TapeId,
};
#[allow(unused_imports)]
use crate::content_store::Cached as _;
use rusqlite::{Connection, OptionalExtension, Params, TransactionBehavior, params};
use std::fs;

pub(super) const OBJECT_TAPE: i64 = 1;
/// A store session's kind (`store_sessions.kind`, decision 0450 §2): a workspace, keyed by its root's path.
pub(super) const SESSION_WORKSPACE: i64 = 1;
/// A corpus lane, keyed by a digest of its root and lane (`corpus_runs::lane_session`).
pub(super) const SESSION_LANE: i64 = 2;
/// A pin's owner, keyed by its name's `strings` id: its source and tape pins are its data.
pub(super) const SESSION_PINS: i64 = 3;

/// The ended sessions of the kinds collection ages (workspaces and pin owners) with the aged bytes of those ended
/// since, newest first, them included: one budget for both (decision 0450 step 3).
macro_rules! aged_sessions {
    () => {
        "(SELECT kind, key, ended, sum(aged_bytes) OVER (ORDER BY ended DESC, kind, key ROWS UNBOUNDED PRECEDING) held \
         FROM store_sessions WHERE kind IN (1,3) AND ended IS NOT NULL)"
    };
}

/// How long an ended store session's data is kept (decision 0450 step 3; 0053 §3's marks and backstop until the user
/// sets others): the sessions ended last keep theirs while together they hold at most `bytes`, and none past
/// `backstop_secs` after it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ageing {
    pub bytes: u64,
    pub backstop_secs: i64,
}

impl Default for Ageing {
    fn default() -> Self {
        Ageing { bytes: 384 << 20, backstop_secs: 30 * 86_400 }
    }
}

/// A store handle's `Ageing`, which a test may change (`set_session_ageing`).
#[derive(Default)]
pub(super) struct SessionAgeing(parking_lot::Mutex<Ageing>);

impl SessionAgeing {
    pub(super) fn get(&self) -> Ageing {
        *self.0.lock()
    }
}

impl Clone for SessionAgeing {
    fn clone(&self) -> Self {
        SessionAgeing(parking_lot::Mutex::new(self.get()))
    }
}

impl ContentStore {
    /// Keeps ended sessions' data as `ageing` says, on this handle (a test's: `Ageing { bytes: 0, backstop_secs: 0 }`
    /// releases an ended workspace at the next pass, as before decision 0450 step 3).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_session_ageing(&self, ageing: Ageing) {
        *self.session_ageing.0.lock() = ageing;
    }
}
pub(super) const OBJECT_SOURCE: i64 = 2;
pub(super) const STORE_SOFT_LIMIT: u64 = 7 * 1024 * 1024 * 1024;
pub(super) const STORE_HARD_LIMIT: u64 = 9 * 1024 * 1024 * 1024;
pub(super) const STORE_ABSOLUTE_LIMIT: u64 = 10 * 1024 * 1024 * 1024;

impl ContentStore {
    pub fn pin_source(
        &self,
        source_id: SourceId,
        owner: &str,
        lease_until: i64,
    ) -> StoreResult<()> {
        self.pin_object(OBJECT_SOURCE, source_id.0, owner, lease_until)
    }

    /// When `owner`'s pin of `source_id` ends (unix seconds), where it has one: a reader renews a lease only once it
    /// has run down, so a run that reads what an earlier run kept writes nothing (decision 0250).
    pub fn source_lease(&self, source_id: SourceId, owner: &str) -> StoreResult<Option<i64>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row_cached(
                "SELECT p.lease_until FROM object_pins p JOIN strings s ON s.string_id=p.owner_string_id \
                 WHERE p.object_kind_id=?1 AND p.object_id=?2 AND s.domain_id=?3 AND s.value=?4",
                params![OBJECT_SOURCE, source_id.0, STRING_OWNER, owner],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// `source_lease` of many sources, in their order, under one hold of the connection.
    pub fn source_leases(&self, source_ids: &[SourceId], owner: &str) -> StoreResult<Vec<Option<i64>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT p.lease_until FROM object_pins p JOIN strings s ON s.string_id=p.owner_string_id \
             WHERE p.object_kind_id=?1 AND p.object_id=?2 AND s.domain_id=?3 AND s.value=?4",
        )?;
        source_ids.iter().map(|id| statement.query_row(params![OBJECT_SOURCE, id.0, STRING_OWNER, owner], |row| row.get(0)).optional().map_err(StoreError::from)).collect()
    }

    pub fn pin_tape(&self, tape_id: TapeId, owner: &str, lease_until: i64) -> StoreResult<()> {
        self.pin_object(OBJECT_TAPE, tape_id.0, owner, lease_until)
    }

    fn pin_object(
        &self,
        object_kind: i64,
        object_id: i64,
        owner: &str,
        lease_until: i64,
    ) -> StoreResult<()> {
        if owner.is_empty() || lease_until <= 0 {
            return Err(StoreError::Invalid("object pin is invalid".to_owned()));
        }
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction =
            super::connection::begin(&mut connection, TransactionBehavior::Immediate)?;
        let owner_id = intern_string(&transaction, STRING_OWNER, owner)?;
        transaction.execute_cached(
            "INSERT INTO object_pins(object_kind_id,object_id,owner_string_id,lease_until) VALUES (?1,?2,?3,?4)\
             ON CONFLICT(object_kind_id,object_id,owner_string_id) DO UPDATE SET lease_until=excluded.lease_until",
            params![object_kind, object_id, owner_id, lease_until],
        )?;
        // Its owner is a store session (decision 0450 §2), live while one of its pins is.
        transaction.execute_cached(
            "INSERT INTO store_sessions(kind,key,latest,last_used,ended,aged_bytes) VALUES (?1,?2,?3,?4,NULL,0) \
             ON CONFLICT(kind,key) DO UPDATE SET latest=excluded.latest,last_used=excluded.last_used,ended=NULL,aged_bytes=0",
            params![SESSION_PINS, owner_id, generation(&transaction)?, super::unix_seconds()?],
        )?;
        self.commit(transaction)?;
        Ok(())
    }

    /// The stored source of each of `hashes`, in their order: one read, nothing written. A run holds what it finds by
    /// its collection lease (`hold_collection`, decision 0306 step 4).
    pub fn sources_by_hashes(&self, hashes: &[HashKey]) -> StoreResult<Vec<Option<SourceId>>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        sources_by_hashes(&connection, hashes)
    }

    pub fn release_pins(&self, owner: &str) -> StoreResult<usize> {
        // A write: it waits out other writers as every store write does, the connection held by each attempt alone.
        retry_busy(&mut BusyRetries::default(), || {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let Some(owner_id) = connection
                .query_row_cached("SELECT string_id FROM strings WHERE domain_id=?1 AND value=?2", params![STRING_OWNER, owner], |r| r.get::<_, i64>(0))
                .optional()?
            else {
                return Ok(0);
            };
            let released = connection.execute_cached("DELETE FROM object_pins WHERE owner_string_id=?1", [owner_id])?;
            // Released by its owner, what it pinned is not aged (decision 0450 step 3): its session goes with its pins.
            connection.execute_cached("DELETE FROM store_sessions WHERE kind=?1 AND key=?2", params![SESSION_PINS, owner_id])?;
            Ok(released)
        })
    }

    pub fn capacity(&self) -> StoreResult<StoreCapacity> {
        let mut bytes = 0_u64;
        for suffix in ["", "-wal", "-shm"] {
            let path = if suffix.is_empty() {
                self.database_path.clone()
            } else {
                std::path::PathBuf::from(format!("{}{suffix}", self.database_path.display()))
            };
            match fs::metadata(path) {
                Ok(metadata) => {
                    bytes = bytes.checked_add(metadata.len()).ok_or_else(|| {
                        StoreError::Corrupt("content store size overflow".to_owned())
                    })?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(StoreError::Io(error)),
            }
        }
        // The segments beside it (decision 0295): tapes and sources, frames, memo entries, uncompressed.
        let bytes = bytes.saturating_add(super::segments::segment_bytes(&self.database_path));
        Ok(StoreCapacity {
            bytes,
            state: capacity_state(bytes),
        })
    }

    pub(super) fn require_write_capacity(&self) -> StoreResult<()> {
        match self.capacity()?.state {
            CapacityState::Normal | CapacityState::Collect => Ok(()),
            CapacityState::RejectWrites | CapacityState::OverBudget => Err(StoreError::Invalid(
                "content store hard limit rejects new payloads".to_owned(),
            )),
        }
    }

    /// Deletes up to `batch` unreachable rows per table in one write transaction. Unreachable Git
    /// trees are deleted whole, each row with all of its entries, so a surviving tree row always
    /// has its complete entry set: up to `batch` trees whose entries fit the publication budget
    /// `GIT_TREE_BATCH_ENTRIES`, or the first one alone when it is larger.
    /// Returns an empty report while a cache reader or another collector holds the collection lease.
    pub fn collect(&self, now: i64, batch: usize) -> StoreResult<CollectionReport> {
        if batch == 0 || batch > MAX_COLLECTION_BATCH {
            return Err(StoreError::Invalid(format!(
                "collection batch must be between 1 and {MAX_COLLECTION_BATCH}"
            )));
        }
        let Some(_lease) = self.try_collect()? else {
            return Ok(CollectionReport::default());
        };
        self.collect_held(now, batch)
    }

    /// The caller holds the exclusive collection lease through the complete pass. Its transaction, and then the
    /// vacuum (a write too), wait out other writers as every store write does (`retry_busy`): one attempt each, with
    /// SQLite's busy timeout alone, failed the pass while parallel writers held the store (O-collector-busy-flake).
    pub(super) fn collect_held(&self, now: i64, batch: usize) -> StoreResult<CollectionReport> {
        let report = retry_busy(&mut BusyRetries::default(), || {
            self.collect_held_once(now, batch)
        })?;
        // Free pages back to the file system, a write of its own after the pass committed: a writer holding the
        // store past the retries leaves them to the next pass. Failing the pass for it failed a collector whose rows
        // were already collected. A file mostly free is the upkeep job's, vacuumed whole: an incremental vacuum's
        // cost grows with the free list (ep2: 8.1 s for these 4,096 pages with 1.8M free, behind every run).
        match retry_busy(&mut BusyRetries::default(), || {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            connection.query_row_cached("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))?;
            if super::upkeep::leaves_to_whole_vacuum(&connection, self.database_path())? {
                return Ok(());
            }
            incremental_vacuum(&connection, 4096)
        }) {
            Ok(()) | Err(StoreError::Busy) => {}
            Err(error) => return Err(error),
        }
        Ok(report)
    }

    /// One attempt at `collect_held`'s transaction: run again whole if it finds the store busy.
    fn collect_held_once(&self, now: i64, batch: usize) -> StoreResult<CollectionReport> {
        let limit = i64::try_from(batch)
            .map_err(|_| StoreError::Invalid("collection batch exceeds i64".to_owned()))?;
        let entry_budget = GIT_TREE_BATCH_ENTRIES as i64;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = super::connection::begin(&mut connection, TransactionBehavior::Immediate)?;
        Self::discard_abandoned_analysis(&transaction)?;
        let mut report = CollectionReport::default();
        execute_collect(
            &transaction,
            "UPDATE workspaces SET state_id=2 WHERE state_id=1 AND lease_until < ?1",
            [now],
        )?;
        // A workspace's store session ends when its lease does (decision 0450 §2), with the bytes its snapshot and
        // overlay hold; its data is aged, not released at once.
        let mut ended = execute_collect(
            &transaction,
            "UPDATE store_sessions SET ended=w.lease_until, aged_bytes=\
             coalesce((SELECT sum(o.seg_len) FROM snapshot_files f JOIN source_objects o ON o.source_id=f.source_id WHERE f.snapshot_id=w.snapshot_id),0)\
             +coalesce((SELECT sum(t.seg_len) FROM snapshot_files f JOIN tape_objects t ON t.tape_id=f.tape_id WHERE f.snapshot_id=w.snapshot_id),0)\
             +coalesce((SELECT sum(o.seg_len) FROM workspace_overlay v JOIN source_objects o ON o.source_id=v.source_id WHERE v.workspace_id=w.workspace_id),0)\
             +coalesce((SELECT sum(t.seg_len) FROM workspace_overlay v JOIN tape_objects t ON t.tape_id=v.tape_id WHERE v.workspace_id=w.workspace_id),0) \
             FROM workspaces w WHERE store_sessions.kind=?1 AND store_sessions.key=w.root_path_id AND store_sessions.ended IS NULL \
             AND w.state_id=2",
            [SESSION_WORKSPACE],
        )?;
        // A pin owner's session ends when its last source or tape pin lapses, with the bytes they hold.
        ended += execute_collect(
            &transaction,
            "UPDATE store_sessions SET ended=coalesce((SELECT max(p.lease_until) FROM object_pins p WHERE p.owner_string_id=store_sessions.key),?2), \
             aged_bytes=coalesce((SELECT sum(o.seg_len) FROM object_pins p JOIN source_objects o ON o.source_id=p.object_id \
                                  WHERE p.owner_string_id=store_sessions.key AND p.object_kind_id=?3),0)\
             +coalesce((SELECT sum(t.seg_len) FROM object_pins p JOIN tape_objects t ON t.tape_id=p.object_id \
                        WHERE p.owner_string_id=store_sessions.key AND p.object_kind_id=?4),0) \
             WHERE kind=?1 AND ended IS NULL \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.owner_string_id=store_sessions.key AND p.lease_until>=?2)",
            params![SESSION_PINS, now, OBJECT_SOURCE, OBJECT_TAPE],
        )?;
        if ended > 0 {
            // What they hold may be archived (decision 0450 step 4).
            execute_collect(&transaction, "UPDATE corpus_state SET archive_due=1 WHERE singleton=1", [])?;
        }
        // An ended workspace is released (state 3) once it ended `backstop` ago, or once the sessions that ended after
        // it hold the budget; one with no session (a store of an older binary) at once.
        let ageing = self.session_ageing.get();
        execute_collect(
            &transaction,
            concat!(
                "UPDATE workspaces SET state_id=3 WHERE state_id=2 AND workspace_id IN (SELECT w.workspace_id FROM workspaces w \
                 LEFT JOIN ",
                aged_sessions!(),
                " r ON r.kind=?1 AND r.key=w.root_path_id \
                 WHERE w.state_id=2 AND (r.key IS NULL OR r.ended <= ?2 - ?3 OR r.held > ?4))"
            ),
            params![SESSION_WORKSPACE, now, ageing.backstop_secs, i64::try_from(ageing.bytes).unwrap_or(i64::MAX)],
        )?;
        // A lapsed pin goes at once unless it is a source's or tape's and its owner's session is aged: a live owner's
        // lapsed pins are not its latest data, and a plan's hold lapses (`corpus_runs::lapsed_plan_hold`).
        report.expired_pins = execute_collect(
            &transaction,
            concat!(
                "DELETE FROM object_pins WHERE (object_kind_id,object_id,owner_string_id) IN (\
                 SELECT p.object_kind_id,p.object_id,p.owner_string_id FROM object_pins p LEFT JOIN ",
                aged_sessions!(),
                " r ON r.kind=?1 AND r.key=p.owner_string_id \
                 WHERE p.lease_until < ?2 AND (p.object_kind_id NOT IN (?5,?6) OR r.key IS NULL OR r.ended <= ?2 - ?3 OR r.held > ?4) \
                 ORDER BY p.object_kind_id,p.object_id,p.owner_string_id LIMIT ?7)"
            ),
            params![SESSION_PINS, now, ageing.backstop_secs, i64::try_from(ageing.bytes).unwrap_or(i64::MAX), OBJECT_SOURCE, OBJECT_TAPE, limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM store_sessions WHERE kind=?1 AND ended IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.owner_string_id=store_sessions.key)",
            [SESSION_PINS],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM store_sessions WHERE kind=?1 AND key IN (SELECT root_path_id FROM workspaces WHERE state_id=3)",
            [SESSION_WORKSPACE],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM workspace_overlay WHERE (workspace_id,path_id) IN (\
             SELECT o.workspace_id,o.path_id FROM workspace_overlay o JOIN workspaces w ON w.workspace_id=o.workspace_id \
             WHERE w.state_id=3 ORDER BY o.workspace_id,o.path_id LIMIT ?1)",
            [limit],
        )?;
        report.retired_workspaces = execute_collect(
            &transaction,
            "DELETE FROM workspaces WHERE workspace_id IN (SELECT w.workspace_id FROM workspaces w \
             WHERE w.state_id=3 AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.workspace_id=w.workspace_id) \
             ORDER BY w.workspace_id LIMIT ?1)",
            [limit],
        )?;
        let snapshot = transaction
            .query_row(
                collect_sql(
                    "SELECT s.snapshot_id FROM snapshots s WHERE NOT EXISTS (SELECT 1 FROM workspaces w WHERE w.snapshot_id=s.snapshot_id) \
                     AND NOT EXISTS (SELECT 1 FROM analysis_scope_snapshot a WHERE a.snapshot_id=s.snapshot_id) \
                     AND NOT EXISTS (SELECT 1 FROM analysis_scope_workspace_snapshot a WHERE a.snapshot_id=s.snapshot_id) \
                     ORDER BY s.snapshot_id LIMIT 1",
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(snapshot_id) = snapshot {
            report.snapshot_files = execute_collect(
                &transaction,
                "DELETE FROM snapshot_files WHERE (snapshot_id,path_id) IN (\
                 SELECT f.snapshot_id,f.path_id FROM snapshot_files f WHERE f.snapshot_id=?1 \
                 ORDER BY f.path_id LIMIT ?2)",
                params![snapshot_id, limit],
            )?;
            if report.snapshot_files == 0 {
                report.snapshots = execute_collect(
                    &transaction,
                    "DELETE FROM snapshots WHERE snapshot_id=?1",
                    [snapshot_id],
                )?;
            }
        }
        let current_generation = generation(&transaction)?;
        // Tapes leave in bulk, and their ids may be reused: the working set holds none of them.
        self.working_set().clear();
        // The tapes' prefilter facts and search indexes leave in the same transaction (t33 steps 6 and 7): an id
        // reused names a new tape.
        let tapes: Vec<i64> = transaction
            .prepare(collect_sql(
            "DELETE FROM tape_objects WHERE tape_id IN (SELECT t.tape_id FROM tape_objects t \
             WHERE t.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.tape_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.tape_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM file_documents d WHERE d.document_tape_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM corpus_live l WHERE l.source_id=t.source_id \
                             AND l.grammar_version=t.grammar_version AND l.tape_format=t.tape_format_version) \
             ORDER BY t.tape_id LIMIT ?3) RETURNING tape_id",
            ))?
            .query_map(params![current_generation, OBJECT_TAPE, limit], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        report.tapes = tapes.len();
        let prefilter = super::prefilter::forget_tapes(&self.database_path, &transaction, &tapes)?;
        let search = super::search_index::forget_tapes(&self.database_path, &transaction, &tapes)?;
        execute_collect(
            &transaction,
            "DELETE FROM source_aliases WHERE source_id IN (SELECT s.source_id FROM source_objects s \
             WHERE s.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM corpus_live l WHERE l.source_id=s.source_id) \
             ORDER BY s.source_id LIMIT ?3)",
            params![current_generation, OBJECT_SOURCE, limit],
        )?;
        report.sources = execute_collect(
            &transaction,
            "DELETE FROM source_objects WHERE source_id IN (SELECT s.source_id FROM source_objects s \
             WHERE s.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM corpus_live l WHERE l.source_id=s.source_id) \
             ORDER BY s.source_id LIMIT ?3)",
            params![current_generation, OBJECT_SOURCE, limit],
        )?;
        // A workspace's results whose content went with the sources above, and its completed directories of a
        // catalog with no result left (`results.rs`).
        super::results::collect_workspace_results(&transaction)?;
        // The live set is materialized once (NOT IN), not rescanned per candidate tree. Whole
        // trees are taken in tree order until their entries would exceed the publication budget,
        // and always at least one, so a pass holds the write lock over a bounded number of rows.
        // A tree without a recorded count (written by a binary that kept none) is counted.
        let unreachable_trees = transaction
            .prepare(collect_sql(
                "WITH RECURSIVE live_trees(tree_id) AS (\
                 SELECT root_tree_id FROM snapshots \
                 UNION \
                 SELECT child.tree_id FROM live_trees parent JOIN git_tree_entries e \
                 ON e.tree_id=parent.tree_id AND e.object_kind_id=1 \
                 JOIN git_trees child ON child.tree_hash_id=e.target_hash_id), \
                 candidates(tree_id,cost) AS (\
                 SELECT t.tree_id,max(1,coalesce(t.entry_count,\
                 (SELECT count(*) FROM git_tree_entries c WHERE c.tree_id=t.tree_id))) \
                 FROM git_trees t WHERE t.tree_id NOT IN (SELECT tree_id FROM live_trees) \
                 ORDER BY t.tree_id LIMIT ?1), \
                 budgeted(tree_id,spent,position) AS (\
                 SELECT tree_id,sum(cost) OVER (ORDER BY tree_id),row_number() OVER (ORDER BY tree_id) \
                 FROM candidates) \
                 SELECT tree_id FROM budgeted WHERE position=1 OR spent<=?2 ORDER BY tree_id",
            ))?
            .query_map(params![limit, entry_budget], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let unreachable_trees = serde_json::to_string(&unreachable_trees)
            .map_err(|error| StoreError::Invalid(error.to_string()))?;
        execute_collect(
            &transaction,
            "DELETE FROM git_tree_entries WHERE tree_id IN (SELECT value FROM json_each(?1))",
            [&unreachable_trees],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM git_trees WHERE tree_id IN (SELECT value FROM json_each(?1))",
            [&unreachable_trees],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM paths WHERE path_id IN (\
             SELECT p.path_id FROM paths p WHERE NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.path_id=p.path_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.path_id=p.path_id) \
             AND NOT EXISTS (SELECT 1 FROM workspaces w WHERE w.root_path_id=p.path_id) \
             ORDER BY p.path_id LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM strings WHERE string_id IN (\
             SELECT s.string_id FROM strings s WHERE NOT EXISTS (SELECT 1 FROM paths p WHERE p.string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM git_tree_entries e WHERE e.name_string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM workspaces w WHERE w.repository_string_id=s.string_id OR w.owner_string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.owner_string_id=s.string_id) \
             ORDER BY s.string_id LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM hashes WHERE hash_id IN (\
             SELECT h.hash_id FROM hashes h WHERE NOT EXISTS (SELECT 1 FROM strings s WHERE s.content_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM source_objects s WHERE s.canonical_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM source_aliases a WHERE a.alias_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.canonical_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM git_trees t WHERE t.tree_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM git_tree_entries e WHERE e.target_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshots s WHERE s.grammar_set_hash_id=h.hash_id) \
             ORDER BY h.hash_id LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "UPDATE store_state SET generation=generation+1 WHERE singleton=1",
            [],
        )?;
        self.commit(transaction)?;
        self.note_kept(&super::prefilter::kept::space(), prefilter);
        self.note_kept(&super::search_index::space(), search);
        Ok(report)
    }
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Every SQL statement `collect` ran on this thread, in execution order.
    pub(super) static EXECUTED_COLLECT_SQL: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Every statement `collect` runs passes through here, so tests EXPLAIN the exact SQL it executed.
fn collect_sql(sql: &'static str) -> &'static str {
    #[cfg(any(test, feature = "test-hooks"))]
    EXECUTED_COLLECT_SQL.with(|executed| executed.borrow_mut().push(sql));
    sql
}

fn execute_collect(
    transaction: &Connection,
    sql: &'static str,
    params: impl Params,
) -> StoreResult<usize> {
    transaction
        .execute(collect_sql(sql), params)
        .map_err(StoreError::from)
}

/// Returns up to `pages` free pages to the file system (16 MB at 4096: a grammar change frees about 63 MB on the full
/// corpus, decision 0053). The pragma frees a page per step, so it is stepped to the end: run as a batch, it freed none.
pub(super) fn incremental_vacuum(connection: &rusqlite::Connection, pages: u32) -> StoreResult<()> {
    let mut statement = connection.prepare(&format!("PRAGMA incremental_vacuum({pages})"))?;
    let mut rows = statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

pub(super) fn capacity_state(bytes: u64) -> CapacityState {
    if bytes >= STORE_ABSOLUTE_LIMIT {
        CapacityState::OverBudget
    } else if bytes >= STORE_HARD_LIMIT {
        CapacityState::RejectWrites
    } else if bytes >= STORE_SOFT_LIMIT {
        CapacityState::Collect
    } else {
        CapacityState::Normal
    }
}

/// The stored source of each of `hashes`, in their order, read on `connection` in one statement.
fn sources_by_hashes(
    connection: &rusqlite::Connection,
    hashes: &[HashKey],
) -> StoreResult<Vec<Option<SourceId>>> {
    let mut out = vec![None; hashes.len()];
    if hashes.is_empty() {
        return Ok(out);
    }
    let list = serde_json::to_string(
        &hashes
            .iter()
            .map(|h| super::rows::hash_params(*h))
            .collect::<Vec<_>>(),
    )
    .expect("integers");
    let mut statement = connection.prepare_cached(
        "SELECT j.key, s.source_id FROM json_each(?1) j JOIN hashes h INDEXED BY hashes_by_prefix \
         ON h.algorithm_id=json_extract(j.value,'$[0]') AND h.lane0=json_extract(j.value,'$[1]') \
         AND h.lane1=json_extract(j.value,'$[2]') AND h.lane2=json_extract(j.value,'$[3]') AND h.lane3=json_extract(j.value,'$[4]') \
         JOIN source_objects s ON s.canonical_hash_id=h.hash_id",
    )?;
    for row in statement.query_map([&list], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })? {
        let (at, id) = row?;
        out[at as usize] = Some(SourceId(id));
    }
    Ok(out)
}
