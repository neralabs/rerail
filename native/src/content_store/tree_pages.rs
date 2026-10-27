// COPY: Tempo ner-1562-scale-test-the-re-railer@8fe9ffed4:tempo/native/src/content_store.rs, fragments verbatim:
// l.265-279 (SCHEMA_V10), l.523-542 (segment structs), l.2962-3071 (complete_snapshot_trees),
// l.3161-3252 (tree_result_segments), l.3439-3446 (collection).
const SCHEMA_V10: &str = r#"
CREATE TABLE tree_result_segments (
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    page_index INTEGER NOT NULL CHECK(page_index >= 0),
    page_count INTEGER NOT NULL CHECK(page_count > 0),
    raw_length INTEGER NOT NULL CHECK(raw_length >= 0),
    file_count INTEGER NOT NULL CHECK(file_count >= 0 AND file_count <= 64),
    frame BLOB NOT NULL,
    PRIMARY KEY (tree_hash_id, prefix, catalog_hash_id, page_index)
) STRICT, WITHOUT ROWID;
CREATE INDEX tree_result_segments_by_catalog
ON tree_result_segments(catalog_hash_id, prefix, tree_hash_id);
"#;

#[derive(Clone, Debug)]
pub struct TreeResultSegmentPublication {
    pub prefix: String,
    pub tree: HashKey,
    pub page_index: usize,
    pub page_count: usize,
    pub raw_length: usize,
    pub file_count: usize,
    pub frame: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct StoredTreeResultSegment {
    pub prefix: String,
    pub page_index: usize,
    pub page_count: usize,
    pub file_count: usize,
    pub bytes: Vec<u8>,
}


    pub fn complete_snapshot_trees(
        &self,
        completion: WorkspaceCompletion,
        snapshot_id: SnapshotId,
        catalog: CandidateCatalogIdentity,
        trees: &[(String, HashKey)],
        segments: Vec<TreeResultSegmentPublication>,
    ) -> StoreResult<()> {
        if trees.is_empty() {
            return self.complete_workspace_revision(
                completion.workspace_id,
                completion.revision,
                catalog,
                completion.completed_at,
            );
        }
        let input = trees
            .iter()
            .map(|(prefix, tree)| {
                serde_json::json!([
                    prefix,
                    tree.algorithm,
                    tree.lanes[0],
                    tree.lanes[1],
                    tree.lanes[2],
                    tree.lanes[3]
                ])
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&input).map_err(|error| {
            StoreError::Invalid(format!("completed Git trees cannot be encoded: {error}"))
        })?;
        let mut segment_plans = Vec::new();
        let mut remaining = segments;
        while !remaining.is_empty() {
            let tail = remaining.split_off(remaining.len().min(256));
            let chunk = std::mem::replace(&mut remaining, tail);
            let mut sql = String::from(
                "INSERT INTO tree_result_segments(tree_hash_id,prefix,catalog_hash_id,page_index,page_count,raw_length,file_count,frame) ",
            );
            let mut values = Vec::with_capacity(chunk.len() * 12);
            let mut catalog_indexes = Vec::with_capacity(chunk.len());
            for (index, segment) in chunk.into_iter().enumerate() {
                if index == 0 {
                    sql.push_str("SELECT h.hash_id,?,?,?,?,?,?,? FROM hashes h INDEXED BY hashes_by_prefix WHERE h.algorithm_id=? AND h.lane0=? AND h.lane1=? AND h.lane2=? AND h.lane3=?");
                } else {
                    sql.push_str(" UNION ALL SELECT h.hash_id,?,?,?,?,?,?,? FROM hashes h INDEXED BY hashes_by_prefix WHERE h.algorithm_id=? AND h.lane0=? AND h.lane1=? AND h.lane2=? AND h.lane3=?");
                }
                values.push(rusqlite::types::Value::Text(segment.prefix));
                catalog_indexes.push(values.len());
                values.push(rusqlite::types::Value::Integer(0));
                values.push(rusqlite::types::Value::Integer(
                    segment.page_index.try_into().map_err(|_| {
                        StoreError::Invalid("tree result page index is too large".to_owned())
                    })?,
                ));
                values.push(rusqlite::types::Value::Integer(
                    segment.page_count.try_into().map_err(|_| {
                        StoreError::Invalid("tree result page count is too large".to_owned())
                    })?,
                ));
                values.push(rusqlite::types::Value::Integer(
                    segment.raw_length.try_into().map_err(|_| {
                        StoreError::Invalid("tree result segment is too large".to_owned())
                    })?,
                ));
                values.push(rusqlite::types::Value::Integer(
                    segment.file_count.try_into().map_err(|_| {
                        StoreError::Invalid("tree result file count is too large".to_owned())
                    })?,
                ));
                values.push(rusqlite::types::Value::Blob(segment.frame));
                values.push(rusqlite::types::Value::Integer(segment.tree.algorithm));
                values.extend(segment.tree.lanes.map(rusqlite::types::Value::Integer));
            }
            sql.push_str(" ON CONFLICT(tree_hash_id,prefix,catalog_hash_id,page_index) DO UPDATE SET raw_length=excluded.raw_length \
                WHERE tree_result_segments.page_count=excluded.page_count \
                AND tree_result_segments.raw_length=excluded.raw_length \
                AND tree_result_segments.file_count=excluded.file_count AND tree_result_segments.frame=excluded.frame");
            let expected = catalog_indexes.len();
            segment_plans.push((sql, values, catalog_indexes, expected));
        }
        self.immediate_transaction(|transaction| {
            let catalog_hash_id = intern_hash(transaction, catalog.key())?;
            transaction.prepare_cached(
                "WITH input AS (SELECT json_extract(value,'$[0]') prefix, \
                 CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, \
                 CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
                 CAST(json_extract(value,'$[3]') AS INTEGER) lane1, \
                 CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
                 CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
                 INSERT OR IGNORE INTO completed_snapshot_trees(snapshot_id,tree_hash_id,prefix,catalog_hash_id) \
                 SELECT ?2,h.hash_id,i.prefix,?3 FROM input i \
                 CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
                 AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3",
            )?.execute(params![encoded, snapshot_id.0, catalog_hash_id])?;
            for (sql, values, catalog_indexes, expected) in &mut segment_plans {
                for &index in catalog_indexes.iter() {
                    values[index] = rusqlite::types::Value::Integer(catalog_hash_id);
                }
                let changed = transaction.execute(
                    sql.as_str(),
                    rusqlite::params_from_iter(values.iter()),
                )?;
                if changed != *expected {
                    return Err(StoreError::Corrupt(
                        "tree result segment identity has different bytes".to_owned(),
                    ));
                }
            }

    pub fn tree_result_segments(
        &self,
        catalog: CandidateCatalogIdentity,
        trees: &[(String, HashKey)],
    ) -> StoreResult<Vec<StoredTreeResultSegment>> {
        if trees.is_empty() {
            return Ok(Vec::new());
        }
        let input = trees
            .iter()
            .map(|(prefix, tree)| {
                serde_json::json!([
                    prefix,
                    tree.algorithm,
                    tree.lanes[0],
                    tree.lanes[1],
                    tree.lanes[2],
                    tree.lanes[3]
                ])
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&input).map_err(|error| {
            StoreError::Invalid(format!("tree result lookup cannot be encoded: {error}"))
        })?;
        let key = catalog.key();
        let compressed = {
            let connection = self
                .connection
                .lock()
                .expect("content store connection unavailable");
            let mut statement = connection.prepare_cached(
                "WITH input AS (SELECT json_extract(value,'$[0]') prefix, \
                 CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
                 CAST(json_extract(value,'$[3]') AS INTEGER) lane1, CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
                 CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
                 SELECT i.prefix,r.page_index,r.page_count,r.raw_length,r.file_count,r.frame FROM input i \
                 CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
                 AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3 \
                 JOIN tree_result_segments r ON r.tree_hash_id=h.hash_id AND r.prefix=i.prefix \
                 JOIN hashes ch ON ch.hash_id=r.catalog_hash_id WHERE ch.algorithm_id=?2 AND ch.lane0=?3 \
                 AND ch.lane1=?4 AND ch.lane2=?5 AND ch.lane3=?6 ORDER BY i.prefix,r.page_index",
            )?;
            statement
                .query_map(
                    params![
                        encoded,
                        key.algorithm,
                        key.lanes[0],
                        key.lanes[1],
                        key.lanes[2],
                        key.lanes[3]
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Vec<u8>>(5)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        compressed
            .into_iter()
            .map(
                |(prefix, page_index, page_count, raw_length, file_count, frame)| {
                    let bytes = zstd::stream::decode_all(Cursor::new(frame))?;
                    if bytes.len() != usize::try_from(raw_length).unwrap_or(usize::MAX) {
                        return Err(StoreError::Corrupt(
                            "tree result length mismatch".to_owned(),
                        ));
                    }
                    Ok(StoredTreeResultSegment {
                        prefix,
                        page_index: usize::try_from(page_index).map_err(|_| {
                            StoreError::Corrupt("tree result page index is invalid".to_owned())
                        })?,
                        page_count: usize::try_from(page_count).map_err(|_| {
                            StoreError::Corrupt("tree result page count is invalid".to_owned())
                        })?,
                        file_count: usize::try_from(file_count).map_err(|_| {
                            StoreError::Corrupt("tree result file count is invalid".to_owned())
                        })?,
                        bytes,
                    })
                },
            )
            .collect()
    }

        transaction.execute(
            "DELETE FROM tree_result_segments WHERE (tree_hash_id,prefix,catalog_hash_id) IN (\
             SELECT r.tree_hash_id,r.prefix,r.catalog_hash_id FROM tree_result_segments r \
             WHERE NOT EXISTS (SELECT 1 FROM completed_snapshot_trees c WHERE c.tree_hash_id=r.tree_hash_id \
             AND c.prefix=r.prefix AND c.catalog_hash_id=r.catalog_hash_id) \
             ORDER BY r.catalog_hash_id,r.prefix,r.tree_hash_id LIMIT ?1)",
            [limit],
        )?;
