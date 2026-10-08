//! Persisted local/context check results, their process-local cache, and completion markers.

use super::rows::{hash_params, intern_hash};
use super::{
    CODEC_ZSTD, CandidateCatalogIdentity, CandidateClause, ContentStore, DirectoryReuse, HashKey,
    SourceId, StoreError, StoreResult, StoredLocalResult, WorkspaceCompletion, WorkspaceId,
};
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;

pub(super) struct LocalResultCache {
    byte_limit: usize,
    used_bytes: usize,
    entries: HashMap<(HashKey, HashKey, HashKey), Arc<Vec<StoredLocalResult>>>,
}

impl LocalResultCache {
    pub(super) fn new(byte_limit: usize) -> Self {
        Self {
            byte_limit,
            used_bytes: 0,
            entries: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get(
        &self,
        catalog: HashKey,
        source: HashKey,
        context: HashKey,
    ) -> Option<Arc<Vec<StoredLocalResult>>> {
        self.entries.get(&(catalog, source, context)).cloned()
    }

    fn insert(
        &mut self,
        catalog: HashKey,
        source: HashKey,
        context: HashKey,
        results: Vec<StoredLocalResult>,
    ) -> Arc<Vec<StoredLocalResult>> {
        let bytes = results
            .iter()
            .map(|result| result.bytes.len() + result.candidate.clause.len())
            .sum::<usize>();
        if self.used_bytes.saturating_add(bytes) > self.byte_limit {
            self.entries.clear();
            self.used_bytes = 0;
        }
        let results = Arc::new(results);
        if bytes <= self.byte_limit {
            self.used_bytes += bytes;
            self.entries
                .insert((catalog, source, context), Arc::clone(&results));
        }
        results
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.used_bytes = 0;
    }
}

impl ContentStore {
    #[allow(clippy::type_complexity)]
    pub fn local_check_results_by_key(
        &self,
        keys: &[(HashKey, HashKey)],
        catalog: CandidateCatalogIdentity,
    ) -> StoreResult<HashMap<(HashKey, HashKey), Arc<Vec<StoredLocalResult>>>> {
        let catalog_key = catalog.key();
        let mut results = HashMap::<(HashKey, HashKey), Arc<Vec<StoredLocalResult>>>::new();
        let mut missing = Vec::new();
        {
            let cache = self
                .local_result_cache
                .lock()
                .expect("local result cache mutex poisoned");
            for &(source, context) in keys {
                if results.contains_key(&(source, context)) {
                    continue;
                }
                if let Some(cached) = cache.get(catalog_key, source, context) {
                    results.insert((source, context), cached);
                } else {
                    missing.push((source, context));
                }
            }
        }
        if missing.is_empty() {
            return Ok(results);
        }
        let mut loaded = HashMap::<(HashKey, HashKey), Vec<StoredLocalResult>>::new();
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        for chunk in missing.chunks(500) {
            let mut sql = String::from(
                "WITH requested(source_algorithm,source_lane0,source_lane1,source_lane2,source_lane3,\
                 context_algorithm,context_lane0,context_lane1,context_lane2,context_lane3) AS (VALUES ",
            );
            let mut values = Vec::with_capacity(chunk.len() * 10 + 5);
            for (index, (source, context)) in chunk.iter().enumerate() {
                if index != 0 {
                    sql.push(',');
                }
                sql.push_str("(?,?,?,?,?,?,?,?,?,?)");
                values.extend(hash_params(*source).map(rusqlite::types::Value::Integer));
                values.extend(hash_params(*context).map(rusqlite::types::Value::Integer));
            }
            sql.push_str(
                ") SELECT q.source_algorithm,q.source_lane0,q.source_lane1,q.source_lane2,q.source_lane3,\
                 q.context_algorithm,q.context_lane0,q.context_lane1,q.context_lane2,q.context_lane3,\
                 ph.algorithm_id,ph.lane0,ph.lane1,ph.lane2,ph.lane3,cs.value,\
                 r.codec_id,r.raw_length,r.frame \
                 FROM requested q CROSS JOIN hashes sh ON sh.algorithm_id=q.source_algorithm \
                 AND sh.lane0=q.source_lane0 AND sh.lane1=q.source_lane1 AND sh.lane2=q.source_lane2 \
                 AND sh.lane3=q.source_lane3 CROSS JOIN source_objects so ON so.canonical_hash_id=sh.hash_id \
                 CROSS JOIN hashes ch ON ch.algorithm_id=q.context_algorithm AND ch.lane0=q.context_lane0 \
                 AND ch.lane1=q.context_lane1 AND ch.lane2=q.context_lane2 AND ch.lane3=q.context_lane3 \
                 CROSS JOIN context_check_results r ON r.source_id=so.source_id AND r.context_hash_id=ch.hash_id \
                 JOIN candidate_checks c ON c.check_id=r.check_id \
                 JOIN candidate_catalogs a ON a.catalog_id=c.catalog_id \
                 JOIN hashes ah ON ah.hash_id=a.identity_hash_id \
                 JOIN hashes ph ON ph.hash_id=c.policy_hash_id \
                 JOIN strings cs ON cs.string_id=c.check_string_id \
                 WHERE ah.algorithm_id=? AND ah.lane0=? AND ah.lane1=? AND ah.lane2=? AND ah.lane3=? \
                 ORDER BY so.source_id,r.context_hash_id,c.check_id",
            );
            values.extend(hash_params(catalog_key).map(rusqlite::types::Value::Integer));
            let mut statement = connection.prepare(&sql)?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(values), |row| {
                    Ok((
                        (
                            HashKey {
                                algorithm: row.get(0)?,
                                lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                            },
                            HashKey {
                                algorithm: row.get(5)?,
                                lanes: [row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?],
                            },
                        ),
                        CandidateClause {
                            rule: HashKey {
                                algorithm: row.get(10)?,
                                lanes: [row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?],
                            },
                            clause: row.get(15)?,
                        },
                        row.get::<_, i64>(16)?,
                        row.get::<_, i64>(17)?,
                        row.get::<_, Vec<u8>>(18)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (key, candidate, codec, raw_length, frame) in rows {
                let bytes = decode_local_result(codec, raw_length, frame)?;
                loaded
                    .entry(key)
                    .or_default()
                    .push(StoredLocalResult { candidate, bytes });
            }
        }
        drop(connection);
        let mut cache = self
            .local_result_cache
            .lock()
            .expect("local result cache mutex poisoned");
        for ((source, context), loaded) in loaded {
            let loaded = cache.insert(catalog_key, source, context, loaded);
            results.insert((source, context), loaded);
        }
        Ok(results)
    }

    pub fn local_check_results(
        &self,
        source_id: SourceId,
        context: HashKey,
        catalog: CandidateCatalogIdentity,
    ) -> StoreResult<Vec<StoredLocalResult>> {
        let rows = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            let mut statement = connection.prepare(
                "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,s.value,\
                 r.codec_id,r.raw_length,r.frame \
                 FROM context_check_results r \
                 JOIN hashes ch ON ch.hash_id=r.context_hash_id \
                 JOIN candidate_checks c ON c.check_id=r.check_id \
                 JOIN candidate_catalogs a ON a.catalog_id=c.catalog_id \
                 JOIN hashes ah ON ah.hash_id=a.identity_hash_id \
                 JOIN hashes h ON h.hash_id=c.policy_hash_id \
                 JOIN strings s ON s.string_id=c.check_string_id \
                 WHERE r.source_id=?1 AND ch.algorithm_id=?2 AND ch.lane0=?3 \
                 AND ch.lane1=?4 AND ch.lane2=?5 AND ch.lane3=?6 \
                 AND ah.algorithm_id=?7 AND ah.lane0=?8 AND ah.lane1=?9 \
                 AND ah.lane2=?10 AND ah.lane3=?11 ORDER BY c.check_id",
            )?;
            statement
                .query_map(
                    params![
                        source_id.0,
                        context.algorithm,
                        context.lanes[0],
                        context.lanes[1],
                        context.lanes[2],
                        context.lanes[3],
                        catalog.key().algorithm,
                        catalog.key().lanes[0],
                        catalog.key().lanes[1],
                        catalog.key().lanes[2],
                        catalog.key().lanes[3]
                    ],
                    |row| {
                        Ok((
                            CandidateClause {
                                rule: HashKey {
                                    algorithm: row.get(0)?,
                                    lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                                },
                                clause: row.get(5)?,
                            },
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, Vec<u8>>(8)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        rows.into_iter()
            .map(|(candidate, codec, raw_length, frame)| {
                Ok(StoredLocalResult {
                    candidate,
                    bytes: decode_local_result(codec, raw_length, frame)?,
                })
            })
            .collect()
    }

    pub fn complete_workspace_revision(
        &self,
        workspace_id: WorkspaceId,
        revision: i64,
        catalog: CandidateCatalogIdentity,
        completed_at: i64,
    ) -> StoreResult<()> {
        if revision <= 0 || completed_at <= 0 {
            return Err(StoreError::Invalid(
                "completed workspace revision is invalid".to_owned(),
            ));
        }
        self.immediate_transaction(|transaction| {
            let catalog_hash_id = intern_hash(transaction, catalog.key())?;
            transaction.execute(
                "INSERT INTO completed_workspace_revisions(workspace_id,revision,catalog_hash_id,completed_at) \
                 VALUES (?1,?2,?3,?4) ON CONFLICT(workspace_id) DO UPDATE SET \
                 revision=excluded.revision,catalog_hash_id=excluded.catalog_hash_id,completed_at=excluded.completed_at",
                params![workspace_id.0, revision, catalog_hash_id, completed_at],
            )?;
            Ok(())
        })
    }

    pub fn complete_directories(
        &self,
        completion: WorkspaceCompletion,
        catalog: CandidateCatalogIdentity,
        directories: &[DirectoryReuse],
    ) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            let catalog_hash_id = intern_hash(transaction, catalog.key())?;
            for directory in directories {
                let tree_hash_id = intern_hash(transaction, directory.tree)?;
                let package_hash_id = intern_hash(transaction, directory.package_markers)?;
                transaction.execute(
                    "INSERT OR IGNORE INTO completed_directory_reuse(tree_hash_id,prefix,package_hash_id,catalog_hash_id) VALUES(?1,?2,?3,?4)",
                    params![tree_hash_id, directory.prefix, package_hash_id, catalog_hash_id],
                )?;
            }
            transaction.execute(
                "INSERT INTO completed_workspace_revisions(workspace_id,revision,catalog_hash_id,completed_at) \
                 VALUES (?1,?2,?3,?4) ON CONFLICT(workspace_id) DO UPDATE SET \
                 revision=excluded.revision,catalog_hash_id=excluded.catalog_hash_id,completed_at=excluded.completed_at",
                params![completion.workspace_id.0, completion.revision, catalog_hash_id, completion.completed_at],
            )?;
            Ok(())
        })
    }

    pub fn reusable_directory_prefixes(
        &self,
        catalog: CandidateCatalogIdentity,
        directories: &[DirectoryReuse],
    ) -> StoreResult<HashSet<String>> {
        if directories.is_empty() {
            return Ok(HashSet::new());
        }
        let key = catalog.key();
        let connection = self
            .connection
            .lock()
            .expect("content store connection unavailable");
        let mut prefixes = HashSet::new();
        let mut statement = connection.prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM completed_directory_reuse d \
             JOIN hashes t ON t.hash_id=d.tree_hash_id JOIN hashes p ON p.hash_id=d.package_hash_id \
             JOIN hashes c ON c.hash_id=d.catalog_hash_id \
             WHERE d.prefix=?1 AND t.algorithm_id=?2 AND t.lane0=?3 AND t.lane1=?4 AND t.lane2=?5 AND t.lane3=?6 \
             AND p.algorithm_id=?7 AND p.lane0=?8 AND p.lane1=?9 AND p.lane2=?10 AND p.lane3=?11 \
             AND c.algorithm_id=?12 AND c.lane0=?13 AND c.lane1=?14 AND c.lane2=?15 AND c.lane3=?16)",
        )?;
        for directory in directories {
            let found: bool = statement.query_row(
                params![
                    directory.prefix,
                    directory.tree.algorithm,
                    directory.tree.lanes[0],
                    directory.tree.lanes[1],
                    directory.tree.lanes[2],
                    directory.tree.lanes[3],
                    directory.package_markers.algorithm,
                    directory.package_markers.lanes[0],
                    directory.package_markers.lanes[1],
                    directory.package_markers.lanes[2],
                    directory.package_markers.lanes[3],
                    key.algorithm,
                    key.lanes[0],
                    key.lanes[1],
                    key.lanes[2],
                    key.lanes[3],
                ],
                |row| row.get(0),
            )?;
            if found {
                prefixes.insert(directory.prefix.clone());
            }
        }
        Ok(prefixes)
    }

    pub fn completed_workspace_revision(
        &self,
        workspace_id: WorkspaceId,
    ) -> StoreResult<Option<i64>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row(
                "SELECT revision FROM completed_workspace_revisions WHERE workspace_id=?1",
                [workspace_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }
}

fn decode_local_result(codec: i64, raw_length: i64, frame: Vec<u8>) -> StoreResult<Vec<u8>> {
    if codec != CODEC_ZSTD {
        return Err(StoreError::Corrupt(format!(
            "unsupported local result codec {codec}"
        )));
    }
    let bytes = zstd::stream::decode_all(Cursor::new(frame))?;
    if bytes.len() != usize::try_from(raw_length).unwrap_or(usize::MAX) {
        return Err(StoreError::Corrupt(
            "local result length does not match metadata".to_owned(),
        ));
    }
    Ok(bytes)
}
