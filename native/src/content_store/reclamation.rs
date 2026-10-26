//! Capacity limits, object pins, and collection of unreachable rows.

use super::rows::{generation, intern_string};
use super::snapshots::GIT_TREE_BATCH_ENTRIES;
use super::{
    CapacityState, CollectionReport, ContentStore, MAX_COLLECTION_BATCH, STRING_OWNER, SourceId,
    StoreCapacity, StoreError, StoreResult, TapeId,
};
use rusqlite::{OptionalExtension, Params, Transaction, TransactionBehavior, params};
use std::fs;
use std::time::Instant;

const OBJECT_TAPE: i64 = 1;
const OBJECT_SOURCE: i64 = 2;

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
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner_id = intern_string(&transaction, STRING_OWNER, owner)?;
        transaction.execute(
            "INSERT INTO object_pins(object_kind_id,object_id,owner_string_id,lease_until) VALUES (?1,?2,?3,?4)\
             ON CONFLICT(object_kind_id,object_id,owner_string_id) DO UPDATE SET lease_until=excluded.lease_until",
            params![object_kind, object_id, owner_id, lease_until],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn release_pins(&self, owner: &str) -> StoreResult<usize> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection.execute(
            "DELETE FROM object_pins WHERE owner_string_id IN (SELECT string_id FROM strings WHERE domain_id=?1 AND value=?2)",
            params![STRING_OWNER, owner],
        ).map_err(StoreError::from)
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
    pub fn collect(&self, now: i64, batch: usize) -> StoreResult<CollectionReport> {
        self.decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .expire(Instant::now());
        if batch == 0 || batch > MAX_COLLECTION_BATCH {
            return Err(StoreError::Invalid(format!(
                "collection batch must be between 1 and {MAX_COLLECTION_BATCH}"
            )));
        }
        let limit = i64::try_from(batch)
            .map_err(|_| StoreError::Invalid("collection batch exceeds i64".to_owned()))?;
        let entry_budget = GIT_TREE_BATCH_ENTRIES as i64;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expired_pins = execute_collect(
            &transaction,
            "DELETE FROM object_pins WHERE (object_kind_id,object_id,owner_string_id) IN (\
             SELECT object_kind_id,object_id,owner_string_id FROM object_pins WHERE lease_until < ?1 \
             ORDER BY object_kind_id,object_id,owner_string_id LIMIT ?2)",
            params![now, limit],
        )?;
        let mut report = CollectionReport {
            expired_pins,
            ..CollectionReport::default()
        };
        execute_collect(
            &transaction,
            "UPDATE workspaces SET state_id=2 WHERE state_id=1 AND lease_until < ?1",
            [now],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM workspace_overlay WHERE (workspace_id,path_id) IN (\
             SELECT o.workspace_id,o.path_id FROM workspace_overlay o JOIN workspaces w ON w.workspace_id=o.workspace_id \
             WHERE w.state_id=2 ORDER BY o.workspace_id,o.path_id LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM completed_workspace_revisions WHERE workspace_id IN (SELECT w.workspace_id FROM workspaces w \
             WHERE w.state_id=2 AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.workspace_id=w.workspace_id) \
             ORDER BY w.workspace_id LIMIT ?1)",
            [limit],
        )?;
        report.retired_workspaces = execute_collect(
            &transaction,
            "DELETE FROM workspaces WHERE workspace_id IN (SELECT w.workspace_id FROM workspaces w \
             WHERE w.state_id=2 AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.workspace_id=w.workspace_id) \
             ORDER BY w.workspace_id LIMIT ?1)",
            [limit],
        )?;
        let snapshot = transaction
            .query_row(
                collect_sql(
                    "SELECT s.snapshot_id FROM snapshots s WHERE NOT EXISTS (SELECT 1 FROM workspaces w WHERE w.snapshot_id=s.snapshot_id) \
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
        report.tapes = execute_collect(
            &transaction,
            "DELETE FROM tape_objects WHERE tape_id IN (SELECT t.tape_id FROM tape_objects t \
             WHERE t.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.tape_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.tape_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=t.tape_id) \
             AND NOT EXISTS (SELECT 1 FROM file_documents d WHERE d.document_tape_id=t.tape_id) \
             ORDER BY t.tape_id LIMIT ?3)",
            params![current_generation, OBJECT_TAPE, limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM source_aliases WHERE source_id IN (SELECT s.source_id FROM source_objects s \
             WHERE s.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=s.source_id) \
             ORDER BY s.source_id LIMIT ?3)",
            params![current_generation, OBJECT_SOURCE, limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM local_check_results WHERE source_id IN (SELECT s.source_id FROM source_objects s \
             WHERE s.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=s.source_id) \
             ORDER BY s.source_id LIMIT ?3)",
            params![current_generation, OBJECT_SOURCE, limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM context_check_results WHERE source_id IN (SELECT s.source_id FROM source_objects s \
             WHERE s.created_generation < ?1 \
             AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM snapshot_files f WHERE f.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.source_id=s.source_id) \
             AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=?2 AND p.object_id=s.source_id) \
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
             ORDER BY s.source_id LIMIT ?3)",
            params![current_generation, OBJECT_SOURCE, limit],
        )?;
        report.candidate_checks = execute_collect(
            &transaction,
            "DELETE FROM candidate_checks WHERE check_id IN (SELECT c.check_id FROM candidate_checks c \
             WHERE NOT EXISTS (SELECT 1 FROM local_check_results r WHERE r.check_id=c.check_id) \
             AND NOT EXISTS (SELECT 1 FROM context_check_results r WHERE r.check_id=c.check_id) \
             ORDER BY c.check_id LIMIT ?1)",
            [limit],
        )?;
        report.candidate_catalogs = execute_collect(
            &transaction,
            "DELETE FROM candidate_catalogs WHERE catalog_id IN (SELECT c.catalog_id FROM candidate_catalogs c \
             WHERE NOT EXISTS (SELECT 1 FROM candidate_checks k WHERE k.catalog_id=c.catalog_id) \
             ORDER BY c.catalog_id LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "DELETE FROM completed_directory_reuse WHERE (tree_hash_id,prefix,package_hash_id,catalog_hash_id) IN (\
             SELECT d.tree_hash_id,d.prefix,d.package_hash_id,d.catalog_hash_id FROM completed_directory_reuse d \
             WHERE NOT EXISTS (SELECT 1 FROM candidate_catalogs c JOIN hashes h ON h.hash_id=c.identity_hash_id \
                               WHERE h.hash_id=d.catalog_hash_id) \
             ORDER BY d.tree_hash_id,d.prefix,d.package_hash_id,d.catalog_hash_id LIMIT ?1)",
            [limit],
        )?;
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
             AND NOT EXISTS (SELECT 1 FROM candidate_checks c WHERE c.check_string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM name_bindings b WHERE b.name_string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM derivations d WHERE d.output_string_id=s.string_id) \
             AND NOT EXISTS (SELECT 1 FROM derivation_inputs i WHERE i.name_string_id=s.string_id) \
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
             AND NOT EXISTS (SELECT 1 FROM candidate_catalogs c WHERE c.identity_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM candidate_checks c WHERE c.policy_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM completed_workspace_revisions r WHERE r.catalog_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM completed_directory_reuse d WHERE d.tree_hash_id=h.hash_id OR d.package_hash_id=h.hash_id OR d.catalog_hash_id=h.hash_id) \
             AND NOT EXISTS (SELECT 1 FROM context_check_results r WHERE r.context_hash_id=h.hash_id) \
             ORDER BY h.hash_id LIMIT ?1)",
            [limit],
        )?;
        // Facts memo rows cascade from their sources; a plan set no completion row names any
        // more is dead (facts_memo.rs writes a set in the transaction that first names it).
        execute_collect(
            &transaction,
            "DELETE FROM facts_plan_sets WHERE set_digest IN (\
             SELECT DISTINCT p.set_digest FROM facts_plan_sets p \
             WHERE NOT EXISTS (SELECT 1 FROM facts_completions c WHERE c.set_digest=p.set_digest) \
             ORDER BY p.set_digest LIMIT ?1)",
            [limit],
        )?;
        execute_collect(
            &transaction,
            "UPDATE store_state SET generation=generation+1 WHERE singleton=1",
            [],
        )?;
        transaction.commit()?;
        connection
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA incremental_vacuum(128);")?;
        drop(connection);
        self.decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .clear();
        self.local_result_cache
            .lock()
            .expect("local result cache mutex poisoned")
            .clear();
        Ok(report)
    }
}

#[cfg(test)]
thread_local! {
    /// Every SQL statement `collect` ran on this thread, in execution order.
    pub(super) static EXECUTED_COLLECT_SQL: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Every statement `collect` runs passes through here, so tests EXPLAIN the exact SQL it executed.
fn collect_sql(sql: &'static str) -> &'static str {
    #[cfg(test)]
    EXECUTED_COLLECT_SQL.with(|executed| executed.borrow_mut().push(sql));
    sql
}

fn execute_collect(
    transaction: &Transaction<'_>,
    sql: &'static str,
    params: impl Params,
) -> StoreResult<usize> {
    transaction
        .execute(collect_sql(sql), params)
        .map_err(StoreError::from)
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
