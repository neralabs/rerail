//! Workspace registry, overlays, promotion to snapshots, and workspace leases.

#[cfg(test)]
use super::StoredFile;
use super::rows::{intern_path, intern_string, validate_relative_path};
use super::{
    ContentStore, HashKey, MAX_PUBLICATION_BATCH, STRING_OWNER, STRING_PATH, STRING_REPOSITORY,
    SnapshotId, SourceId, StoreError, StoreResult, StoredPathObjects, StoredSourceObjects, TapeId,
    WorkspaceId, WorkspaceRegistration,
};
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{Transaction, TransactionBehavior, params};
use std::collections::HashMap;

impl WorkspaceRegistration<'_> {
    pub(super) fn validate(&self) -> StoreResult<()> {
        if self.root.is_empty() || self.repository.is_empty() || self.owner.is_empty() {
            return Err(StoreError::Invalid(
                "workspace identity fields must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    /// Registers the workspace over `snapshot_id`, which `transaction` must keep alive: a
    /// snapshot no workspace references is collectable. The lease ends at `lease_until`.
    pub(super) fn register(
        &self,
        transaction: &Transaction<'_>,
        snapshot_id: SnapshotId,
        lease_until: i64,
    ) -> StoreResult<WorkspaceId> {
        let root_path_id = intern_path(transaction, self.root)?;
        let repository_id = intern_string(transaction, STRING_REPOSITORY, self.repository)?;
        let owner_id = intern_string(transaction, STRING_OWNER, self.owner)?;
        transaction.execute(
            "INSERT INTO workspaces(root_path_id,repository_string_id,owner_string_id,snapshot_id,generation,lease_until,state_id)\
             VALUES (?1,?2,?3,?4,?5,?6,1)\
             ON CONFLICT(root_path_id) DO UPDATE SET repository_string_id=excluded.repository_string_id,owner_string_id=excluded.owner_string_id,\
             snapshot_id=excluded.snapshot_id,generation=excluded.generation,lease_until=excluded.lease_until,state_id=1",
            params![root_path_id, repository_id, owner_id, snapshot_id.0, self.generation, lease_until],
        )?;
        let workspace_id = transaction.query_row(
            "SELECT workspace_id FROM workspaces WHERE root_path_id=?1",
            [root_path_id],
            |row| row.get(0),
        )?;
        Ok(WorkspaceId(workspace_id))
    }
}

impl ContentStore {
    /// Registers a workspace over an existing snapshot with a lease ending at `lease_until`.
    /// Production registers its workspace in the snapshot's own transaction (`publish_snapshot`),
    /// so collection cannot delete it in between.
    #[cfg(test)]
    pub fn open_workspace(
        &self,
        root: &str,
        repository: &str,
        owner: &str,
        snapshot_id: SnapshotId,
        generation: i64,
        lease_until: i64,
    ) -> StoreResult<WorkspaceId> {
        let registration = WorkspaceRegistration {
            root,
            repository,
            owner,
            generation,
            lease_seconds: 0,
        };
        registration.validate()?;
        self.immediate_transaction(|transaction| {
            registration.register(transaction, snapshot_id, lease_until)
        })
    }

    pub fn put_overlay(
        &self,
        workspace_id: WorkspaceId,
        path: &str,
        state_id: i64,
        source_id: Option<SourceId>,
        tape_id: Option<TapeId>,
    ) -> StoreResult<()> {
        validate_relative_path(path)?;
        if state_id <= 0 {
            return Err(StoreError::Invalid(
                "overlay state must be positive".to_owned(),
            ));
        }
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let path_id = intern_path(&transaction, path)?;
        transaction.execute(
            "INSERT INTO workspace_overlay(workspace_id,path_id,state_id,source_id,tape_id) VALUES (?1,?2,?3,?4,?5)\
             ON CONFLICT(workspace_id,path_id) DO UPDATE SET state_id=excluded.state_id,source_id=excluded.source_id,tape_id=excluded.tape_id",
            params![workspace_id.0, path_id, state_id, source_id.map(|id| id.0), tape_id.map(|id| id.0)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn promote_overlay_files_to_snapshot(
        &self,
        workspace_id: WorkspaceId,
        snapshot_id: SnapshotId,
        paths: &[&str],
    ) -> StoreResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if paths.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(format!(
                "snapshot promotion batch cannot exceed {MAX_PUBLICATION_BATCH} files"
            )));
        }
        for path in paths {
            validate_relative_path(path)?;
        }
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for path in paths {
            let path_id = intern_path(&transaction, path)?;
            transaction.execute(
                "INSERT INTO snapshot_files(snapshot_id,path_id,source_id,tape_id) \
                 SELECT ?1,path_id,source_id,tape_id FROM workspace_overlay \
                 WHERE workspace_id=?2 AND path_id=?3 \
                 ON CONFLICT(snapshot_id,path_id) DO UPDATE SET \
                 source_id=excluded.source_id,tape_id=excluded.tape_id",
                params![snapshot_id.0, workspace_id.0, path_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn overlay_file(
        &self,
        workspace_id: WorkspaceId,
        path: &str,
    ) -> StoreResult<Option<StoredFile>> {
        validate_relative_path(path)?;
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row(
                "SELECT o.source_id,o.tape_id FROM workspace_overlay o \
                 JOIN paths p ON p.path_id=o.path_id JOIN strings s ON s.string_id=p.string_id \
                 WHERE o.workspace_id=?1 AND s.domain_id=?2 AND s.value=?3",
                params![workspace_id.0, STRING_PATH, path],
                |row| {
                    Ok(StoredFile {
                        source: SourceId(row.get(0)?),
                        tape: row.get::<_, Option<i64>>(1)?.map(TapeId),
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn remove_overlay_files(
        &self,
        workspace_id: WorkspaceId,
        paths: &[String],
    ) -> StoreResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for path in paths {
            validate_relative_path(path)?;
        }
        let encoded =
            serde_json::to_string(paths).map_err(|error| StoreError::Invalid(error.to_string()))?;
        self.immediate_transaction(|transaction| {
            transaction.execute(
                "DELETE FROM workspace_overlay WHERE workspace_id=?1 AND path_id IN (\
                 SELECT p.path_id FROM json_each(?2) i JOIN strings s ON s.domain_id=?3 AND s.value=i.value \
                 JOIN paths p ON p.string_id=s.string_id)",
                params![workspace_id.0, encoded, STRING_PATH],
            )?;
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn remove_overlay_file(&self, workspace_id: WorkspaceId, path: &str) -> StoreResult<()> {
        validate_relative_path(path)?;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let path_id = transaction
            .query_row(
                "SELECT p.path_id FROM paths p JOIN strings s ON s.string_id=p.string_id \
                 WHERE s.domain_id=?1 AND s.value=?2",
                params![STRING_PATH, path],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(path_id) = path_id {
            transaction.execute(
                "DELETE FROM workspace_overlay WHERE workspace_id=?1 AND path_id=?2",
                params![workspace_id.0, path_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn workspace_stored_path_manifest(
        &self,
        workspace_id: WorkspaceId,
        snapshot_id: SnapshotId,
        grammar_version: i64,
        tape_format_version: i64,
    ) -> StoreResult<HashMap<String, StoredPathObjects>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare(
            "WITH effective(path_id,source_id,tape_id) AS (\
             SELECT path_id,source_id,tape_id FROM workspace_overlay WHERE workspace_id=?1 AND source_id IS NOT NULL \
             UNION ALL \
             SELECT f.path_id,f.source_id,f.tape_id FROM snapshot_files f WHERE f.snapshot_id=?2 \
             AND NOT EXISTS (SELECT 1 FROM workspace_overlay o WHERE o.workspace_id=?1 AND o.path_id=f.path_id)\
             ) SELECT ps.value,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,e.source_id,\
             t.grammar_id,t.tape_id FROM effective e \
             JOIN paths p ON p.path_id=e.path_id JOIN strings ps ON ps.string_id=p.string_id \
             JOIN source_objects s ON s.source_id=e.source_id \
             JOIN hashes h ON h.hash_id=s.canonical_hash_id \
             LEFT JOIN tape_objects t ON t.source_id=e.source_id \
             AND t.grammar_version=?3 AND t.tape_format_version=?4 \
             ORDER BY e.path_id,t.grammar_id",
        )?;
        let rows = statement
            .query_map(
                params![
                    workspace_id.0,
                    snapshot_id.0,
                    grammar_version,
                    tape_format_version
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        HashKey {
                            algorithm: row.get(1)?,
                            lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                        },
                        SourceId(row.get(6)?),
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let mut manifest = HashMap::new();
        for (path, source_hash, source, grammar, tape) in rows {
            let stored = manifest.entry(path).or_insert_with(|| StoredPathObjects {
                source_hash,
                objects: StoredSourceObjects {
                    source,
                    tapes: HashMap::new(),
                },
            });
            if let (Some(grammar), Some(tape)) = (grammar, tape) {
                stored.objects.tapes.insert(grammar, TapeId(tape));
            }
        }
        Ok(manifest)
    }

    pub fn renew_workspace_lease(
        &self,
        workspace_id: WorkspaceId,
        owner: &str,
        lease_until: i64,
    ) -> StoreResult<()> {
        let changed = self.renew_workspace_leases(&[(workspace_id, owner)], lease_until)?;
        if changed != 1 {
            return Err(StoreError::Invalid(
                "workspace lease owner does not match".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn renew_workspace_leases(
        &self,
        workspaces: &[(WorkspaceId, &str)],
        lease_until: i64,
    ) -> StoreResult<usize> {
        if lease_until <= 0 || workspaces.iter().any(|(_, owner)| owner.is_empty()) {
            return Err(StoreError::Invalid("workspace lease is invalid".to_owned()));
        }
        if workspaces.is_empty() {
            return Ok(0);
        }
        let input = workspaces
            .iter()
            .map(|(workspace_id, owner)| serde_json::json!([workspace_id.0, owner]))
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&input).map_err(|error| {
            StoreError::Invalid(format!("workspace leases cannot be encoded: {error}"))
        })?;
        self.immediate_transaction(|transaction| {
            transaction
                .prepare_cached(
                    "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) workspace_id, \
                     json_extract(value,'$[1]') owner FROM json_each(?1)) \
                     UPDATE workspaces SET lease_until=?2 WHERE state_id=1 AND EXISTS (SELECT 1 FROM input i \
                     JOIN strings s ON s.domain_id=?3 AND s.value=i.owner \
                     WHERE i.workspace_id=workspaces.workspace_id AND s.string_id=workspaces.owner_string_id)",
                )?
                .execute(params![encoded, lease_until, STRING_OWNER])
                .map_err(StoreError::from)
        })
    }

    pub fn retire_workspace(&self, workspace_id: WorkspaceId) -> StoreResult<()> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection.execute(
            "UPDATE workspaces SET state_id=2, lease_until=0 WHERE workspace_id=?1",
            [workspace_id.0],
        )?;
        Ok(())
    }
}
