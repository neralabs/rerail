DROP TABLE anchor_postings;
DROP TABLE anchor_predicates;
DROP TABLE anchor_index_sets;
CREATE TABLE search_index_sets (
    tape_id INTEGER PRIMARY KEY REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    node_count INTEGER NOT NULL CHECK(node_count > 0),
    complete INTEGER NOT NULL CHECK(complete IN (0,1))
) STRICT;
CREATE VIRTUAL TABLE search_kind_fts USING fts5(
    token, content='', contentless_delete=1, detail=full, tokenize='ascii'
);
CREATE VIRTUAL TABLE search_value_fts USING fts5(
    token, content='', contentless_delete=1, detail=none, tokenize='ascii'
);
CREATE VIRTUAL TABLE search_text_fts USING fts5(
    token, content='', contentless_delete=1, detail=none,
    tokenize='unicode61 remove_diacritics 0'
);
CREATE VIRTUAL TABLE search_kind_instances USING fts5vocab(search_kind_fts, 'instance');
CREATE TRIGGER delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
    DELETE FROM search_value_fts WHERE rowid=OLD.tape_id;
    DELETE FROM search_text_fts WHERE rowid=OLD.tape_id;
END;
"#;

#[cfg(test)]
impl TapeId {
    pub(crate) const fn for_benchmark(value: i64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TapeSearchIndex {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub node_count: u32,
    pub kind_tokens: Arc<String>,
    pub value_tokens: Arc<String>,
    pub source_text: Arc<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchAnchorLookup {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub kind_token: String,
    pub value_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StoredAnchorSelection {
    Unknown,
    Candidate,
    Roots(Vec<u32>),
}


impl ContentStore {
    /// Publish one query-independent FTS document per immutable tape. The kind stream contains one
    /// token per preorder node. The value stream contains distinct bounded values for file filtering.
    /// All string construction and validation finish before the short write transaction starts.
    pub(crate) fn put_search_indexes_bulk(&self, indexes: &[TapeSearchIndex]) -> StoreResult<()> {
        self.require_write_capacity()?;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::put_search_indexes_transaction(&transaction, indexes)?;
        transaction.commit()?;
        Ok(())
    }

    fn put_search_indexes_transaction(
        transaction: &Transaction<'_>,
        indexes: &[TapeSearchIndex],
    ) -> StoreResult<()> {
        if indexes.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(
                "search-index publication exceeds 64 tapes".to_owned(),
            ));
        }
        for index in indexes {
            let valid_tokens = |stream: &str| {
                stream
                    .split_ascii_whitespace()
                    .all(|token| token.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            };
            if index.index_version < 0
                || index.node_count == 0
                || index.kind_tokens.split_ascii_whitespace().count() != index.node_count as usize
                || !valid_tokens(&index.kind_tokens)
                || !valid_tokens(&index.value_tokens)
            {
                return Err(StoreError::Invalid(
                    "invalid positional FTS search document".to_owned(),
                ));
            }
        }
        let mut unique = BTreeMap::new();
        for index in indexes {
            if let Some(previous) = unique.insert(index.tape_id.0, index)
                && previous != index
            {
                return Err(StoreError::Invalid(
                    "conflicting search indexes for one immutable tape".to_owned(),
                ));
            }
        }
        let ids: Array = Rc::new(unique.keys().copied().map(SqlValue::Integer).collect());
        let mut existing_statement = transaction.prepare_cached(
            "SELECT sets.tape_id,sets.index_version,sets.node_count \
             FROM rarray(?1) requested JOIN search_index_sets sets ON sets.tape_id=requested.value",
        )?;
        let existing = existing_statement
            .query_map([ids], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    (row.get::<_, i64>(1)?, row.get::<_, u32>(2)?),
                ))
            })?
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        drop(existing_statement);
        let mut rebuild = BTreeSet::new();
        for (tape_id, (index_version, node_count)) in &existing {
            let index = unique[tape_id];
            if index.index_version != *index_version {
                rebuild.insert(*tape_id);
            } else if index.node_count != *node_count {
                return Err(StoreError::Invalid(
                    "stored search index conflicts with immutable tape index".to_owned(),
                ));
            }
        }
        if !rebuild.is_empty() {
            let ids: Array = Rc::new(rebuild.iter().copied().map(SqlValue::Integer).collect());
            transaction.execute(
                "DELETE FROM search_kind_fts WHERE rowid IN (SELECT value FROM rarray(?1))",
                [ids.clone()],
            )?;
            transaction.execute(
                "DELETE FROM search_value_fts WHERE rowid IN (SELECT value FROM rarray(?1))",
                [ids.clone()],
            )?;
            transaction.execute(
                "DELETE FROM search_text_fts WHERE rowid IN (SELECT value FROM rarray(?1))",
                [ids.clone()],
            )?;
            transaction.execute(
                "DELETE FROM search_index_sets WHERE tape_id IN (SELECT value FROM rarray(?1))",
                [ids],
            )?;
        }
        let missing = unique
            .into_iter()
            .filter_map(|(tape_id, index)| {
                (!existing.contains_key(&tape_id) || rebuild.contains(&tape_id)).then_some(index)
            })
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        let documents: Array = Rc::new(
            missing
                .iter()
                .map(|index| {
                    let mut packed = format!(
                        "{:020}{:010}{:010}",
                        index.tape_id.0,
                        index.kind_tokens.len(),
                        index.value_tokens.len()
                    )
                    .into_bytes();
                    packed.reserve(
                        index.kind_tokens.len()
                            + index.value_tokens.len()
                            + index.source_text.len(),
                    );
                    packed.extend_from_slice(index.kind_tokens.as_bytes());
                    packed.extend_from_slice(index.value_tokens.as_bytes());
                    packed.extend_from_slice(index.source_text.as_bytes());
                    SqlValue::Blob(packed)
                })
                .collect(),
        );
        // The three stable statements scan one shared packed BLOB array. Tape objects and their
        // indexes are immutable, so existing rows are retained instead of deleted/reindexed.
        transaction.execute(
            "INSERT INTO search_kind_fts(rowid,token) \
             SELECT CAST(substr(value,1,20) AS INTEGER), \
                    CAST(substr(value,41,CAST(substr(value,21,10) AS INTEGER)) AS TEXT) \
             FROM rarray(?1)",
            [documents.clone()],
        )?;
        transaction.execute(
            "INSERT INTO search_value_fts(rowid,token) \
             SELECT CAST(substr(value,1,20) AS INTEGER), \
                    CAST(substr(value,41+CAST(substr(value,21,10) AS INTEGER), \
                                CAST(substr(value,31,10) AS INTEGER)) AS TEXT) \
             FROM rarray(?1)",
            [documents.clone()],
        )?;
        transaction.execute(
            "INSERT INTO search_text_fts(rowid,token) \
             SELECT CAST(substr(value,1,20) AS INTEGER), \
                    CAST(substr(value,41+CAST(substr(value,21,10) AS INTEGER) \
                                      +CAST(substr(value,31,10) AS INTEGER)) AS TEXT) \
             FROM rarray(?1)",
            [documents],
        )?;
        let index_sets: Array = Rc::new(
            missing
                .iter()
                .map(|index| {
                    SqlValue::Blob(
                        format!(
                            "{:020}{:020}{:010}",
                            index.tape_id.0, index.index_version, index.node_count
                        )
                        .into_bytes(),
                    )
                })
                .collect(),
        );
        transaction.execute(
            "INSERT INTO search_index_sets(tape_id,index_version,node_count,complete) \
             SELECT CAST(substr(value,1,20) AS INTEGER), \
                    CAST(substr(value,21,20) AS INTEGER), \
                    CAST(substr(value,41,10) AS INTEGER),1 FROM rarray(?1)",
            [index_sets],
        )?;
        Ok(())
    }

    /// Resolve all requested tape/predicate pairs with one SQL statement. Kind queries return
    /// preorder positions. Exact-value queries return file candidates for terminal confirmation.
    /// Missing index sets remain unknown. A complete noncandidate returns an empty root list.
    pub(crate) fn get_search_anchors_bulk(
        &self,
        lookups: &[SearchAnchorLookup],
    ) -> StoreResult<Vec<StoredAnchorSelection>> {
        let requests: Array = Rc::new(
            lookups
                .iter()
                .enumerate()
                .map(|(position, lookup)| {
                    let value = lookup.value_token.as_deref().unwrap_or_default();
                    let mut packed = format!(
                        "{position:010}{:020}{:020}{:010}{:010}",
                        lookup.tape_id.0,
                        lookup.index_version,
                        lookup.kind_token.len(),
                        value.len()
                    )
                    .into_bytes();
                    packed.extend_from_slice(lookup.kind_token.as_bytes());
                    packed.extend_from_slice(value.as_bytes());
                    SqlValue::Blob(packed)
                })
                .collect(),
        );
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH requested AS MATERIALIZED (SELECT \
               CAST(substr(value,1,10) AS INTEGER) position, \
               CAST(substr(value,11,20) AS INTEGER) tape_id, \
               CAST(substr(value,31,20) AS INTEGER) version, \
               CAST(substr(value,71,CAST(substr(value,51,10) AS INTEGER)) AS TEXT) kind_token, \
               CASE CAST(substr(value,61,10) AS INTEGER) WHEN 0 THEN NULL ELSE \
                 CAST(substr(value,71+CAST(substr(value,51,10) AS INTEGER), \
                                   CAST(substr(value,61,10) AS INTEGER)) AS TEXT) END value_token \
               FROM rarray(?1)), \
             wanted_kinds AS (SELECT DISTINCT kind_token FROM requested WHERE value_token IS NULL), \
             kind_hits AS MATERIALIZED (SELECT w.kind_token,k.doc,k.offset FROM wanted_kinds w \
               JOIN search_kind_instances k ON k.term=w.kind_token), \
             wanted_values AS (SELECT DISTINCT value_token FROM requested WHERE value_token IS NOT NULL), \
             value_files AS MATERIALIZED (SELECT w.value_token,v.rowid doc FROM wanted_values w \
               JOIN search_value_fts v ON search_value_fts MATCH w.value_token), \
             roots AS (SELECT r.position,k.offset FROM requested r JOIN kind_hits k \
               ON r.value_token IS NULL AND k.doc=r.tape_id AND k.kind_token=r.kind_token) \
             SELECT r.position,s.complete,s.node_count,roots.offset, \
                    r.value_token IS NOT NULL exact_value,v.doc IS NOT NULL value_candidate \
             FROM requested r LEFT JOIN search_index_sets s ON s.tape_id=r.tape_id \
                 AND s.index_version=r.version \
               LEFT JOIN roots ON roots.position=r.position \
               LEFT JOIN value_files v ON r.value_token IS NOT NULL \
                 AND v.doc=r.tape_id AND v.value_token=r.value_token \
             ORDER BY r.position,roots.offset",
        )?;
        let rows = statement
            .query_map([requests], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<bool>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut selections = vec![StoredAnchorSelection::Unknown; lookups.len()];
        for (position, complete, node_count, root, exact_value, value_candidate) in rows {
            let position = usize::try_from(position)
                .map_err(|_| StoreError::Corrupt("invalid search request position".to_owned()))?;
            if complete != Some(true) {
                continue;
            }
            let node_count = u32::try_from(node_count.ok_or_else(|| {
                StoreError::Corrupt("complete search index has no node count".to_owned())
            })?)
            .map_err(|_| StoreError::Corrupt("invalid search-index node count".to_owned()))?;
            if matches!(selections[position], StoredAnchorSelection::Unknown) {
                selections[position] = if exact_value && value_candidate {
                    StoredAnchorSelection::Candidate
                } else {
                    StoredAnchorSelection::Roots(Vec::new())
                };
            }
            if let Some(root) = root {
                let root = u32::try_from(root)
                    .map_err(|_| StoreError::Corrupt("invalid FTS preorder offset".to_owned()))?;
                if root >= node_count {
                    return Err(StoreError::Corrupt(
                        "FTS preorder offset exceeds tape".to_owned(),
                    ));
                }
                if let StoredAnchorSelection::Roots(roots) = &mut selections[position] {
                    roots.push(root);
                }
            }
        }
        Ok(selections)
    }

    /// Select exact-value file candidates without returning the file-by-predicate Cartesian product.
    /// The statement returns one status row per tape and one sparse row per positive pair.
    pub(crate) fn get_exact_value_candidates_bulk(
        &self,
        tape_ids: &[TapeId],
        index_version: i64,
        value_tokens: &[String],
    ) -> StoreResult<Vec<Vec<Option<bool>>>> {
        if value_tokens.is_empty() {
            return Ok(tape_ids.iter().map(|_| Vec::new()).collect());
        }
        let tapes: Array = Rc::new(
            tape_ids
                .iter()
                .enumerate()
                .map(|(position, tape)| {
                    SqlValue::Blob(format!("{position:010}{:020}", tape.0).into_bytes())
                })
                .collect(),
        );
        let predicates: Array = Rc::new(
            value_tokens
                .iter()
                .enumerate()
                .map(|(position, token)| {
                    let mut packed = format!("{position:010}").into_bytes();
                    packed.extend_from_slice(token.as_bytes());
                    SqlValue::Blob(packed)
                })
                .collect(),
        );
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH tapes AS MATERIALIZED (SELECT CAST(substr(value,1,10) AS INTEGER) position, \
               CAST(substr(value,11,20) AS INTEGER) tape_id FROM rarray(?1)), \
             predicates AS MATERIALIZED (SELECT CAST(substr(value,1,10) AS INTEGER) position, \
               CAST(substr(value,11) AS TEXT) token FROM rarray(?2)), \
             complete_tapes AS MATERIALIZED (SELECT t.position,t.tape_id,s.complete FROM tapes t \
               LEFT JOIN search_index_sets s ON s.tape_id=t.tape_id AND s.index_version=?3), \
             hits AS MATERIALIZED (SELECT p.position program,v.rowid tape_id FROM predicates p \
               JOIN search_value_fts v ON search_value_fts MATCH p.token) \
             SELECT 0 tag,t.position,-1 program,t.complete FROM complete_tapes t \
             UNION ALL SELECT 1,t.position,h.program,1 FROM hits h \
               JOIN complete_tapes t ON t.tape_id=h.tape_id WHERE t.complete=1",
        )?;
        let rows = statement
            .query_map(params![tapes, predicates, index_version], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<bool>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = vec![vec![None; value_tokens.len()]; tape_ids.len()];
        for (tag, file, program, complete) in rows {
            let file = usize::try_from(file)
                .map_err(|_| StoreError::Corrupt("invalid exact-value file position".to_owned()))?;
            if tag == 0 {
                if complete == Some(true) {
                    result[file].fill(Some(false));
                }
                continue;
            }
            let program = usize::try_from(program).map_err(|_| {
                StoreError::Corrupt("invalid exact-value predicate position".to_owned())
            })?;
            result[file][program] = Some(true);
        }
        Ok(result)
    }

    /// Return the live SQLite pages for the search-index tables and FTS shadow tables.
    #[cfg(test)]
    pub(crate) fn search_index_storage_bytes(&self) -> StoreResult<(u64, [u64; 4])> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT CASE WHEN name LIKE 'search_kind_fts%' THEN 1 \
                         WHEN name LIKE 'search_value_fts%' THEN 2 \
                         WHEN name LIKE 'search_text_fts%' THEN 3 ELSE 0 END family, \
                    sum(pgsize) FROM dbstat WHERE name LIKE 'search_%' GROUP BY family",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut parts = [0_u64; 4];
        for (family, bytes) in rows {
            let family = usize::try_from(family)
                .map_err(|_| StoreError::Corrupt("invalid search-index family".to_owned()))?;
            parts[family] = u64::try_from(bytes)
                .map_err(|_| StoreError::Corrupt("invalid search-index storage size".to_owned()))?;
        }
        Ok((parts.iter().sum(), parts))
    }

    /// Use the contentless source-word FTS as a conservative file filter. `Some(false)` is a
    /// complete rejection, `Some(true)` is a candidate, and `None` means that the tape has no
    /// complete search index. Terminal matching must confirm every candidate.
    pub(crate) fn get_text_candidates_bulk(
        &self,
        tape_ids: &[TapeId],
        required_words: &[String],
    ) -> StoreResult<Vec<Option<bool>>> {
        if required_words.is_empty() {
            return Ok(vec![Some(true); tape_ids.len()]);
        }
        let query = required_words
            .iter()
            .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let requests: Array = Rc::new(
            tape_ids
                .iter()
                .enumerate()
                .map(|(position, tape)| {
                    SqlValue::Blob(format!("{position:010}{:020}", tape.0).into_bytes())
                })
                .collect(),
        );
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH requested AS (SELECT CAST(substr(value,1,10) AS INTEGER) position, \
               CAST(substr(value,11,20) AS INTEGER) tape_id FROM rarray(?1)) \
             SELECT r.position,s.complete,f.rowid IS NOT NULL FROM requested r \
               LEFT JOIN search_index_sets s ON s.tape_id=r.tape_id \
               LEFT JOIN search_text_fts f ON f.rowid=r.tape_id AND search_text_fts MATCH ?2 \
             ORDER BY r.position",
        )?;
        let rows = statement
            .query_map(params![requests, query], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<bool>>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = vec![None; tape_ids.len()];
        for (position, complete, matched) in rows {
            let position = usize::try_from(position)
                .map_err(|_| StoreError::Corrupt("invalid text request position".to_owned()))?;
            if complete == Some(true) {
                result[position] = Some(matched);
            }
        }
        Ok(result)
    }
}
