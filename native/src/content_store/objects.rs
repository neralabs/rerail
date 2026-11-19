//! Content-addressed source and tape objects: compression, aliases, and bulk reads.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::read_cache::CacheKey;
use super::rows::{generation, hash_params, intern_hash};
use super::object_segments::{ObjectFrame, object_dir, row_frame};
use super::segments::SegmentMaps;
use super::{
    CODEC_ZSTD, CachedBytes, ContentStore, DocumentPayload, HASH_SHA256, HashKey, SourceId,
    StoreError, StoreResult, StoredDocument, StoredPathObjects, StoredSourceObjects, TapeId,
    TapePayload,
    compression::{compress_source, compress_tape, path_tag, tape_tag},
};
use rusqlite::Connection;
use rusqlite::{OptionalExtension, params};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// A stored tape's frames as residency admits them (PIAT §20.3): the tape's and its source's (zstd,
/// or in a segment) with their raw lengths, the source's SHA-256 and the tape's grammar.
pub struct ResidentFrames {
    pub grammar_id: i64,
    pub tape_length: usize,
    pub tape_frame: ObjectFrame,
    pub source_length: usize,
    pub source_frame: ObjectFrame,
    pub source_sha256: [u8; 32],
}

/// A source's and its tape's compressed frames, with what storing them needs of the raw bytes (their lengths, the
/// tape's identity), made before the store's writer takes them (`source_and_tape_frames`): the writer holds and
/// reads no raw source or tape.
pub struct SourceAndTapeFrames {
    pub source_len: usize,
    pub source: Vec<u8>,
    pub tape_len: usize,
    pub tape_identity: HashKey,
    pub tape: Vec<u8>,
}

impl SourceAndTapeFrames {
    /// The bytes the frames hold.
    pub fn bytes(&self) -> usize {
        self.source.len() + self.tape.len()
    }

    /// The tape, decompressed: a read of a tape still queued for the writer.
    pub fn tape_bytes(&self) -> StoreResult<Vec<u8>> {
        Ok(super::compression::decompress(&self.tape, self.tape_len)?)
    }
}

impl ContentStore {
    /// The frames of the tapes whose SHA-256 is each of `tapes`, in one read: `None` where no tape
    /// has that digest (or its segment is gone). Frames are returned as stored, for the caller to
    /// decompress (or copy) in place.
    pub fn resident_frames(&self, tapes: &[[u8; 32]]) -> StoreResult<Vec<Option<ResidentFrames>>> {
        let encoded = serde_json::to_string(
            &tapes
                .iter()
                .enumerate()
                .map(|(at, digest)| {
                    let key = HashKey::from_sha256_digest(*digest);
                    serde_json::json!([at, key.algorithm, key.lanes[0], key.lanes[1], key.lanes[2], key.lanes[3]])
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) at, \
             CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
             CAST(json_extract(value,'$[3]') AS INTEGER) lane1, CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
             CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
             SELECT i.at,t.grammar_id,t.codec_id,t.raw_length,t.frame,s.codec_id,s.raw_length,s.frame,\
             sh.algorithm_id,sh.lane0,sh.lane1,sh.lane2,sh.lane3,t.seg,t.seg_at,t.seg_len,s.seg,s.seg_at,s.seg_len \
             FROM input i CROSS JOIN hashes th INDEXED BY hashes_by_prefix ON th.algorithm_id=i.algorithm AND th.lane0=i.lane0 \
             AND th.lane1=i.lane1 AND th.lane2=i.lane2 AND th.lane3=i.lane3 \
             JOIN tape_objects t ON t.canonical_hash_id=th.hash_id \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes sh ON sh.hash_id=s.canonical_hash_id",
        )?;
        let mut out = (0..tapes.len()).map(|_| None).collect::<Vec<_>>();
        type Row = (i64, i64, usize, usize, [u8; 32], (i64, Vec<u8>, [i64; 3]), (i64, Vec<u8>, [i64; 3]));
        let rows: Vec<Row> = statement
            .query_map([encoded], |row| {
                let length = |at: usize| row.get::<_, i64>(at).map(|length| usize::try_from(length).unwrap_or(usize::MAX));
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get(1)?,
                    length(3)?,
                    length(6)?,
                    HashKey { algorithm: row.get(8)?, lanes: [row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?] }.sha256_bytes().unwrap_or_default(),
                    (row.get(2)?, row.get(4)?, [row.get(13)?, row.get(14)?, row.get(15)?]),
                    (row.get(5)?, row.get(7)?, [row.get(16)?, row.get(17)?, row.get(18)?]),
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for (at, grammar_id, tape_length, source_length, source_sha256, (tc, tf, t), (sc, sf, s)) in rows {
            let (Some(tape_frame), Some(source_frame)) = (row_frame(&mut maps, tc, tf, t[0], t[1], t[2])?, row_frame(&mut maps, sc, sf, s[0], s[1], s[2])?) else { continue };
            if let Some(slot) = usize::try_from(at).ok().and_then(|at| out.get_mut(at)) {
                *slot = Some(ResidentFrames { grammar_id, tape_length, tape_frame, source_length, source_frame, source_sha256 });
            }
        }
        Ok(out)
    }
    pub fn put_source(&self, source: &[u8], aliases: &[HashKey]) -> StoreResult<SourceId> {
        self.put_source_hashed(source, HashKey::sha256(source), aliases)
    }

    /// [`Self::put_source`] for a caller that already holds the source's SHA-256 (`canonical`): the bytes are not
    /// hashed again.
    pub fn put_source_hashed(&self, source: &[u8], canonical: HashKey, aliases: &[HashKey]) -> StoreResult<SourceId> {
        self.put_source_tagged(source, canonical, aliases, None)
    }

    /// [`Self::put_source`] for a file at `path`: its frame may use its language's dictionary.
    pub fn put_source_at(&self, source: &[u8], aliases: &[HashKey], path: &str) -> StoreResult<SourceId> {
        self.put_source_tagged(source, HashKey::sha256(source), aliases, path_tag(path))
    }

    fn put_source_tagged(
        &self,
        source: &[u8],
        canonical: HashKey,
        aliases: &[HashKey],
        tag: Option<u8>,
    ) -> StoreResult<SourceId> {
        self.require_write_capacity()?;
        let frame = compress_source(source, self.compression_level, tag)?;
        let raw_length = i64::try_from(source.len())
            .map_err(|_| StoreError::Invalid("source length exceeds i64".to_owned()))?;
        // Retried under `retry_busy`: the push writes sources while the overlay's writer holds
        // the store, and one busy timeout behind it must not fail the push.
        self.immediate_transaction(|transaction| {
            let canonical_hash_id = intern_hash(transaction, canonical)?;
            let generation = generation(transaction)?;
            transaction.execute_cached(
                "INSERT INTO source_objects(canonical_hash_id, raw_length, codec_id, frame, created_generation)\
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(canonical_hash_id) DO NOTHING",
                params![canonical_hash_id, raw_length, CODEC_ZSTD, frame, generation],
            )?;
            let source_id: i64 = transaction.query_row_cached(
                "SELECT source_id FROM source_objects WHERE canonical_hash_id = ?1",
                [canonical_hash_id],
                |row| row.get(0),
            )?;
            for alias in aliases {
                let alias_hash_id = intern_hash(transaction, *alias)?;
                let existing = transaction
                    .query_row_cached(
                        "SELECT source_id FROM source_aliases WHERE alias_hash_id = ?1",
                        [alias_hash_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                if existing.is_some_and(|existing| existing != source_id) {
                    return Err(StoreError::Corrupt(
                        "Git object alias maps to different source bytes".to_owned(),
                    ));
                }
                transaction.execute_cached(
                    "INSERT INTO source_aliases(alias_hash_id, source_id) VALUES (?1, ?2)\
                     ON CONFLICT(alias_hash_id) DO NOTHING",
                    params![alias_hash_id, source_id],
                )?;
            }
            Ok(SourceId(source_id))
        })
    }

    pub fn source_by_hash(&self, hash: HashKey) -> StoreResult<Option<SourceId>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        source_id_by_hash(&connection, hash)
    }

    /// The tape of each of `sources` in `grammar_id` at the given versions, as its stored frame, in their order: one
    /// read, nothing written. A run reads its files' tapes before its first wave, and decompresses each in its file's
    /// wave (decision 0104).
    pub fn tape_frames_for(
        &self,
        sources: &[SourceId],
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
    ) -> StoreResult<Vec<Option<TapeFrame>>> {
        let mut out: Vec<Option<TapeFrame>> = vec![None; sources.len()];
        if sources.is_empty() {
            return Ok(out);
        }
        let list = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("integers");
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT j.key, t.raw_length, t.codec_id, t.frame, t.seg, t.seg_at, t.seg_len FROM json_each(?1) j JOIN tape_objects t \
             ON t.source_id=j.value AND t.grammar_id=?2 AND t.grammar_version=?3 AND t.tape_format_version=?4",
        )?;
        type Row = (i64, i64, i64, Vec<u8>, [i64; 3]);
        let rows: Vec<Row> = statement
            .query_map(params![list, grammar_id, grammar_version, tape_format_version], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, [row.get(4)?, row.get(5)?, row.get(6)?]))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for (at, raw_length, codec, frame, [seg, seg_at, seg_len]) in rows {
            // A frame of a codec this binary does not read, or whose segment is gone, is a miss: parsed again.
            if let Ok(Some(frame)) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len) {
                out[at as usize] = Some(TapeFrame { raw_length, frame });
            }
        }
        Ok(out)
    }

    /// The tapes of `sources` in `grammar_id` at the given versions, each with its source's id and content hash: one
    /// read, nothing written. A run reads before its first wave the tapes of the files the root's last runs named
    /// (Q22), to find a file's by its source or by its text's hash (a file the run knows by its digest alone).
    pub fn tape_frames_with_hashes(&self, sources: &[SourceId], grammar_id: i64, grammar_version: i64, tape_format_version: i64) -> StoreResult<Vec<(SourceId, HashKey, TapeFrame)>> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let list = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("integers");
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT t.source_id, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, t.raw_length, t.codec_id, t.frame, t.seg, t.seg_at, t.seg_len \
             FROM json_each(?1) j JOIN tape_objects t ON t.source_id=j.value AND t.grammar_id=?2 AND t.grammar_version=?3 AND t.tape_format_version=?4 \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes h ON h.hash_id=s.canonical_hash_id",
        )?;
        type Row = (SourceId, HashKey, i64, i64, Vec<u8>, [i64; 3]);
        let rows: Vec<Row> = statement
            .query_map(params![list, grammar_id, grammar_version, tape_format_version], |row| {
                Ok((
                    SourceId(row.get(0)?),
                    HashKey { algorithm: row.get(1)?, lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?] },
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    [row.get(9)?, row.get(10)?, row.get(11)?],
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut out = Vec::new();
        for (source, hash, raw_length, codec, frame, [seg, seg_at, seg_len]) in rows {
            // A frame of a codec this binary does not read, or whose segment is gone, is a miss: parsed again.
            if let Ok(Some(frame)) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len) {
                out.push((source, hash, TapeFrame { raw_length, frame }));
            }
        }
        Ok(out)
    }

    pub fn tape_by_source(
        &self,
        source_id: SourceId,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
    ) -> StoreResult<Option<TapeId>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row_cached(
                "SELECT tape_id FROM tape_objects WHERE source_id=?1 AND grammar_id=?2 \
                 AND grammar_version=?3 AND tape_format_version=?4",
                params![
                    source_id.0,
                    grammar_id,
                    grammar_version,
                    tape_format_version
                ],
                |row| row.get::<_, i64>(0).map(TapeId),
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn has_source_aliases(&self) -> StoreResult<bool> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row_cached(
                "SELECT EXISTS(SELECT 1 FROM source_aliases LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    pub fn source_by_alias(&self, alias: HashKey) -> StoreResult<Option<SourceId>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        connection
            .query_row_cached(
                "SELECT a.source_id FROM hashes h JOIN source_aliases a ON a.alias_hash_id = h.hash_id \
                 WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5",
                hash_params(alias),
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|value| value.map(SourceId))
            .map_err(StoreError::from)
    }

    /// `source_by_alias` of many aliases, in their order, under one hold of the connection: a run that reads 300
    /// programs' images asks once, not 300 times from as many threads.
    pub fn sources_by_aliases(&self, aliases: &[HashKey]) -> StoreResult<Vec<Option<SourceId>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT a.source_id FROM hashes h JOIN source_aliases a ON a.alias_hash_id = h.hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5",
        )?;
        aliases
            .iter()
            .map(|alias| statement.query_row(hash_params(*alias), |row| row.get::<_, i64>(0)).optional().map(|v| v.map(SourceId)).map_err(StoreError::from))
            .collect()
    }

    /// The stored source each path's alias names, with its length in bytes. A source is stored
    /// under a Git blob alias only when its bytes hash to that blob, so the length is the blob's.
    pub fn stored_sources_by_alias(
        &self,
        requests: &[(String, HashKey)],
    ) -> StoreResult<HashMap<String, (StoredPathObjects, u64)>> {
        if requests.is_empty() {
            return Ok(HashMap::new());
        }
        let input = requests
            .iter()
            .map(|(path, alias)| {
                serde_json::json!([
                    path,
                    alias.algorithm,
                    alias.lanes[0],
                    alias.lanes[1],
                    alias.lanes[2],
                    alias.lanes[3]
                ])
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_string(&input).map_err(|error| {
            StoreError::Invalid(format!("source alias lookup cannot be encoded: {error}"))
        })?;
        let connection = self
            .connection
            .lock()
            .expect("content store connection unavailable");
        let mut statement = connection.prepare_cached(
            "WITH input AS (SELECT json_extract(value,'$[0]') path, \
             CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
             CAST(json_extract(value,'$[3]') AS INTEGER) lane1, CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
             CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
             SELECT i.path,ch.algorithm_id,ch.lane0,ch.lane1,ch.lane2,ch.lane3,s.source_id,s.raw_length \
             FROM input i CROSS JOIN hashes ah INDEXED BY hashes_by_prefix ON ah.algorithm_id=i.algorithm AND ah.lane0=i.lane0 \
             AND ah.lane1=i.lane1 AND ah.lane2=i.lane2 AND ah.lane3=i.lane3 \
             JOIN source_aliases a ON a.alias_hash_id=ah.hash_id \
             JOIN source_objects s ON s.source_id=a.source_id JOIN hashes ch ON ch.hash_id=s.canonical_hash_id \
             ORDER BY i.path",
        )?;
        let rows = statement.query_map([encoded], |row| {
            Ok((
                row.get::<_, String>(0)?,
                HashKey {
                    algorithm: row.get(1)?,
                    lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                },
                SourceId(row.get(6)?),
                u64::try_from(row.get::<_, i64>(7)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
            ))
        })?;
        rows.map(|row| {
            let (path, source_hash, source, length) = row?;
            Ok((
                path,
                (
                    StoredPathObjects {
                        source_hash,
                        objects: StoredSourceObjects {
                            source,
                            tapes: HashMap::new(),
                        },
                    },
                    length,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, rusqlite::Error>>()
        .map_err(StoreError::from)
    }

    pub fn source_sha256(&self, source_id: SourceId) -> StoreResult<[u8; 32]> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let key: HashKey = connection.query_row_cached(
            "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 FROM source_objects s \
             JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
            [source_id.0],
            |row| {
                Ok(HashKey {
                    algorithm: row.get(0)?,
                    lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                })
            },
        )?;
        if key.algorithm != HASH_SHA256 {
            return Err(StoreError::Corrupt(
                "source canonical hash is not SHA-256".to_owned(),
            ));
        }
        let mut digest = [0_u8; 32];
        for (chunk, lane) in digest.chunks_exact_mut(8).zip(key.lanes) {
            chunk.copy_from_slice(&lane.to_be_bytes());
        }
        Ok(digest)
    }

    /// The SHA-256 of each of `sources`' contents, in their order (`None`: no such source): one read.
    pub fn source_digests(&self, sources: &[SourceId]) -> StoreResult<Vec<Option<[u8; 32]>>> {
        let mut out = vec![None; sources.len()];
        if sources.is_empty() {
            return Ok(out);
        }
        let ids = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("integers");
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT j.key, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3 FROM json_each(?1) j \
             CROSS JOIN source_objects s ON s.source_id = j.value JOIN hashes h ON h.hash_id = s.canonical_hash_id",
        )?;
        let rows = statement.query_map([ids], |row| {
            Ok((row.get::<_, i64>(0)?, HashKey { algorithm: row.get(1)?, lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?] }))
        })?;
        for row in rows {
            let (at, key) = row?;
            out[at as usize] = key.sha256_bytes();
        }
        Ok(out)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn source_has_hash(&self, source_id: SourceId, key: HashKey) -> StoreResult<bool> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let stored = connection
            .query_row_cached(
                "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 FROM source_objects s \
                 JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
                [source_id.0],
                |row| {
                    Ok(HashKey {
                        algorithm: row.get(0)?,
                        lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                    })
                },
            )
            .optional()?;
        Ok(stored == Some(key))
    }

    pub fn get_source(&self, source_id: SourceId) -> StoreResult<Vec<u8>> {
        let canonical_hash = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            connection.query_row_cached(
                "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 FROM source_objects s \
                 JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
                [source_id.0],
                |row| {
                    Ok(HashKey {
                        algorithm: row.get(0)?,
                        lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                    })
                },
            )?
        };
        let cache_key = CacheKey::Source(canonical_hash);
        if let Some(source) = self
            .decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .get(cache_key)
        {
            return Ok(source.to_vec());
        }
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let (raw_length, codec, frame, key, [seg, seg_at, seg_len]): (i64, i64, Vec<u8>, HashKey, [i64; 3]) = connection.query_row_cached(
            "SELECT s.raw_length, s.codec_id, s.frame, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, s.seg, s.seg_at, s.seg_len \
             FROM source_objects s JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
            [source_id.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, HashKey { algorithm: row.get(3)?, lanes: [row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?] }, [row.get(8)?, row.get(9)?, row.get(10)?])),
        )?;
        drop(connection);
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let Some(frame) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len)? else {
            return Err(StoreError::Corrupt("a source's segment is not there".to_owned()));
        };
        // In a segment: read in place, no copy to the cache.
        if let ObjectFrame::Raw(bytes) = &frame {
            if usize::try_from(raw_length).ok() != Some(bytes.len()) || key != canonical_hash {
                return Err(StoreError::Corrupt("source length or hash does not match metadata".to_owned()));
            }
            return Ok(bytes.bytes().to_vec());
        }
        let source = frame.bytes(0)?.into_vec();
        if source.len() != usize::try_from(raw_length).unwrap_or(usize::MAX) {
            return Err(StoreError::Corrupt(
                "source length does not match metadata".to_owned(),
            ));
        }
        if key != canonical_hash || key != HashKey::sha256(&source) {
            return Err(StoreError::Corrupt(
                "source hash does not match metadata".to_owned(),
            ));
        }
        let source = self
            .decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .insert_bytes(cache_key, source)?;
        Ok(source.to_vec())
    }

    /// Read immutable source IDs qualified by the canonical hashes from bulk metadata.
    /// Cache hits require no source query. Cold frames use bounded typed batches.
    pub fn get_sources_by_hash_bulk(
        &self,
        requests: &[(SourceId, HashKey)],
        max_bytes: usize,
    ) -> StoreResult<HashMap<SourceId, Vec<u8>>> {
        let mut expected = HashMap::new();
        for &(id, hash) in requests {
            if expected.insert(id, hash).is_some_and(|old| old != hash) {
                return Err(StoreError::Invalid(
                    "source request has conflicting hashes".into(),
                ));
            }
        }
        let mut total = 0usize;
        let mut result = HashMap::new();
        let mut ids = Vec::new();
        {
            let mut cache = self
                .decompressed_cache
                .lock()
                .expect("decompressed cache mutex poisoned");
            for (&id, &hash) in &expected {
                if let Some(bytes) = cache.get(CacheKey::Source(hash)) {
                    total = total
                        .checked_add(bytes.len())
                        .filter(|n| *n <= max_bytes)
                        .ok_or_else(|| {
                            StoreError::Invalid("source batch exceeds byte budget".into())
                        })?;
                    result.insert(id, bytes.to_vec());
                } else {
                    ids.push(id.0);
                }
            }
        }
        ids.sort_unstable();
        for ids in ids.chunks(128) {
            let placeholders = std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let frames = {
                let connection = self
                    .connection
                    .lock()
                    .expect("content store mutex poisoned");
                let sql = format!(
                    "SELECT s.source_id,s.raw_length,s.codec_id,s.frame,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,s.seg,s.seg_at,s.seg_len FROM source_objects s JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id IN ({placeholders})"
                );
                let mut statement = connection.prepare_cached(&sql)?;
                let mut rows = statement.query(rusqlite::params_from_iter(ids))?;
                let mut frames = Vec::new();
                while let Some(row) = rows.next()? {
                    let length: i64 = row.get(1)?;
                    let length = usize::try_from(length)
                        .map_err(|_| StoreError::Corrupt("invalid source length".into()))?;
                    total = total
                        .checked_add(length)
                        .filter(|total| *total <= max_bytes)
                        .ok_or_else(|| {
                            StoreError::Invalid("source batch exceeds byte budget".into())
                        })?;
                    frames.push((
                        SourceId(row.get(0)?),
                        length,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        HashKey {
                            algorithm: row.get(4)?,
                            lanes: [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
                        },
                        [row.get::<_, i64>(9)?, row.get::<_, i64>(10)?, row.get::<_, i64>(11)?],
                    ));
                }
                frames
            };
            let mut maps = SegmentMaps::new(object_dir(&self.database_path));
            for (id, length, codec, frame, hash, [seg, seg_at, seg_len]) in frames {
                if expected[&id] != hash {
                    return Err(StoreError::Corrupt(
                        "source identity does not match metadata".into(),
                    ));
                }
                let key = CacheKey::Source(hash);
                let cached = self
                    .decompressed_cache
                    .lock()
                    .expect("decompressed cache mutex poisoned")
                    .get(key);
                let bytes = match cached {
                    Some(bytes) => bytes,
                    None => {
                        let Some(frame) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len)? else { continue };
                        if let ObjectFrame::Raw(bytes) = &frame {
                            if bytes.len() != length {
                                return Err(StoreError::Corrupt("source length does not match metadata".into()));
                            }
                            result.insert(id, bytes.bytes().to_vec());
                            continue;
                        }
                        let bytes = frame.bytes(0)?.into_vec();
                        if bytes.len() != length || HashKey::sha256(&bytes) != hash {
                            return Err(StoreError::Corrupt(
                                "source length or hash does not match metadata".into(),
                            ));
                        }
                        self.decompressed_cache
                            .lock()
                            .expect("decompressed cache mutex poisoned")
                            .insert_bytes(key, bytes)?
                    }
                };
                result.insert(id, bytes.to_vec());
            }
        }
        Ok(result)
    }

    pub fn put_tape(
        &self,
        source_id: SourceId,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
        tape: &[u8],
    ) -> StoreResult<TapeId> {
        self.require_write_capacity()?;
        if grammar_id < 0 || grammar_version < 0 || tape_format_version < 0 {
            return Err(StoreError::Invalid(
                "tape identifiers must be nonnegative".to_owned(),
            ));
        }
        let frame = compress_tape(tape, self.compression_level)?;
        // Retried under `retry_busy`, as `put_source` is.
        self.immediate_transaction(|transaction| {
            put_tape_tx(
                transaction,
                source_id,
                TapePayload {
                    grammar_id,
                    grammar_version,
                    tape_format_version,
                    bytes: tape,
                    documents: &[],
                },
                &frame,
                &[],
            )
        })
    }

    /// [`Self::put_source_hashed`] and [`Self::put_tape`] in one transaction: the source (`canonical` its SHA-256),
    /// stored or found, and its tape. A source looked up earlier may have been collected since (a pass deletes a
    /// source no tape, pin or live manifest holds); in one transaction no pass can come between the source and the
    /// tape that holds it (O95). Returns the source's id as it is now.
    pub fn put_source_and_tape(
        &self,
        source: &[u8],
        canonical: HashKey,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
        tape: &[u8],
    ) -> StoreResult<(SourceId, TapeId)> {
        let frames = self.source_and_tape_frames(source, canonical, tape)?;
        self.put_source_and_tape_framed(canonical, grammar_id, grammar_version, tape_format_version, &frames)
    }

    /// The compressed frames [`Self::put_source_and_tape_framed`] stores: made where the source and tape are made (a
    /// parse worker), so the store's one writer thread only inserts them. Compressing on the writer took most of its
    /// time on a cold run, and the queue held every source whole until then.
    pub fn source_and_tape_frames(&self, source: &[u8], canonical: HashKey, tape: &[u8]) -> StoreResult<SourceAndTapeFrames> {
        Ok(SourceAndTapeFrames {
            source_len: source.len(),
            source: compress_source(source, self.compression_level, tape_tag(tape))?,
            tape_len: tape.len(),
            tape_identity: HashKey::tape_identity(canonical, tape),
            tape: compress_tape(tape, self.compression_level)?,
        })
    }

    /// [`Self::put_source_and_tape`] of frames made beforehand (`source_and_tape_frames`).
    pub fn put_source_and_tape_framed(
        &self,
        canonical: HashKey,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
        frames: &SourceAndTapeFrames,
    ) -> StoreResult<(SourceId, TapeId)> {
        self.require_write_capacity()?;
        if grammar_id < 0 || grammar_version < 0 || tape_format_version < 0 {
            return Err(StoreError::Invalid(
                "tape identifiers must be nonnegative".to_owned(),
            ));
        }
        let (source_frame, tape_frame) = (&frames.source, &frames.tape);
        self.immediate_transaction(|transaction| {
            let created_generation = generation(transaction)?;
            let source_id = put_source_prepared_tx(
                transaction,
                canonical,
                frames.source_len,
                source_frame,
                &[],
                created_generation,
            )?;
            // The source's hash is `canonical` (stored or found under it), so the tape's identity was made with it.
            let tape_id = put_tape_prepared_tx(
                transaction,
                source_id,
                TapePayload { grammar_id, grammar_version, tape_format_version, bytes: &[], documents: &[] },
                frames.tape_len,
                frames.tape_identity,
                tape_frame,
                created_generation,
            )?;
            Ok((source_id, tape_id))
        })
    }

    pub fn get_tapes_bulk(
        &self,
        tape_ids: &[TapeId],
    ) -> StoreResult<HashMap<TapeId, (Arc<CachedBytes>, bool)>> {
        let ids = tape_ids.iter().map(|id| id.0).collect::<BTreeSet<_>>();
        Ok(self
            .objects_bulk(&ids, BulkObjects::Tapes)?
            .into_iter()
            .map(|(id, object)| (TapeId(id), object))
            .collect())
    }

    /// The sources `source_ids` name, each with whether the read cache held it; a source that is
    /// not stored is left out.
    pub fn get_sources_bulk(
        &self,
        source_ids: &[SourceId],
    ) -> StoreResult<HashMap<SourceId, (Arc<CachedBytes>, bool)>> {
        let ids = source_ids.iter().map(|id| id.0).collect::<BTreeSet<_>>();
        Ok(self
            .objects_bulk(&ids, BulkObjects::Sources)?
            .into_iter()
            .map(|(id, object)| (SourceId(id), object))
            .collect())
    }

    /// The documents derived from each of `tapes` (`file_documents`), in ordinal order, with
    /// their sources and tapes. A tape that derives none is left out.
    pub fn file_documents_bulk(
        &self,
        tapes: &[TapeId],
    ) -> StoreResult<HashMap<TapeId, Vec<StoredDocument>>> {
        let mut documents = HashMap::<TapeId, Vec<StoredDocument>>::new();
        if tapes.is_empty() {
            return Ok(documents);
        }
        let requested =
            serde_json::to_string(&tapes.iter().map(|id| id.0).collect::<BTreeSet<_>>())
                .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        // A row whose tape is gone stays in the answer, so its file reads as not stored.
        let mut statement = connection.prepare_cached(
            "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
             SELECT d.tape_id,d.ordinal,d.parent_ordinal,d.role_id,d.via_id,d.document_tape_id,\
             t.source_id,t.grammar_id,d.source_map,d.host_errors FROM requested r \
             JOIN file_documents d ON d.tape_id=r.id \
             LEFT JOIN tape_objects t ON t.tape_id=d.document_tape_id \
             ORDER BY d.tape_id,d.ordinal",
        )?;
        let rows = statement.query_map([&requested], |row| {
            Ok((
                TapeId(row.get(0)?),
                StoredDocument {
                    ordinal: row.get(1)?,
                    parent_ordinal: row.get(2)?,
                    role_id: row.get(3)?,
                    via_id: row.get(4)?,
                    tape: TapeId(row.get(5)?),
                    source: row.get::<_, Option<i64>>(6)?.map(SourceId),
                    grammar_id: row.get(7)?,
                    source_map: row.get(8)?,
                    host_errors: row.get(9)?,
                },
            ))
        })?;
        for row in rows {
            let (tape, document) = row?;
            documents.entry(tape).or_default().push(document);
        }
        Ok(documents)
    }

    /// The stored objects `ids` name, read from the read cache or decompressed and validated,
    /// each with whether the cache held it. A missing row is a cache miss: callers parse again
    /// rather than issuing a point lookup.
    fn objects_bulk(
        &self,
        ids: &BTreeSet<i64>,
        objects: BulkObjects,
    ) -> StoreResult<HashMap<i64, (Arc<CachedBytes>, bool)>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let (identity_sql, frame_sql, what) = match objects {
            BulkObjects::Tapes => (
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT t.tape_id,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 \
                 FROM requested r JOIN tape_objects t ON t.tape_id=r.id \
                 JOIN hashes h ON h.hash_id=t.canonical_hash_id",
                // A tape's identity covers its source's hash (`HashKey::tape_identity`).
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT t.tape_id,t.raw_length,t.codec_id,t.frame,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,\
                 sh.algorithm_id,sh.lane0,sh.lane1,sh.lane2,sh.lane3,t.seg,t.seg_at,t.seg_len \
                 FROM requested r JOIN tape_objects t ON t.tape_id=r.id JOIN hashes h ON h.hash_id=t.canonical_hash_id \
                 JOIN source_objects s ON s.source_id=t.source_id JOIN hashes sh ON sh.hash_id=s.canonical_hash_id",
                "tape",
            ),
            BulkObjects::Sources => (
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT s.source_id,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 \
                 FROM requested r JOIN source_objects s ON s.source_id=r.id \
                 JOIN hashes h ON h.hash_id=s.canonical_hash_id",
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT s.source_id,s.raw_length,s.codec_id,s.frame,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,\
                 h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,s.seg,s.seg_at,s.seg_len \
                 FROM requested r JOIN source_objects s ON s.source_id=r.id JOIN hashes h ON h.hash_id=s.canonical_hash_id",
                "source",
            ),
        };
        let cache_key = |hash| match objects {
            BulkObjects::Tapes => CacheKey::Tape(hash),
            BulkObjects::Sources => CacheKey::Source(hash),
        };
        let requested =
            serde_json::to_string(ids).map_err(|error| StoreError::Invalid(error.to_string()))?;
        let identities = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            let mut statement = connection.prepare_cached(identity_sql)?;
            statement
                .query_map([&requested], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        HashKey {
                            algorithm: row.get(1)?,
                            lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                        },
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut result = HashMap::new();
        let mut missing = BTreeSet::new();
        {
            let mut cache = self
                .decompressed_cache
                .lock()
                .expect("decompressed cache mutex poisoned");
            for (id, hash) in identities {
                if let Some(bytes) = cache.get(cache_key(hash)) {
                    result.insert(id, (bytes, true));
                } else {
                    missing.insert(id);
                }
            }
        }
        if missing.is_empty() {
            return Ok(result);
        }
        let encoded = serde_json::to_string(&missing)
            .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let rows = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            let mut statement = connection.prepare_cached(frame_sql)?;
            statement
                .query_map([encoded], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        HashKey {
                            algorithm: row.get(4)?,
                            lanes: [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
                        },
                        HashKey {
                            algorithm: row.get(9)?,
                            lanes: [row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?],
                        },
                        [row.get::<_, i64>(14)?, row.get::<_, i64>(15)?, row.get::<_, i64>(16)?],
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        // Decode and validate outside the database connection and transaction. Bytes in a segment are read in place,
        // their length checked; a row whose segment is gone is a miss.
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for (id, length, codec, frame, hash, source, [seg, seg_at, seg_len]) in rows {
            let Some(frame) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len).map_err(|_| StoreError::Corrupt(format!("unsupported {what} codec")))? else { continue };
            if let ObjectFrame::Raw(bytes) = frame {
                if usize::try_from(length).ok() != Some(bytes.len()) {
                    return Err(StoreError::Corrupt(format!("{what} length does not match metadata")));
                }
                result.insert(id, (Arc::new(CachedBytes::mapped(bytes)), false));
                continue;
            }
            let bytes = frame.bytes(0)?.into_vec();
            let identity = match objects {
                BulkObjects::Tapes => HashKey::tape_identity(source, &bytes),
                BulkObjects::Sources => HashKey::sha256(&bytes),
            };
            if usize::try_from(length).ok() != Some(bytes.len()) || identity != hash {
                return Err(StoreError::Corrupt(format!(
                    "{what} length or hash does not match metadata"
                )));
            }
            let bytes = self
                .decompressed_cache
                .lock()
                .expect("decompressed cache mutex poisoned")
                .insert_bytes(cache_key(hash), bytes)?;
            result.insert(id, (bytes, false));
        }
        Ok(result)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn get_tape(&self, tape_id: TapeId) -> StoreResult<Arc<CachedBytes>> {
        self.get_tape_with_status(tape_id).map(|(tape, _)| tape)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn get_tape_with_status(&self, tape_id: TapeId) -> StoreResult<(Arc<CachedBytes>, bool)> {
        let canonical_hash = {
            let connection = self
                .connection
                .lock()
                .expect("content store mutex poisoned");
            connection.query_row_cached(
                "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 FROM tape_objects t \
                 JOIN hashes h ON h.hash_id=t.canonical_hash_id WHERE t.tape_id=?1",
                [tape_id.0],
                |row| {
                    Ok(HashKey {
                        algorithm: row.get(0)?,
                        lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                    })
                },
            )?
        };
        let cache_key = CacheKey::Tape(canonical_hash);
        if let Some(tape) = self
            .decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .get(cache_key)
        {
            return Ok((tape, true));
        }
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let (raw_length, codec, frame, key, source, [seg, seg_at, seg_len]): (i64, i64, Vec<u8>, HashKey, HashKey, [i64; 3]) = connection.query_row_cached(
            "SELECT t.raw_length,t.codec_id,t.frame,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,\
             sh.algorithm_id,sh.lane0,sh.lane1,sh.lane2,sh.lane3,t.seg,t.seg_at,t.seg_len \
             FROM tape_objects t JOIN hashes h ON h.hash_id=t.canonical_hash_id \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes sh ON sh.hash_id=s.canonical_hash_id \
             WHERE t.tape_id=?1",
            [tape_id.0],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    HashKey {
                        algorithm: row.get(3)?,
                        lanes: [row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?],
                    },
                    HashKey {
                        algorithm: row.get(8)?,
                        lanes: [row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?],
                    },
                    [row.get(13)?, row.get(14)?, row.get(15)?],
                ))
            },
        )?;
        drop(connection);
        let raw_length = usize::try_from(raw_length)
            .map_err(|_| StoreError::Corrupt("tape length does not fit memory".to_owned()))?;
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let Some(frame) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len)? else {
            return Err(StoreError::Corrupt("a tape's segment is not there".to_owned()));
        };
        if let ObjectFrame::Raw(bytes) = frame {
            if bytes.len() != raw_length || key != canonical_hash || key != HashKey::tape_identity(source, bytes.bytes()) {
                return Err(StoreError::Corrupt("tape length or hash does not match metadata".to_owned()));
            }
            return Ok((Arc::new(CachedBytes::mapped(bytes)), false));
        }
        let tape = frame.bytes(0)?.into_vec();
        if tape.len() != raw_length {
            return Err(StoreError::Corrupt(
                "tape length does not match metadata".to_owned(),
            ));
        }
        if key != canonical_hash || key != HashKey::tape_identity(source, &tape) {
            return Err(StoreError::Corrupt(
                "tape hash does not match metadata".to_owned(),
            ));
        }
        let tape = self
            .decompressed_cache
            .lock()
            .expect("decompressed cache mutex poisoned")
            .insert_bytes(cache_key, tape)?;
        Ok((tape, false))
    }

    pub fn stored_objects_by_hash(
        &self,
        sources: &[HashKey],
        grammar_version: i64,
        tape_format_version: i64,
    ) -> StoreResult<HashMap<HashKey, StoredSourceObjects>> {
        let mut results = HashMap::<HashKey, StoredSourceObjects>::new();
        if sources.is_empty() {
            return Ok(results);
        }
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        for chunk in sources.chunks(1000) {
            let mut sql =
                String::from("WITH requested(algorithm_id,lane0,lane1,lane2,lane3) AS (VALUES ");
            let mut values = Vec::with_capacity(chunk.len() * 5 + 2);
            for (index, key) in chunk.iter().enumerate() {
                if index != 0 {
                    sql.push(',');
                }
                sql.push_str("(?,?,?,?,?)");
                values.push(rusqlite::types::Value::Integer(key.algorithm));
                values.extend(key.lanes.map(rusqlite::types::Value::Integer));
            }
            sql.push_str(
                ") SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,s.source_id,\
                 t.grammar_id,t.tape_id FROM requested q \
                 JOIN hashes h ON h.algorithm_id=q.algorithm_id AND h.lane0=q.lane0 \
                 AND h.lane1=q.lane1 AND h.lane2=q.lane2 AND h.lane3=q.lane3 \
                 JOIN source_objects s ON s.canonical_hash_id=h.hash_id \
                 LEFT JOIN tape_objects t ON t.source_id=s.source_id \
                 AND t.grammar_version=? AND t.tape_format_version=? \
                 ORDER BY s.source_id,t.grammar_id",
            );
            values.push(rusqlite::types::Value::Integer(grammar_version));
            values.push(rusqlite::types::Value::Integer(tape_format_version));
            let mut statement = connection.prepare(&sql)?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(values), |row| {
                    Ok((
                        HashKey {
                            algorithm: row.get(0)?,
                            lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?],
                        },
                        SourceId(row.get(5)?),
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (key, source, grammar, tape) in rows {
                let stored = results.entry(key).or_insert_with(|| StoredSourceObjects {
                    source,
                    tapes: HashMap::new(),
                });
                if let (Some(grammar), Some(tape)) = (grammar, tape) {
                    stored.tapes.insert(grammar, TapeId(tape));
                }
            }
        }
        Ok(results)
    }
}

/// Which objects `ContentStore::objects_bulk` reads.
#[derive(Clone, Copy)]
enum BulkObjects {
    Sources,
    Tapes,
}

pub(super) fn validate_tape_payload(tape: Option<TapePayload<'_>>) -> StoreResult<()> {
    let Some(tape) = tape else {
        return Ok(());
    };
    if tape.grammar_id < 0 || tape.grammar_version < 0 || tape.tape_format_version < 0 {
        return Err(StoreError::Invalid(
            "tape identifiers must be nonnegative".to_owned(),
        ));
    }
    for (index, document) in tape.documents.iter().enumerate() {
        let ordinal = i64::try_from(index + 1)
            .map_err(|_| StoreError::Invalid("too many derived documents".to_owned()))?;
        if document.grammar_id < 0
            || document.grammar_id == tape.grammar_id
            || document.role_id <= 0
            || !(0..ordinal).contains(&document.parent_ordinal)
        {
            return Err(StoreError::Invalid(
                "a derived document needs a grammar of its own, a role and an earlier parent"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

/// Each of `documents`' source and tape frames, compressed before a transaction starts.
pub(super) fn compress_documents(
    documents: &[DocumentPayload<'_>],
    level: i32,
) -> StoreResult<Vec<(Vec<u8>, Vec<u8>)>> {
    documents
        .iter()
        .map(|document| {
            Ok((
                compress_source(document.source, level, tape_tag(document.tape))?,
                compress_tape(document.tape, level)?,
            ))
        })
        .collect()
}

pub(super) fn put_source_tx(
    transaction: &Connection,
    source: &[u8],
    frame: &[u8],
    aliases: &[HashKey],
) -> StoreResult<SourceId> {
    let created_generation = generation(transaction)?;
    put_source_prepared_tx(
        transaction,
        HashKey::sha256(source),
        source.len(),
        frame,
        aliases,
        created_generation,
    )
}

fn put_source_prepared_tx(
    transaction: &Connection,
    canonical_hash: HashKey,
    source_length: usize,
    frame: &[u8],
    aliases: &[HashKey],
    created_generation: i64,
) -> StoreResult<SourceId> {
    let canonical_hash_id = intern_hash(transaction, canonical_hash)?;
    let raw_length = i64::try_from(source_length)
        .map_err(|_| StoreError::Invalid("source length exceeds i64".to_owned()))?;
    transaction
        .prepare_cached(
            "INSERT INTO source_objects(canonical_hash_id,raw_length,codec_id,frame,created_generation) \
             VALUES (?1,?2,?3,?4,?5) ON CONFLICT(canonical_hash_id) DO NOTHING",
        )?
        .execute(params![
            canonical_hash_id,
            raw_length,
            CODEC_ZSTD,
            frame,
            created_generation
        ])?;
    let source_id: i64 = transaction
        .prepare_cached("SELECT source_id FROM source_objects WHERE canonical_hash_id=?1")?
        .query_row([canonical_hash_id], |row| row.get(0))?;
    let source_id = SourceId(source_id);
    put_source_aliases_tx(transaction, source_id, aliases)?;
    Ok(source_id)
}

fn put_source_aliases_tx(
    transaction: &Connection,
    source_id: SourceId,
    aliases: &[HashKey],
) -> StoreResult<()> {
    for alias in aliases {
        let alias_hash_id = intern_hash(transaction, *alias)?;
        let existing = transaction
            .prepare_cached("SELECT source_id FROM source_aliases WHERE alias_hash_id=?1")?
            .query_row([alias_hash_id], |row| row.get::<_, i64>(0))
            .optional()?;
        if existing.is_some_and(|existing| existing != source_id.0) {
            return Err(StoreError::Corrupt(
                "Git object alias maps to different source bytes".to_owned(),
            ));
        }
        transaction
            .prepare_cached(
                "INSERT INTO source_aliases(alias_hash_id,source_id) VALUES (?1,?2) \
                 ON CONFLICT(alias_hash_id) DO NOTHING",
            )?
            .execute(params![alias_hash_id, source_id.0])?;
    }
    Ok(())
}

/// Stores `tape` over `source_id`, with the documents derived from it (`file_documents`), whose
/// source and tape frames `document_frames` holds in order.
pub(super) fn put_tape_tx(
    transaction: &Connection,
    source_id: SourceId,
    tape: TapePayload<'_>,
    frame: &[u8],
    document_frames: &[(Vec<u8>, Vec<u8>)],
) -> StoreResult<TapeId> {
    let created_generation = generation(transaction)?;
    let source_hash: HashKey = transaction.query_row_cached(
        "SELECT h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 FROM source_objects s \
         JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
        [source_id.0],
        |row| Ok(HashKey { algorithm: row.get(0)?, lanes: [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?] }),
    )?;
    let tape_id = put_tape_prepared_tx(
        transaction,
        source_id,
        tape,
        tape.bytes.len(),
        HashKey::tape_identity(source_hash, tape.bytes),
        frame,
        created_generation,
    )?;
    if tape.documents.len() != document_frames.len() {
        return Err(StoreError::Invalid(
            "every derived document needs its frames".to_owned(),
        ));
    }
    for (index, (document, (source_frame, tape_frame))) in
        tape.documents.iter().zip(document_frames).enumerate()
    {
        let document_source_hash = HashKey::sha256(document.source);
        let document_source = put_source_prepared_tx(
            transaction,
            document_source_hash,
            document.source.len(),
            source_frame,
            &[],
            created_generation,
        )?;
        let document_tape = put_tape_prepared_tx(
            transaction,
            document_source,
            TapePayload {
                grammar_id: document.grammar_id,
                grammar_version: tape.grammar_version,
                tape_format_version: tape.tape_format_version,
                bytes: document.tape,
                documents: &[],
            },
            document.tape.len(),
            HashKey::tape_identity(document_source_hash, document.tape),
            tape_frame,
            created_generation,
        )?;
        transaction
            .prepare_cached(
                "INSERT INTO file_documents(tape_id,ordinal,parent_ordinal,role_id,via_id,document_tape_id,source_map,host_errors) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(tape_id,ordinal) DO NOTHING",
            )?
            .execute(params![
                tape_id.0,
                index as i64 + 1,
                document.parent_ordinal,
                document.role_id,
                document.via_id,
                document_tape.0,
                document.source_map,
                document.host_errors,
            ])?;
    }
    Ok(tape_id)
}

fn put_tape_prepared_tx(
    transaction: &Connection,
    source_id: SourceId,
    tape: TapePayload<'_>,
    raw_length: usize,
    canonical_hash: HashKey,
    frame: &[u8],
    created_generation: i64,
) -> StoreResult<TapeId> {
    validate_tape_payload(Some(tape))?;
    let canonical_hash_id = intern_hash(transaction, canonical_hash)?;
    let raw_length = i64::try_from(raw_length)
        .map_err(|_| StoreError::Invalid("tape length exceeds i64".to_owned()))?;
    transaction
        .prepare_cached(
            "INSERT INTO tape_objects(source_id,canonical_hash_id,grammar_id,grammar_version,tape_format_version,codec_id,raw_length,frame,created_generation) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
             ON CONFLICT(source_id,grammar_id,grammar_version,tape_format_version) DO NOTHING",
        )?
        .execute(params![
            source_id.0,
            canonical_hash_id,
            tape.grammar_id,
            tape.grammar_version,
            tape.tape_format_version,
            super::compression::tape_codec(frame),
            raw_length,
            frame,
            created_generation,
        ])?;
    let (tape_id, stored_hash_id): (i64, i64) = transaction
        .prepare_cached(
            "SELECT tape_id,canonical_hash_id FROM tape_objects WHERE source_id=?1 AND grammar_id=?2 \
             AND grammar_version=?3 AND tape_format_version=?4",
        )?
        .query_row(
            params![
                source_id.0,
                tape.grammar_id,
                tape.grammar_version,
                tape.tape_format_version
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    if stored_hash_id != canonical_hash_id {
        return Err(StoreError::Corrupt(
            "tape identity maps to different tape bytes".to_owned(),
        ));
    }
    Ok(TapeId(tape_id))
}

pub(super) fn source_id_by_hash(connection: &Connection, key: HashKey) -> StoreResult<Option<SourceId>> {
    connection
        .query_row_cached(
            "SELECT s.source_id FROM source_objects s JOIN hashes h ON h.hash_id=s.canonical_hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5",
            hash_params(key),
            |row| row.get::<_, i64>(0).map(SourceId),
        )
        .optional()
        .map_err(StoreError::from)
}

/// A stored tape as its row has it (`tape_frames_for`): a frame before it is decompressed, or its bytes in a segment.
#[derive(Clone, Debug)]
pub struct TapeFrame {
    raw_length: i64,
    frame: ObjectFrame,
}

impl TapeFrame {
    /// The tape's stored bytes where the frame holds them as they are (a segment's range, or a plain frame), read in
    /// place: `None` for a compressed frame, which only a decompression reads.
    pub fn in_place(&self) -> Option<&[u8]> {
        match &self.frame {
            ObjectFrame::Raw(b) => Some(b.bytes()),
            ObjectFrame::Plain(f) => Some(f),
            ObjectFrame::Zstd(_) => None,
        }
    }

    /// The tape's bytes: decompressed, or read in place.
    pub fn bytes(&self) -> StoreResult<super::ObjectBytes> {
        // The frame may name a language's dictionary (RER-131's `compression`), which `decompress` reads.
        let bytes = self.frame.bytes(usize::try_from(self.raw_length).unwrap_or(0))?;
        if usize::try_from(self.raw_length).ok() != Some(bytes.len()) {
            return Err(StoreError::Corrupt("tape length does not match metadata".to_owned()));
        }
        Ok(bytes)
    }

    /// The frame's size, as stored.
    pub fn len(&self) -> usize {
        self.frame.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frame.is_empty()
    }

    /// The tape's length, as its metadata gives it.
    pub fn raw_len(&self) -> StoreResult<usize> {
        usize::try_from(self.raw_length).map_err(|_| StoreError::Corrupt("a tape's length".to_owned()))
    }

    /// The tape's bytes into `out` (`raw_len` of them): decompressed straight there, or read from its segment
    /// (decision 0353 step 4a: a tape's load writes its one home).
    pub fn copy_into(&self, out: &mut [u8]) -> std::io::Result<()> {
        self.frame.copy_into(out)
    }
}
