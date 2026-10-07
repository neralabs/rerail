//! Query-independent tape search index: publication, backlog cursor, and kind presence.

#[cfg(test)]
use super::SEARCH_INDEX_FAMILY_WRITES;
use super::{
    CODEC_ZSTD, ContentStore, HashKey, MAX_PUBLICATION_BATCH, SearchKindLookup, StoreError,
    StoreResult, StoredKindPresence, TapeId, TapeSearchIndex, UnindexedTape,
};
use rusqlite::{TransactionBehavior, params};
use std::io::Cursor;
#[cfg(test)]
use std::sync::atomic::Ordering;

/// The maintenance backlog: stored tapes of one grammar version and tape format after a keyset
/// cursor that have no search index yet, with the source and tape frames needed to build one.
/// Tapes of another tape format are never in it: this build cannot decode them, and no lookup of
/// this build reads them (`tape_objects` lookups match the format too).
pub(super) const TAPES_WITHOUT_SEARCH_INDEX_SQL: &str = "SELECT t.tape_id, t.grammar_id, \
     s.codec_id, s.raw_length, s.frame, sh.algorithm_id, sh.lane0, sh.lane1, sh.lane2, sh.lane3, \
     t.codec_id, t.raw_length, t.frame, th.algorithm_id, th.lane0, th.lane1, th.lane2, th.lane3 \
     FROM tape_objects t \
     JOIN hashes th ON th.hash_id = t.canonical_hash_id \
     JOIN source_objects s ON s.source_id = t.source_id \
     JOIN hashes sh ON sh.hash_id = s.canonical_hash_id \
     WHERE t.grammar_version = ?1 AND t.tape_format_version = ?2 AND t.tape_id > ?3 \
     AND NOT EXISTS (SELECT 1 FROM search_index_sets x WHERE x.tape_id = t.tape_id) \
     ORDER BY t.tape_id LIMIT ?4";

impl ContentStore {
    /// Take `tapes`, which this build can never decode into a search index, out of the maintenance
    /// backlog: each gets an incomplete search-index set, which presence lookups answer as
    /// `Unknown`, so the tape is never narrowed. A tape that already has a set keeps it.
    pub(crate) fn put_unavailable_search_indexes(
        &self,
        tapes: &[TapeId],
        index_version: i64,
    ) -> StoreResult<()> {
        if tapes.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(
                "search-index publication exceeds 64 tapes".to_owned(),
            ));
        }
        self.require_write_capacity()?;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The schema requires a positive node count; nothing reads the count of an incomplete set.
        let mut put_set = transaction.prepare_cached(
            "INSERT INTO search_index_sets(tape_id,index_version,node_count,complete,has_errors) \
             VALUES(?1,?2,1,0,0) ON CONFLICT(tape_id) DO NOTHING",
        )?;
        for tape in tapes {
            put_set.execute(params![tape.0, index_version])?;
        }
        drop(put_set);
        transaction.commit()?;
        Ok(())
    }

    /// Publish one query-independent FTS document per immutable tape. The kind stream contains one
    /// token per preorder node. All string construction and validation finish before the short
    /// write transaction starts.
    pub(crate) fn put_search_indexes_bulk(&self, indexes: &[TapeSearchIndex]) -> StoreResult<()> {
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
            {
                return Err(StoreError::Invalid(
                    "invalid positional FTS search document".to_owned(),
                ));
            }
        }
        self.require_write_capacity()?;
        let mut connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut remove_kind =
            transaction.prepare_cached("DELETE FROM search_kind_fts WHERE rowid=?1")?;
        let mut put_kind =
            transaction.prepare_cached("INSERT INTO search_kind_fts(rowid,token) VALUES(?1,?2)")?;
        let mut put_set = transaction.prepare_cached(
            "INSERT INTO search_index_sets(tape_id,index_version,node_count,complete,has_errors) \
             VALUES(?1,?2,?3,1,?4) ON CONFLICT(tape_id) DO UPDATE SET \
             index_version=excluded.index_version,node_count=excluded.node_count,complete=1, \
             has_errors=excluded.has_errors",
        )?;
        for index in indexes {
            // Contentless-delete makes an absent row a no-op. Delete-first keeps publication
            // idempotent without a point lookup or stale FTS segments.
            remove_kind.execute([index.tape_id.0])?;
            put_kind.execute(params![index.tape_id.0, index.kind_tokens])?;
            #[cfg(test)]
            SEARCH_INDEX_FAMILY_WRITES.fetch_add(1, Ordering::Relaxed);
            put_set.execute(params![
                index.tape_id.0,
                index.index_version,
                index.node_count,
                index.has_errors
            ])?;
        }
        drop((remove_kind, put_kind, put_set));
        transaction.commit()?;
        Ok(())
    }

    /// Up to `limit` stored tapes of `grammar_version` and `tape_format_version` after `after`
    /// without a search index: tape id, grammar id, source text and decoded tape image. The caller
    /// retains the keyset cursor across maintenance wakes, so each batch seeks past already-indexed
    /// tapes.
    pub(crate) fn tapes_without_search_index_after(
        &self,
        grammar_version: i64,
        tape_format_version: i64,
        after: Option<TapeId>,
        limit: usize,
    ) -> StoreResult<Vec<UnindexedTape>> {
        type Frame = (i64, i64, Vec<u8>, HashKey);
        let rows = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            let mut statement = connection.prepare_cached(TAPES_WITHOUT_SEARCH_INDEX_SQL)?;
            let frame = |row: &rusqlite::Row<'_>, at: usize| -> rusqlite::Result<Frame> {
                Ok((
                    row.get(at)?,
                    row.get(at + 1)?,
                    row.get(at + 2)?,
                    HashKey {
                        algorithm: row.get(at + 3)?,
                        lanes: [
                            row.get(at + 4)?,
                            row.get(at + 5)?,
                            row.get(at + 6)?,
                            row.get(at + 7)?,
                        ],
                    },
                ))
            };
            statement
                .query_map(
                    params![
                        grammar_version,
                        tape_format_version,
                        after.map_or(0, |tape| tape.0),
                        limit as i64
                    ],
                    |row| {
                        Ok((
                            TapeId(row.get(0)?),
                            row.get::<_, i64>(1)?,
                            frame(row, 2)?,
                            frame(row, 10)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let decode = |(codec, length, frame, hash): Frame| -> StoreResult<Vec<u8>> {
            if codec != CODEC_ZSTD {
                return Err(StoreError::Corrupt("unsupported object codec".to_owned()));
            }
            let bytes = zstd::stream::decode_all(Cursor::new(frame))?;
            if usize::try_from(length).ok() != Some(bytes.len()) || HashKey::sha256(&bytes) != hash
            {
                return Err(StoreError::Corrupt(
                    "object length or hash does not match metadata".to_owned(),
                ));
            }
            Ok(bytes)
        };
        rows.into_iter()
            .map(|(tape, grammar, source, image)| {
                Ok(UnindexedTape {
                    tape,
                    grammar,
                    source: decode(source)?,
                    image: decode(image)?,
                })
            })
            .collect()
    }

    /// Returns whether each complete, error-free stored tape contains any requested root kind.
    /// Missing, stale, malformed, and syntax-error index rows remain non-negative answers.
    pub(crate) fn get_search_kind_presence_bulk(
        &self,
        lookups: &[SearchKindLookup],
    ) -> StoreResult<Vec<StoredKindPresence>> {
        let requests = lookups
            .iter()
            .enumerate()
            .map(|(position, lookup)| {
                serde_json::json!([
                    position,
                    lookup.tape_id.0,
                    lookup.index_version,
                    lookup.kind_tokens.join(" OR ")
                ])
            })
            .collect::<Vec<_>>();
        let requests = serde_json::to_string(&requests)
            .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH requested AS MATERIALIZED (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) position, \
               CAST(json_extract(value,'$[1]') AS INTEGER) tape_id, \
               CAST(json_extract(value,'$[2]') AS INTEGER) version, \
               json_extract(value,'$[3]') query FROM json_each(?1)) \
             SELECT r.position,s.complete,s.has_errors,EXISTS( \
               SELECT 1 FROM search_kind_fts \
               WHERE rowid=r.tape_id AND search_kind_fts MATCH r.query \
             ) FROM requested r LEFT JOIN search_index_sets s \
               ON s.tape_id=r.tape_id AND s.index_version=r.version ORDER BY r.position",
        )?;
        let rows = statement
            .query_map([requests], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<bool>>(1)?,
                    row.get::<_, Option<bool>>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut selections = vec![StoredKindPresence::Unknown; lookups.len()];
        for (position, complete, has_errors, present) in rows {
            let position = usize::try_from(position)
                .map_err(|_| StoreError::Corrupt("invalid search request position".to_owned()))?;
            selections[position] = match (complete, has_errors) {
                (Some(true), Some(true)) => StoredKindPresence::SyntaxErrors,
                (Some(true), Some(false)) if present => StoredKindPresence::Present,
                (Some(true), Some(false)) => StoredKindPresence::Absent,
                _ => StoredKindPresence::Unknown,
            };
        }
        Ok(selections)
    }
}
