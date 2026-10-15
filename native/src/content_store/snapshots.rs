//! Git tree and snapshot storage.

use super::objects::{put_source_tx, put_tape_tx, validate_tape_payload};
#[cfg(test)]
use super::rows::intern_string;
use super::rows::{
    hash_params, intern_hash, intern_hashed_string, intern_path, validate_relative_path,
};
use super::{
    ContentStore, GitTree, HASH_SHA256, HashKey, MAX_PUBLICATION_BATCH, OperationDeadline,
    STRING_PATH_COMPONENT, SnapshotFilePublication, SnapshotId, SourceId, StoreError, StoreResult,
    StoredFile, TapeId, WorkspaceId, WorkspaceRegistration, compress,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet, HashSet};
#[cfg(test)]
use std::sync::atomic::Ordering;

/// Entry budget of one git-tree publication transaction: cold publication of a large checkout
/// commits a few hundred entries at a time instead of one transaction per tree.
pub(super) const GIT_TREE_BATCH_ENTRIES: usize = 512;

/// The condition under which tree row `t` is complete: publication marked it, and it still has
/// every entry that publication counted. The count reads the entry key's `tree_id` prefix, so a
/// partial entry set is republished whichever writer deleted from it.
macro_rules! complete_git_tree {
    () => {
        "t.complete=1 AND t.entry_count=(SELECT count(*) FROM git_tree_entries e WHERE e.tree_id=t.tree_id)"
    };
}

#[cfg(test)]
thread_local! {
    /// Runs on this thread after every committed Git-tree write transaction, so tests can let
    /// another connection collect between publication's transactions.
    pub(crate) static AFTER_GIT_TREE_TRANSACTION: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
}

impl ContentStore {
    #[cfg(test)]
    pub fn reset_git_tree_write_count(&self) {
        self.git_tree_write_count.store(0, Ordering::Relaxed);
        self.git_tree_lookup_count.store(0, Ordering::Relaxed);
        self.git_tree_transactions.store(0, Ordering::Relaxed);
    }

    /// Write transactions git-tree publication opened since the last reset.
    #[cfg(test)]
    pub fn git_tree_transactions(&self) -> usize {
        self.git_tree_transactions.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn git_tree_write_count(&self) -> usize {
        self.git_tree_write_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn git_tree_lookup_count(&self) -> usize {
        self.git_tree_lookup_count.load(Ordering::Relaxed)
    }

    /// Publishes `trees`, records a snapshot over `root_tree`, one of them, and registers
    /// `workspace` over that snapshot in the snapshot's own transaction: collection deletes every
    /// snapshot no workspace references, so the snapshot commits referenced. Collection also
    /// deletes every tree no snapshot reaches, so it can delete the root after its publication and
    /// before the snapshot exists: the snapshot's transaction then rewrites the root, which commits
    /// reachable. A child collected the same way is republished by the next publication; the
    /// store reads tree entries only to publish and to collect them. The workspace lease starts in
    /// that transaction, so a slow publication never registers an already-expired lease.
    pub fn publish_snapshot(
        &self,
        trees: &[GitTree],
        root_tree: HashKey,
        scanner_version: i64,
        grammar_set: HashKey,
        tape_format_version: i64,
        workspace: &WorkspaceRegistration<'_>,
    ) -> StoreResult<(SnapshotId, WorkspaceId)> {
        workspace.validate()?;
        // The Git-tree batches and the snapshot's own transaction wait on other writers for one
        // deadline in all.
        let _deadline = OperationDeadline::enter();
        let root = trees
            .iter()
            .rev()
            .find(|tree| tree.id == root_tree)
            .ok_or_else(|| {
                StoreError::Invalid("snapshot root is not one of its Git trees".to_owned())
            })?;
        self.put_git_trees_bulk(trees)?;
        let snapshot = |transaction: &Transaction<'_>| {
            insert_snapshot(
                transaction,
                root_tree,
                scanner_version,
                grammar_set,
                tape_format_version,
            )
        };
        self.git_tree_transaction(|transaction| {
            let snapshot_id = match snapshot(transaction)? {
                Some(snapshot_id) => snapshot_id,
                None => {
                    self.git_tree_batch_writer(&[root])?(transaction)?;
                    snapshot(transaction)?.ok_or_else(|| {
                        StoreError::Invalid("snapshot root Git tree is not published".to_owned())
                    })?
                }
            };
            let lease_until = super::unix_seconds()?.saturating_add(workspace.lease_seconds);
            Ok((
                snapshot_id,
                workspace.register(transaction, snapshot_id, lease_until)?,
            ))
        })
    }

    /// Publish every tree not yet stored. Missing trees are packed whole into write transactions
    /// of up to `GIT_TREE_BATCH_ENTRIES` entries; a larger tree is written alone. A tree's row and
    /// all of its entries therefore always commit together.
    pub fn put_git_trees_bulk(&self, trees: &[GitTree]) -> StoreResult<()> {
        let mut skip = self.present_git_tree_ids(trees)?;
        let mut batch = Vec::new();
        let mut batch_entries = 0;
        for tree in trees {
            if !skip.insert(tree.id) {
                continue;
            }
            let entries = tree.entries.len().max(1);
            if !batch.is_empty() && batch_entries + entries > GIT_TREE_BATCH_ENTRIES {
                self.put_git_tree_batch(&batch)?;
                batch.clear();
                batch_entries = 0;
            }
            batch.push(tree);
            batch_entries += entries;
        }
        if !batch.is_empty() {
            self.put_git_tree_batch(&batch)?;
        }
        Ok(())
    }

    fn present_git_tree_ids(&self, trees: &[GitTree]) -> StoreResult<BTreeSet<HashKey>> {
        let keys = serde_json::to_string(
            &trees
                .iter()
                .map(|tree| (tree.id.algorithm, tree.id.lanes))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .expect("content store connection unavailable");
        #[cfg(test)]
        self.git_tree_lookup_count.fetch_add(1, Ordering::Relaxed);
        let found = connection
            .prepare(concat!(
                "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 \
                 FROM json_each(?1) i \
                 JOIN hashes h INDEXED BY hashes_by_prefix \
                   ON h.algorithm_id=json_extract(i.value,'$[0]') \
                  AND h.lane0=json_extract(i.value,'$[1][0]') \
                  AND h.lane1=json_extract(i.value,'$[1][1]') \
                  AND h.lane2=json_extract(i.value,'$[1][2]') \
                  AND h.lane3=json_extract(i.value,'$[1][3]') \
                 JOIN git_trees t ON t.tree_hash_id=h.hash_id AND ",
                complete_git_tree!()
            ))?
            .query_map([keys], |row| {
                Ok(HashKey {
                    algorithm: row.get(0)?,
                    lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                })
            })?
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        Ok(found)
    }

    fn put_git_tree_batch(&self, trees: &[&GitTree]) -> StoreResult<()> {
        let write = self.git_tree_batch_writer(trees)?;
        self.git_tree_transaction(|tx| write(tx))
    }

    /// Runs one Git-tree write transaction.
    fn git_tree_transaction<T>(
        &self,
        mut operation: impl FnMut(&Transaction<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        let written = self.immediate_transaction(|tx| {
            #[cfg(test)]
            self.git_tree_transactions.fetch_add(1, Ordering::Relaxed);
            operation(tx)
        });
        #[cfg(test)]
        if written.is_ok() {
            AFTER_GIT_TREE_TRANSACTION.with(|hook| {
                if let Some(hook) = hook.borrow_mut().as_mut() {
                    hook();
                }
            });
        }
        written
    }

    /// Encodes `trees` once, outside any transaction. The writer stores every one of them, row,
    /// entry count and entries, inside the caller's transaction. Entries are keyed by name, so a
    /// tree with duplicate names (`git fsck` duplicateEntries) stores one row per name, and its
    /// count is of distinct names. Each entry carries its name's hash, so the writer resolves the
    /// name through `strings_by_hash` instead of comparing it with every stored path component.
    fn git_tree_batch_writer(
        &self,
        trees: &[&GitTree],
    ) -> StoreResult<impl Fn(&Transaction<'_>) -> StoreResult<()> + '_> {
        #[cfg(test)]
        self.git_tree_lookup_count.fetch_add(1, Ordering::Relaxed);
        let mut hashes = std::collections::BTreeSet::new();
        let mut names = BTreeMap::new();
        let mut entries = Vec::new();
        let mut tree_keys = Vec::with_capacity(trees.len());
        let mut tree_names = HashSet::new();
        for tree in trees {
            hashes.insert(tree.id);
            tree_names.clear();
            for entry in &tree.entries {
                if entry.name.is_empty()
                    || entry.name.contains('/')
                    || entry.name == "."
                    || entry.name == ".."
                {
                    return Err(StoreError::Invalid(
                        "Git tree entry name is invalid".to_owned(),
                    ));
                }
                hashes.insert(entry.target);
                tree_names.insert(entry.name.as_str());
                let name_hash = *names
                    .entry(entry.name.clone())
                    .or_insert_with(|| HashKey::sha256(entry.name.as_bytes()));
                let kind = match entry.mode {
                    0o040000 => 1,
                    0o160000 => 3,
                    _ => 2,
                };
                entries.push(serde_json::json!([
                    tree.id.algorithm,
                    tree.id.lanes,
                    entry.name,
                    entry.mode,
                    kind,
                    entry.target.algorithm,
                    entry.target.lanes,
                    name_hash.lanes
                ]));
            }
            tree_keys.push((tree.id.algorithm, tree.id.lanes, tree_names.len()));
        }
        let encode = |value: serde_json::Value| {
            serde_json::to_string(&value).map_err(|error| StoreError::Invalid(error.to_string()))
        };
        let hashes = encode(serde_json::json!(
            hashes
                .into_iter()
                .map(|h| (h.algorithm, h.lanes))
                .collect::<Vec<_>>()
        ))?;
        let tree_keys = encode(serde_json::json!(tree_keys))?;
        let entries = encode(serde_json::Value::Array(entries))?;
        #[cfg(test)]
        let tree_count = trees.len();
        Ok(move |tx: &Transaction<'_>| {
            #[cfg(test)]
            self.git_tree_write_count
                .fetch_add(tree_count, Ordering::Relaxed);
            tx.prepare_cached("WITH input AS (SELECT json_extract(value,'$[0]') a,json_extract(value,'$[1][0]') l0,json_extract(value,'$[1][1]') l1,json_extract(value,'$[1][2]') l2,json_extract(value,'$[1][3]') l3 FROM json_each(?1)) INSERT INTO hashes(algorithm_id,lane0,lane1,lane2,lane3) SELECT a,l0,l1,l2,l3 FROM input i WHERE NOT EXISTS (SELECT 1 FROM hashes h WHERE h.algorithm_id=i.a AND h.lane0=i.l0 AND h.lane1=i.l1 AND h.lane2=i.l2 AND h.lane3=i.l3)")?.execute([&hashes])?;
            for (name, hash) in &names {
                intern_hashed_string(tx, STRING_PATH_COMPONENT, name, *hash)?;
            }
            tx.prepare_cached("INSERT INTO git_trees(tree_hash_id,complete,entry_count) SELECT h.hash_id,1,json_extract(i.value,'$[2]') FROM json_each(?1) i CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=json_extract(i.value,'$[0]') AND h.lane0=json_extract(i.value,'$[1][0]') AND h.lane1=json_extract(i.value,'$[1][1]') AND h.lane2=json_extract(i.value,'$[1][2]') AND h.lane3=json_extract(i.value,'$[1][3]') WHERE 1 ON CONFLICT(tree_hash_id) DO UPDATE SET complete=1,entry_count=excluded.entry_count")?.execute([&tree_keys])?;
            tx.prepare_cached("INSERT INTO git_tree_entries(tree_id,name_string_id,mode_id,object_kind_id,target_hash_id) SELECT t.tree_id,s.string_id,json_extract(i.value,'$[3]'),json_extract(i.value,'$[4]'),d.hash_id FROM json_each(?1) i CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=json_extract(i.value,'$[0]') AND h.lane0=json_extract(i.value,'$[1][0]') AND h.lane1=json_extract(i.value,'$[1][1]') AND h.lane2=json_extract(i.value,'$[1][2]') AND h.lane3=json_extract(i.value,'$[1][3]') JOIN git_trees t ON t.tree_hash_id=h.hash_id CROSS JOIN hashes nh INDEXED BY hashes_by_prefix ON nh.algorithm_id=?3 AND nh.lane0=json_extract(i.value,'$[7][0]') AND nh.lane1=json_extract(i.value,'$[7][1]') AND nh.lane2=json_extract(i.value,'$[7][2]') AND nh.lane3=json_extract(i.value,'$[7][3]') CROSS JOIN strings s INDEXED BY strings_by_hash ON s.domain_id=?2 AND s.content_hash_id=nh.hash_id AND s.value=json_extract(i.value,'$[2]') CROSS JOIN hashes d INDEXED BY hashes_by_prefix ON d.algorithm_id=json_extract(i.value,'$[5]') AND d.lane0=json_extract(i.value,'$[6][0]') AND d.lane1=json_extract(i.value,'$[6][1]') AND d.lane2=json_extract(i.value,'$[6][2]') AND d.lane3=json_extract(i.value,'$[6][3]') WHERE 1 ON CONFLICT(tree_id,name_string_id) DO UPDATE SET mode_id=excluded.mode_id,object_kind_id=excluded.object_kind_id,target_hash_id=excluded.target_hash_id")?.execute(params![entries,STRING_PATH_COMPONENT,HASH_SHA256])?;
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn put_git_tree_entry(
        &self,
        tree: HashKey,
        name: &str,
        mode_id: i64,
        object_kind_id: i64,
        target: HashKey,
    ) -> StoreResult<()> {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err(StoreError::Invalid(
                "Git tree entry name is invalid".to_owned(),
            ));
        }
        self.immediate_transaction(|transaction| {
            let tree_hash_id = intern_hash(transaction, tree)?;
            transaction.execute(
                "INSERT INTO git_trees(tree_hash_id) VALUES (?1) ON CONFLICT(tree_hash_id) DO NOTHING",
                [tree_hash_id],
            )?;
            let tree_id: i64 = transaction.query_row(
                "SELECT tree_id FROM git_trees WHERE tree_hash_id=?1",
                [tree_hash_id],
                |row| row.get(0),
            )?;
            let name_string_id = intern_string(transaction, STRING_PATH_COMPONENT, name)?;
            let target_hash_id = intern_hash(transaction, target)?;
            transaction.execute(
                "INSERT INTO git_tree_entries(tree_id,name_string_id,mode_id,object_kind_id,target_hash_id) VALUES (?1,?2,?3,?4,?5)\
                 ON CONFLICT(tree_id,name_string_id) DO UPDATE SET mode_id=excluded.mode_id,object_kind_id=excluded.object_kind_id,target_hash_id=excluded.target_hash_id",
                params![tree_id, name_string_id, mode_id, object_kind_id, target_hash_id],
            )?;
            Ok(())
        })
    }

    /// Records a snapshot over an already-published root tree; a missing or partial root is
    /// rejected rather than stored as an empty tree that publication would later skip.
    #[cfg(test)]
    pub fn put_snapshot(
        &self,
        root_tree: HashKey,
        scanner_version: i64,
        grammar_set: HashKey,
        tape_format_version: i64,
    ) -> StoreResult<SnapshotId> {
        self.immediate_transaction(|transaction| {
            insert_snapshot(
                transaction,
                root_tree,
                scanner_version,
                grammar_set,
                tape_format_version,
            )?
            .ok_or_else(|| {
                StoreError::Invalid("snapshot root Git tree is not published".to_owned())
            })
        })
    }

    pub fn put_snapshot_file(
        &self,
        snapshot_id: SnapshotId,
        path: &str,
        source_id: SourceId,
        tape_id: Option<TapeId>,
    ) -> StoreResult<()> {
        validate_relative_path(path)?;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let path_id = intern_path(&transaction, path)?;
        transaction.execute(
            "INSERT INTO snapshot_files(snapshot_id,path_id,source_id,tape_id) VALUES (?1,?2,?3,?4)\
             ON CONFLICT(snapshot_id,path_id) DO UPDATE SET source_id=excluded.source_id,tape_id=excluded.tape_id",
            params![snapshot_id.0, path_id, source_id.0, tape_id.map(|id| id.0)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn publish_snapshot_file(
        &self,
        snapshot_id: SnapshotId,
        publication: SnapshotFilePublication<'_>,
    ) -> StoreResult<StoredFile> {
        self.publish_snapshot_files(snapshot_id, &[publication])?
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Invalid("snapshot publication is empty".to_owned()))
    }

    pub fn publish_snapshot_files(
        &self,
        snapshot_id: SnapshotId,
        publications: &[SnapshotFilePublication<'_>],
    ) -> StoreResult<Vec<StoredFile>> {
        self.require_write_capacity()?;
        if publications.is_empty() || publications.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(format!(
                "snapshot publication batch must contain 1 to {MAX_PUBLICATION_BATCH} files"
            )));
        }
        let mut prepared = Vec::with_capacity(publications.len());
        for publication in publications {
            validate_relative_path(publication.path)?;
            validate_tape_payload(publication.tape)?;
            let source_frame = compress(publication.source, self.compression_level)?;
            let tape_frame = publication
                .tape
                .map(|payload| compress(payload.bytes, self.compression_level))
                .transpose()?;
            prepared.push((publication, source_frame, tape_frame));
        }
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = Vec::with_capacity(prepared.len());
        for (publication, source_frame, tape_frame) in prepared {
            let source_id = put_source_tx(
                &transaction,
                publication.source,
                &source_frame,
                publication.aliases,
            )?;
            let tape_id = match (publication.tape, tape_frame.as_deref()) {
                (Some(payload), Some(frame)) => {
                    Some(put_tape_tx(&transaction, source_id, payload, frame)?)
                }
                (None, None) => None,
                _ => unreachable!("tape frame follows tape payload"),
            };
            let path_id = intern_path(&transaction, publication.path)?;
            transaction.execute(
                "INSERT INTO snapshot_files(snapshot_id,path_id,source_id,tape_id) VALUES (?1,?2,?3,?4)\
                 ON CONFLICT(snapshot_id,path_id) DO UPDATE SET source_id=excluded.source_id,tape_id=excluded.tape_id",
                params![snapshot_id.0, path_id, source_id.0, tape_id.map(|id| id.0)],
            )?;
            stored.push(StoredFile {
                source: source_id,
                tape: tape_id,
            });
        }
        transaction.commit()?;
        Ok(stored)
    }
}

/// Records the snapshot over `root_tree` while that tree is complete; `None` when it is not.
fn insert_snapshot(
    transaction: &Transaction<'_>,
    root_tree: HashKey,
    scanner_version: i64,
    grammar_set: HashKey,
    tape_format_version: i64,
) -> StoreResult<Option<SnapshotId>> {
    let Some(tree_id) = transaction
        .query_row(
            concat!(
                "SELECT t.tree_id FROM hashes h INDEXED BY hashes_by_prefix \
                 JOIN git_trees t ON t.tree_hash_id=h.hash_id AND ",
                complete_git_tree!(),
                " WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5"
            ),
            hash_params(root_tree),
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let grammar_hash_id = intern_hash(transaction, grammar_set)?;
    transaction.execute(
        "INSERT INTO snapshots(root_tree_id,scanner_version,grammar_set_hash_id,tape_format_version)\
         VALUES (?1,?2,?3,?4) ON CONFLICT(root_tree_id,scanner_version,grammar_set_hash_id,tape_format_version) DO NOTHING",
        params![tree_id, scanner_version, grammar_hash_id, tape_format_version],
    )?;
    let snapshot_id = transaction.query_row(
        "SELECT snapshot_id FROM snapshots WHERE root_tree_id=?1 AND scanner_version=?2 AND grammar_set_hash_id=?3 AND tape_format_version=?4",
        params![tree_id, scanner_version, grammar_hash_id, tape_format_version],
        |row| row.get(0),
    )?;
    Ok(Some(SnapshotId(snapshot_id)))
}
