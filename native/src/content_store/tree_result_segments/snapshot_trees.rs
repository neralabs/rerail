// COPY: Tempo ner-1562-scale-test-the-re-railer@8fe9ffed4:tempo/native/src/content_store.rs, fragments verbatim:
// l.253-263 (SCHEMA_V9), l.3082-3159 (reusable_snapshot_tree_prefixes, reusable_maximal_tree_prefixes).
const SCHEMA_V9: &str = r#"
CREATE TABLE completed_snapshot_trees (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (snapshot_id, tree_hash_id, prefix, catalog_hash_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX completed_snapshot_trees_by_identity
ON completed_snapshot_trees(tree_hash_id, prefix, catalog_hash_id);
"#;

    pub fn reusable_snapshot_tree_prefixes(
        &self,
        catalog: CandidateCatalogIdentity,
        trees: &[(String, HashKey)],
    ) -> StoreResult<HashSet<String>> {
        if trees.is_empty() {
            return Ok(HashSet::new());
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
            StoreError::Invalid(format!("Git tree lookup cannot be encoded: {error}"))
        })?;
        let key = catalog.key();
        let connection = self
            .connection
            .lock()
            .expect("content store connection unavailable");
        let mut statement = connection.prepare_cached(
            "WITH input AS (SELECT json_extract(value,'$[0]') prefix, \
             CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, \
             CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
             CAST(json_extract(value,'$[3]') AS INTEGER) lane1, \
             CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
             CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
             SELECT DISTINCT i.prefix FROM input i \
             CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
             AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3 \
             JOIN completed_snapshot_trees c ON c.tree_hash_id=h.hash_id AND c.prefix=i.prefix \
             JOIN hashes ch ON ch.hash_id=c.catalog_hash_id \
             WHERE ch.algorithm_id=?2 AND ch.lane0=?3 AND ch.lane1=?4 AND ch.lane2=?5 AND ch.lane3=?6",
        )?;
        let rows = statement.query_map(
            params![
                encoded,
                key.algorithm,
                key.lanes[0],
                key.lanes[1],
                key.lanes[2],
                key.lanes[3]
            ],
            |row| row.get(0),
        )?;
        rows.collect::<Result<HashSet<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn reusable_maximal_tree_prefixes(
        &self,
        catalog: CandidateCatalogIdentity,
        trees: &[(String, HashKey)],
    ) -> StoreResult<HashSet<String>> {
        let prefixes = self.reusable_snapshot_tree_prefixes(catalog, trees)?;
        Ok(prefixes
            .iter()
            .filter(|prefix| {
                !prefixes.iter().any(|ancestor| {
                    ancestor.len() < prefix.len()
                        && (ancestor.is_empty()
                            || prefix
                                .strip_prefix(ancestor)
                                .is_some_and(|suffix| suffix.starts_with('/')))
                })
            })
            .cloned()
            .collect())
    }
