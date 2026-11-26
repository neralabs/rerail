//! Content-addressed source and tape objects: their rows, aliases, and bulk reads. Every object's bytes are in a
//! segment of the store's objects, as they are (`object_segments`).

#[allow(unused_imports)]
use crate::content_store::Cached as _;
use super::rows::{generation, hash_params, intern_hash};
use super::object_segments::{ObjectFrame, Placing, object_dir, row_frame};
use super::reclamation::{OBJECT_SOURCE, OBJECT_TAPE};
use super::segments::Place;
use super::segments::SegmentMaps;
use super::{
    CachedBytes, ContentStore, DocumentPayload, HASH_SHA256, HashKey, SourceId,
    StoreError, StoreResult, StoredDocument, StoredPathObjects, StoredProgram, StoredSourceObjects, TapeId,
    TapePayload,
};
#[allow(unused_imports)]
use crate::content_store::Cached as _;
use rusqlite::Connection;
use rusqlite::{OptionalExtension, params};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// A stored tape's frames as residency admits them (PIAT §20.3): the tape's and its source's, in their segments, with
/// their raw lengths, the source's SHA-256 and the tape's grammar.
pub struct ResidentFrames {
    pub grammar_id: i64,
    pub tape_length: usize,
    pub tape_frame: ObjectFrame,
    pub source_length: usize,
    pub source_frame: ObjectFrame,
    pub source_sha256: [u8; 32],
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
                    serde_json::json!([
                        at,
                        key.algorithm,
                        key.lanes[0],
                        key.lanes[1],
                        key.lanes[2],
                        key.lanes[3]
                    ])
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::Invalid(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) at, \
             CAST(json_extract(value,'$[1]') AS INTEGER) algorithm, CAST(json_extract(value,'$[2]') AS INTEGER) lane0, \
             CAST(json_extract(value,'$[3]') AS INTEGER) lane1, CAST(json_extract(value,'$[4]') AS INTEGER) lane2, \
             CAST(json_extract(value,'$[5]') AS INTEGER) lane3 FROM json_each(?1)) \
             SELECT i.at,t.grammar_id,t.raw_length,s.raw_length,\
             sh.algorithm_id,sh.lane0,sh.lane1,sh.lane2,sh.lane3,t.seg,t.seg_at,t.seg_len,s.seg,s.seg_at,s.seg_len,t.tape_id,s.source_id \
             FROM input i CROSS JOIN hashes th INDEXED BY hashes_by_prefix ON th.algorithm_id=i.algorithm AND th.lane0=i.lane0 \
             AND th.lane1=i.lane1 AND th.lane2=i.lane2 AND th.lane3=i.lane3 \
             JOIN tape_objects t ON t.canonical_hash_id=th.hash_id \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes sh ON sh.hash_id=s.canonical_hash_id",
        )?;
        let mut out = (0..tapes.len()).map(|_| None).collect::<Vec<_>>();
        type Row = ((i64, i64, usize, usize, [u8; 32], [i64; 3], [i64; 3]), [i64; 2]);
        let rows: Vec<Row> = statement
            .query_map([encoded], |row| {
                let length = |at: usize| {
                    row.get::<_, i64>(at)
                        .map(|length| usize::try_from(length).unwrap_or(usize::MAX))
                };
                Ok((
                    (
                        row.get::<_, i64>(0)?,
                        row.get(1)?,
                        length(2)?,
                        length(3)?,
                        HashKey { algorithm: row.get(4)?, lanes: [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?] }.sha256_bytes().unwrap_or_default(),
                        [row.get(9)?, row.get(10)?, row.get(11)?],
                        [row.get(12)?, row.get(13)?, row.get(14)?],
                    ),
                    [row.get(15)?, row.get(16)?],
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        let archived = rows.iter().flat_map(|((.., t, s), [tape, source])| [(OBJECT_TAPE, *tape, t[0]), (OBJECT_SOURCE, *source, s[0])]);
        if self.placed_again(archived)? {
            return self.resident_frames(tapes);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for ((at, grammar_id, tape_length, source_length, source_sha256, t, s), _) in rows {
            let (Some(tape_frame), Some(source_frame)) = (row_frame(&mut maps, t[0], t[1], t[2]), row_frame(&mut maps, s[0], s[1], s[2])) else { continue };
            if let Some(slot) = usize::try_from(at).ok().and_then(|at| out.get_mut(at)) {
                *slot = Some(ResidentFrames {
                    grammar_id,
                    tape_length,
                    tape_frame,
                    source_length,
                    source_frame,
                    source_sha256,
                });
            }
        }
        Ok(out)
    }
    pub fn put_source(&self, source: &[u8], aliases: &[HashKey]) -> StoreResult<SourceId> {
        self.put_source_hashed(source, HashKey::sha256(source), aliases)
    }

    /// [`Self::put_source`] for a caller that already holds the source's SHA-256 (`canonical`): the bytes are not
    /// hashed again. Stored as it is, in a segment of its own (`put_sources_placed`); one stored already keeps its row.
    pub fn put_source_hashed(&self, source: &[u8], canonical: HashKey, aliases: &[HashKey]) -> StoreResult<SourceId> {
        Ok(self.put_sources_placed(&[(source, canonical, aliases)])?.remove(0))
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
        let list = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>())
            .expect("integers");
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT j.key, t.raw_length, t.seg, t.seg_at, t.seg_len, t.tape_id, t.programs FROM json_each(?1) j JOIN tape_objects t \
             ON t.source_id=j.value AND t.grammar_id=?2 AND t.grammar_version=?3 AND t.tape_format_version=?4",
        )?;
        type Row = (i64, i64, [i64; 3], (TapeId, i64));
        let rows: Vec<Row> = statement
            .query_map(params![list, grammar_id, grammar_version, tape_format_version], |row| {
                Ok((row.get(0)?, row.get(1)?, [row.get(2)?, row.get(3)?, row.get(4)?], (TapeId(row.get(5)?), row.get(6)?)))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        if self.placed_again(rows.iter().map(|&(_, _, [seg, ..], (tape, _))| (OBJECT_TAPE, tape.0, seg)))? {
            return self.tape_frames_for(sources, grammar_id, grammar_version, tape_format_version);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut recorded: Vec<(usize, TapeId, i64)> = Vec::new();
        for (at, raw_length, [seg, seg_at, seg_len], (tape, programs)) in rows {
            // A tape whose segment is gone is a miss: parsed again.
            if let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) {
                out[at as usize] = Some(TapeFrame::new(raw_length, frame));
                recorded.push((at as usize, tape, programs));
            }
        }
        let mut frames: Vec<(TapeId, i64, &mut TapeFrame)> = Vec::with_capacity(recorded.len());
        let mut slots: Vec<Option<&mut TapeFrame>> = out.iter_mut().map(Option::as_mut).collect();
        for (at, tape, programs) in recorded {
            if let Some(frame) = slots[at].take() {
                frames.push((tape, programs, frame));
            }
        }
        self.attach_programs(&mut frames)?;
        Ok(out)
    }

    /// Gives each of `frames` (its tape's id and count of programs) the programs recorded with it (decision 0431
    /// step 6): each its `file_documents` row, its text and its tape's frame, in ordinal order. A tape that records
    /// none has an empty list; one whose programs are not known (-1: stored without them), or a program of which is
    /// missing, has none (its file's programs are made again). One read for all, nothing written.
    fn attach_programs(&self, frames: &mut [(TapeId, i64, &mut TapeFrame)]) -> StoreResult<()> {
        let holding: Vec<TapeId> = frames.iter().filter(|(_, n, _)| *n > 0).map(|(t, _, _)| *t).collect();
        let mut documents = self.file_documents_bulk(&holding)?;
        let texts = self.get_sources_bulk(&documents.values().flatten().filter_map(|d| d.source).collect::<Vec<_>>())?;
        let tapes = self.tape_frames_by_id(&documents.values().flatten().map(|d| d.tape).collect::<Vec<_>>())?;
        for (tape, count, frame) in frames.iter_mut() {
            if *count < 0 {
                continue;
            }
            let rows = documents.remove(tape).unwrap_or_default();
            if usize::try_from(*count).ok() != Some(rows.len()) {
                continue;
            }
            let programs: Option<Vec<StoredProgram>> = rows
                .into_iter()
                .map(|document| {
                    let text = Arc::clone(texts.get(&document.source?)?);
                    let tape = tapes.get(&document.tape)?.clone();
                    Some(StoredProgram { document, text, tape })
                })
                .collect();
            frame.set_programs(programs.map(Into::into));
        }
        Ok(())
    }

    /// The frames of the tapes `ids` name; a tape whose segment is gone is left out.
    fn tape_frames_by_id(&self, ids: &[TapeId]) -> StoreResult<HashMap<TapeId, TapeFrame>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let list = serde_json::to_string(&ids.iter().map(|t| t.0).collect::<Vec<_>>()).expect("integers");
        let rows: Vec<(i64, i64, [i64; 3])> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut statement = connection.prepare_cached(
                "SELECT t.tape_id, t.raw_length, t.seg, t.seg_at, t.seg_len FROM json_each(?1) j JOIN tape_objects t ON t.tape_id=j.value",
            )?;
            statement
                .query_map([list], |row| Ok((row.get(0)?, row.get(1)?, [row.get(2)?, row.get(3)?, row.get(4)?])))?
                .collect::<Result<_, _>>()?
        };
        // An archived tape is placed back first (decision 0450 step 4), as every reader a warm run uses does.
        if self.placed_again(rows.iter().map(|&(id, _, [seg, ..])| (OBJECT_TAPE, id, seg)))? {
            return self.tape_frames_by_id(ids);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for (id, raw_length, [seg, seg_at, seg_len]) in rows {
            if let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) {
                out.insert(TapeId(id), TapeFrame::new(raw_length, frame));
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
            "SELECT t.source_id, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, t.raw_length, t.seg, t.seg_at, t.seg_len, t.tape_id, t.programs \
             FROM json_each(?1) j JOIN tape_objects t ON t.source_id=j.value AND t.grammar_id=?2 AND t.grammar_version=?3 AND t.tape_format_version=?4 \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes h ON h.hash_id=s.canonical_hash_id",
        )?;
        type Row = (SourceId, HashKey, i64, [i64; 3], (TapeId, i64));
        let rows: Vec<Row> = statement
            .query_map(params![list, grammar_id, grammar_version, tape_format_version], |row| {
                Ok((
                    SourceId(row.get(0)?),
                    HashKey { algorithm: row.get(1)?, lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?] },
                    row.get(6)?,
                    [row.get(7)?, row.get(8)?, row.get(9)?],
                    (TapeId(row.get(10)?), row.get(11)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        if self.placed_again(rows.iter().map(|&(_, _, _, [seg, ..], (tape, _))| (OBJECT_TAPE, tape.0, seg)))? {
            return self.tape_frames_with_hashes(sources, grammar_id, grammar_version, tape_format_version);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut out = Vec::new();
        let mut recorded: Vec<(TapeId, i64)> = Vec::new();
        for (source, hash, raw_length, [seg, seg_at, seg_len], (tape, programs)) in rows {
            // A tape whose segment is gone is a miss: parsed again.
            if let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) {
                out.push((source, hash, TapeFrame::new(raw_length, frame)));
                recorded.push((tape, programs));
            }
        }
        let mut frames: Vec<(TapeId, i64, &mut TapeFrame)> = out.iter_mut().zip(recorded).map(|((_, _, frame), (tape, programs))| (tape, programs, frame)).collect();
        self.attach_programs(&mut frames)?;
        Ok(out)
    }

    /// [`Self::tape_frames_with_hashes`] in every grammar, each tape with its grammar's id: the tapes of the values a
    /// run's rules parsed (decision 0431 R6), where the program's requests do not name their grammars.
    pub fn tape_frames_any_grammar(&self, sources: &[SourceId], grammar_version: i64, tape_format_version: i64) -> StoreResult<Vec<(SourceId, HashKey, i64, TapeFrame)>> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let list = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("integers");
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT t.source_id, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, t.grammar_id, t.raw_length, t.seg, t.seg_at, t.seg_len, t.tape_id \
             FROM json_each(?1) j JOIN tape_objects t ON t.source_id=j.value AND t.grammar_version=?2 AND t.tape_format_version=?3 \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes h ON h.hash_id=s.canonical_hash_id",
        )?;
        type Row = (SourceId, HashKey, i64, i64, [i64; 3], i64);
        let rows: Vec<Row> = statement
            .query_map(params![list, grammar_version, tape_format_version], |row| {
                Ok((
                    SourceId(row.get(0)?),
                    HashKey { algorithm: row.get(1)?, lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?] },
                    row.get(6)?,
                    row.get(7)?,
                    [row.get(8)?, row.get(9)?, row.get(10)?],
                    row.get(11)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(statement);
        drop(connection);
        if self.placed_again(rows.iter().map(|&(_, _, _, _, [seg, ..], tape)| (OBJECT_TAPE, tape, seg)))? {
            return self.tape_frames_any_grammar(sources, grammar_version, tape_format_version);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut out = Vec::new();
        for (source, hash, grammar, raw_length, [seg, seg_at, seg_len], _) in rows {
            // A tape whose segment is gone is a miss: parsed again.
            if let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) {
                out.push((source, hash, grammar, TapeFrame::new(raw_length, frame)));
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
        for (chunk, lane) in digest.as_chunks_mut::<8>().0.iter_mut().zip(key.lanes) {
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
        let ids = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>())
            .expect("integers");
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            "SELECT j.key, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3 FROM json_each(?1) j \
             CROSS JOIN source_objects s ON s.source_id = j.value JOIN hashes h ON h.hash_id = s.canonical_hash_id",
        )?;
        let rows = statement.query_map([ids], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                HashKey {
                    algorithm: row.get(1)?,
                    lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                },
            ))
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
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let (raw_length, key, [seg, seg_at, seg_len]): (i64, HashKey, [i64; 3]) = connection.query_row_cached(
            "SELECT s.raw_length, h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, s.seg, s.seg_at, s.seg_len \
             FROM source_objects s JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id=?1",
            [source_id.0],
            |row| Ok((row.get(0)?, HashKey { algorithm: row.get(1)?, lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?] }, [row.get(6)?, row.get(7)?, row.get(8)?])),
        )?;
        drop(connection);
        if self.placed_again([(OBJECT_SOURCE, source_id.0, seg)])? {
            return self.get_source(source_id);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) else {
            return Err(StoreError::Corrupt("a source's segment is not there".to_owned()));
        };
        if usize::try_from(raw_length).ok() != Some(frame.len()) || key != HashKey::sha256(frame.segment().bytes()) {
            return Err(StoreError::Corrupt("source length or hash does not match metadata".to_owned()));
        }
        Ok(frame.segment().bytes().to_vec())
    }

    /// Read immutable source IDs qualified by the canonical hashes from bulk metadata, in bounded batches.
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
        let mut ids: Vec<i64> = expected.keys().map(|id| id.0).collect();
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
                    "SELECT s.source_id,s.raw_length,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,s.seg,s.seg_at,s.seg_len FROM source_objects s JOIN hashes h ON h.hash_id=s.canonical_hash_id WHERE s.source_id IN ({placeholders})"
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
                        HashKey {
                            algorithm: row.get(2)?,
                            lanes: [row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?],
                        },
                        [row.get::<_, i64>(7)?, row.get::<_, i64>(8)?, row.get::<_, i64>(9)?],
                    ));
                }
                frames
            };
            if self.placed_again(frames.iter().map(|&(id, _, _, [seg, ..])| (OBJECT_SOURCE, id.0, seg)))? {
                return self.get_sources_by_hash_bulk(requests, max_bytes);
            }
            let mut maps = SegmentMaps::new(object_dir(&self.database_path));
            for (id, length, hash, [seg, seg_at, seg_len]) in frames {
                if expected[&id] != hash {
                    return Err(StoreError::Corrupt(
                        "source identity does not match metadata".into(),
                    ));
                }
                let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) else { continue };
                if frame.len() != length {
                    return Err(StoreError::Corrupt("source length does not match metadata".into()));
                }
                result.insert(id, frame.segment().bytes().to_vec());
            }
        }
        Ok(result)
    }

    /// A tape of `source_id`, stored as it is in a segment of its own (`Placing`); one stored already keeps its row.
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
        let mut placing = Placing::write(&self.database_path, &[tape], self.connection.flush())?;
        // Retried under `retry_busy`, as `put_source` is.
        let (id, seg) = self.immediate_transaction(|transaction| {
            let seg = placing.number(transaction)?.expect("the tape, placed");
            let id = put_tape_tx(
                transaction,
                source_id,
                TapePayload {
                    grammar_id,
                    grammar_version,
                    tape_format_version,
                    bytes: tape,
                    documents: &[],
                    whole: false,
                },
                placing.place(seg, 0),
                &[],
            )?;
            Ok((id, seg))
        })?;
        placing.name(Some(seg))?;
        Ok(id)
    }

    /// Records that the tapes of `tapes` (each a source and its grammar: id, version, tape format) have no programs:
    /// their loads ran the embedders and found none (decision 0431 step 6). A tape whose programs are recorded, or
    /// that is not stored, is left as it is. One transaction for all. Read back by `programs_counts` (decision 0563
    /// A: a run whose documents embed nothing admits no guest) and by a load, which then embeds none again.
    pub fn put_no_programs(&self, tapes: &[(SourceId, (i64, i64, i64))]) -> StoreResult<()> {
        if tapes.is_empty() {
            return Ok(());
        }
        self.require_write_capacity()?;
        self.immediate_transaction(|transaction| {
            let mut update = transaction.prepare_cached(
                "UPDATE tape_objects SET programs=0 WHERE source_id=?1 AND grammar_id=?2 AND grammar_version=?3 AND tape_format_version=?4 AND programs<0",
            )?;
            for &(source, (grammar_id, grammar_version, tape_format_version)) in tapes {
                update.execute(params![source.0, grammar_id, grammar_version, tape_format_version])?;
            }
            Ok(())
        })
    }

    /// Each text's count of programs in its grammar (`(its SHA-256, its grammar id)`, at `grammar_version` and
    /// `tape_format_version`): `None` where its tape is not stored, or stored without its programs known (-1).
    pub fn programs_counts(&self, texts: &[(HashKey, i64)], grammar_version: i64, tape_format_version: i64) -> StoreResult<Vec<Option<i64>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut statement = connection.prepare_cached(
            // The text's source, then its tape: a tape's own hash is its identity (`tape_identity`), not its text's.
            "SELECT t.programs FROM hashes h JOIN source_objects s ON s.canonical_hash_id = h.hash_id \
             JOIN tape_objects t ON t.source_id = s.source_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 \
             AND t.grammar_id=?6 AND t.grammar_version=?7 AND t.tape_format_version=?8",
        )?;
        texts
            .iter()
            .map(|&(hash, grammar_id)| {
                let [a, l0, l1, l2, l3] = hash_params(hash);
                statement
                    .query_row(params![a, l0, l1, l2, l3, grammar_id, grammar_version, tape_format_version], |row| row.get::<_, i64>(0))
                    .optional()
                    .map(|n| n.filter(|&n| n >= 0))
                    .map_err(StoreError::from)
            })
            .collect()
    }

    /// Records the programs of `source`'s tape in `grammar` (grammar id, version, tape format; decision 0431 step 6):
    /// each a source and a tape of its own, placed as they are in one segment (`Placing`), and a `file_documents`
    /// row, and the tape's count of them, in one transaction. A tape whose programs are recorded, or that is not
    /// stored, is left as it is.
    pub fn put_programs(&self, source: SourceId, grammar: (i64, i64, i64), documents: &[DocumentPayload<'_>]) -> StoreResult<()> {
        self.require_write_capacity()?;
        let (grammar_id, grammar_version, tape_format_version) = grammar;
        let count = i64::try_from(documents.len()).map_err(|_| StoreError::Invalid("too many derived documents".to_owned()))?;
        let entries: Vec<&[u8]> = documents.iter().flat_map(|d| [d.source, d.tape]).collect();
        let mut placing = Placing::write(&self.database_path, &entries, self.connection.flush())?;
        let seg = self.immediate_transaction(|transaction| {
            let found: Option<(i64, i64)> = transaction
                .prepare_cached("SELECT tape_id,programs FROM tape_objects WHERE source_id=?1 AND grammar_id=?2 AND grammar_version=?3 AND tape_format_version=?4")?
                .query_row(params![source.0, grammar_id, grammar_version, tape_format_version], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let Some((tape, programs)) = found else { return Ok(None) };
            if programs >= 0 {
                return Ok(None);
            }
            let created_generation = generation(transaction)?;
            let seg = placing.number(transaction)?;
            let places: Vec<(Place, Place)> = match seg {
                Some(seg) => (0..documents.len()).map(|i| (placing.place(seg, 2 * i), placing.place(seg, 2 * i + 1))).collect(),
                None => Vec::new(),
            };
            put_documents_tx(transaction, TapeId(tape), grammar_version, tape_format_version, documents, &places, created_generation)?;
            transaction.prepare_cached("UPDATE tape_objects SET programs=?2 WHERE tape_id=?1")?.execute(params![tape, count])?;
            Ok(seg)
        })?;
        placing.name(seg)?;
        Ok(())
    }

    /// [`Self::put_source_hashed`] and [`Self::put_tape`] in one transaction: the source (`canonical` its SHA-256),
    /// stored or found, and its tape, both as they are in one segment of their own. A source looked up earlier may have
    /// been collected since (a pass deletes a source no tape, pin or live manifest holds); in one transaction no pass
    /// can come between the source and the tape that holds it (O95). Returns the source's id as it is now.
    pub fn put_source_and_tape(
        &self,
        source: &[u8],
        canonical: HashKey,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
        tape: &[u8],
    ) -> StoreResult<(SourceId, TapeId)> {
        self.require_write_capacity()?;
        if grammar_id < 0 || grammar_version < 0 || tape_format_version < 0 {
            return Err(StoreError::Invalid(
                "tape identifiers must be nonnegative".to_owned(),
            ));
        }
        let tape_identity = HashKey::tape_identity(canonical, tape);
        let mut placing = Placing::write(&self.database_path, &[source, tape], self.connection.flush())?;
        let (ids, seg) = self.immediate_transaction(|transaction| {
            let created_generation = generation(transaction)?;
            let seg = placing.number(transaction)?.expect("the source and tape, placed");
            let source_id = put_source_prepared_tx(
                transaction,
                canonical,
                source.len(),
                placing.place(seg, 0),
                &[],
                created_generation,
            )?;
            // The source's hash is `canonical` (stored or found under it), so the tape's identity was made with it.
            let tape_id = put_tape_prepared_tx(
                transaction,
                source_id,
                TapePayload { grammar_id, grammar_version, tape_format_version, bytes: &[], documents: &[], whole: false },
                tape.len(),
                tape_identity,
                placing.place(seg, 1),
                created_generation,
            )?;
            Ok(((source_id, tape_id), seg))
        })?;
        placing.name(Some(seg))?;
        Ok(ids)
    }

    pub fn get_tapes_bulk(
        &self,
        tape_ids: &[TapeId],
    ) -> StoreResult<HashMap<TapeId, Arc<CachedBytes>>> {
        let ids = tape_ids.iter().map(|id| id.0).collect::<BTreeSet<_>>();
        Ok(self
            .objects_bulk(&ids, BulkObjects::Tapes)?
            .into_iter()
            .map(|(id, object)| (TapeId(id), object))
            .collect())
    }

    /// The sources `source_ids` name, read in place; a source that is not stored is left out.
    pub fn get_sources_bulk(
        &self,
        source_ids: &[SourceId],
    ) -> StoreResult<HashMap<SourceId, Arc<CachedBytes>>> {
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
             t.source_id,t.grammar_id,d.source_map,d.host_errors,d.at_start,d.at_end FROM requested r \
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
                    at: match (row.get::<_, Option<i64>>(10)?, row.get::<_, Option<i64>>(11)?) {
                        (Some(start), Some(end)) => u32::try_from(start).ok().zip(u32::try_from(end).ok()),
                        _ => None,
                    },
                },
            ))
        })?;
        for row in rows {
            let (tape, document) = row?;
            documents.entry(tape).or_default().push(document);
        }
        Ok(documents)
    }

    /// The stored objects `ids` name, read in place in their segments, each one's length checked. A missing row (or
    /// segment) is left out: callers parse again rather than issuing a point lookup.
    fn objects_bulk(
        &self,
        ids: &BTreeSet<i64>,
        objects: BulkObjects,
    ) -> StoreResult<HashMap<i64, Arc<CachedBytes>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let (frame_sql, what) = match objects {
            BulkObjects::Tapes => (
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT t.tape_id,t.raw_length,t.seg,t.seg_at,t.seg_len \
                 FROM requested r JOIN tape_objects t ON t.tape_id=r.id",
                "tape",
            ),
            BulkObjects::Sources => (
                "WITH requested AS (SELECT CAST(value AS INTEGER) id FROM json_each(?1)) \
                 SELECT s.source_id,s.raw_length,s.seg,s.seg_at,s.seg_len \
                 FROM requested r JOIN source_objects s ON s.source_id=r.id",
                "source",
            ),
        };
        let encoded = serde_json::to_string(ids).map_err(|error| StoreError::Invalid(error.to_string()))?;
        let mut result = HashMap::new();
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
                        [row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?],
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let kind = match objects {
            BulkObjects::Tapes => OBJECT_TAPE,
            BulkObjects::Sources => OBJECT_SOURCE,
        };
        if self.placed_again(rows.iter().map(|&(id, _, [seg, ..])| (kind, id, seg)))? {
            return self.objects_bulk(ids, objects);
        }
        // Read in their segments, in place, outside the database connection: each one's length checked; a row whose
        // segment is gone is a miss.
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        for (id, length, [seg, seg_at, seg_len]) in rows {
            let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) else { continue };
            if usize::try_from(length).ok() != Some(frame.len()) {
                return Err(StoreError::Corrupt(format!("{what} length does not match metadata")));
            }
            result.insert(id, Arc::new(CachedBytes::mapped(frame.segment().clone())));
        }
        Ok(result)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn get_tape(&self, tape_id: TapeId) -> StoreResult<Arc<CachedBytes>> {
        let connection = self
            .connection
            .lock()
            .expect("content store mutex poisoned");
        let (raw_length, key, source, [seg, seg_at, seg_len]): (i64, HashKey, HashKey, [i64; 3]) = connection.query_row_cached(
            "SELECT t.raw_length,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3,\
             sh.algorithm_id,sh.lane0,sh.lane1,sh.lane2,sh.lane3,t.seg,t.seg_at,t.seg_len \
             FROM tape_objects t JOIN hashes h ON h.hash_id=t.canonical_hash_id \
             JOIN source_objects s ON s.source_id=t.source_id JOIN hashes sh ON sh.hash_id=s.canonical_hash_id \
             WHERE t.tape_id=?1",
            [tape_id.0],
            |row| {
                Ok((
                    row.get(0)?,
                    HashKey {
                        algorithm: row.get(1)?,
                        lanes: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                    },
                    HashKey {
                        algorithm: row.get(6)?,
                        lanes: [row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?],
                    },
                    [row.get(11)?, row.get(12)?, row.get(13)?],
                ))
            },
        )?;
        drop(connection);
        if self.placed_again([(OBJECT_TAPE, tape_id.0, seg)])? {
            return self.get_tape(tape_id);
        }
        let raw_length = usize::try_from(raw_length)
            .map_err(|_| StoreError::Corrupt("tape length does not fit memory".to_owned()))?;
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) else {
            return Err(StoreError::Corrupt("a tape's segment is not there".to_owned()));
        };
        let bytes = frame.segment().clone();
        if bytes.len() != raw_length || key != HashKey::tape_identity(source, bytes.bytes()) {
            return Err(StoreError::Corrupt("tape length or hash does not match metadata".to_owned()));
        }
        Ok(Arc::new(CachedBytes::mapped(bytes)))
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

/// `source`, whose bytes are at `place`, stored (or found by its hash), with its aliases.
pub(super) fn put_source_tx(
    transaction: &Connection,
    source: &[u8],
    place: Place,
    aliases: &[HashKey],
) -> StoreResult<SourceId> {
    let created_generation = generation(transaction)?;
    put_source_prepared_tx(
        transaction,
        HashKey::sha256(source),
        source.len(),
        place,
        aliases,
        created_generation,
    )
}

/// A source of `source_length` bytes at `place` (a segment's entry): inserted, or kept where the store holds it
/// already; one whose segment went missing (`seg` -1) is moved to `place`, whole again.
fn put_source_prepared_tx(
    transaction: &Connection,
    canonical_hash: HashKey,
    source_length: usize,
    place: Place,
    aliases: &[HashKey],
    created_generation: i64,
) -> StoreResult<SourceId> {
    let canonical_hash_id = intern_hash(transaction, canonical_hash)?;
    let raw_length = i64::try_from(source_length)
        .map_err(|_| StoreError::Invalid("source length exceeds i64".to_owned()))?;
    transaction
        .prepare_cached(
            "INSERT INTO source_objects(canonical_hash_id,raw_length,codec_id,frame,created_generation,seg,seg_at,seg_len) \
             VALUES (?1,?2,?3,x'',?4,?5,?6,?7) ON CONFLICT(canonical_hash_id) DO UPDATE SET seg=excluded.seg, \
             seg_at=excluded.seg_at, seg_len=excluded.seg_len WHERE source_objects.seg < 0",
        )?
        .execute(params![
            canonical_hash_id,
            raw_length,
            super::object_segments::CODEC_RAW,
            created_generation,
            place.seg,
            place.at as i64,
            place.len as i64,
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

/// Stores `tape` (its bytes at `place`) over `source_id`, with the documents derived from it (`file_documents`),
/// whose sources' and tapes' places `document_places` holds in order.
pub(super) fn put_tape_tx(
    transaction: &Connection,
    source_id: SourceId,
    tape: TapePayload<'_>,
    place: Place,
    document_places: &[(Place, Place)],
) -> StoreResult<TapeId> {
    let created_generation = generation(transaction)?;
    let source_hash: HashKey = transaction.query_row_cached(
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
    let tape_id = put_tape_prepared_tx(
        transaction,
        source_id,
        tape,
        tape.bytes.len(),
        HashKey::tape_identity(source_hash, tape.bytes),
        place,
        created_generation,
    )?;
    put_documents_tx(transaction, tape_id, tape.grammar_version, tape.tape_format_version, tape.documents, document_places, created_generation)?;
    Ok(tape_id)
}

/// Stores the documents derived from tape `tape_id` (`file_documents`), each its source and tape, whose places
/// `document_places` holds in order, numbered from 1.
fn put_documents_tx(
    transaction: &Connection,
    tape_id: TapeId,
    grammar_version: i64,
    tape_format_version: i64,
    documents: &[DocumentPayload<'_>],
    document_places: &[(Place, Place)],
    created_generation: i64,
) -> StoreResult<()> {
    if documents.len() != document_places.len() {
        return Err(StoreError::Invalid(
            "every derived document needs its places".to_owned(),
        ));
    }
    for (index, (document, &(source_place, tape_place))) in
        documents.iter().zip(document_places).enumerate()
    {
        let document_source_hash = HashKey::sha256(document.source);
        let document_source = put_source_prepared_tx(
            transaction,
            document_source_hash,
            document.source.len(),
            source_place,
            &[],
            created_generation,
        )?;
        let document_tape = put_tape_prepared_tx(
            transaction,
            document_source,
            TapePayload {
                grammar_id: document.grammar_id,
                grammar_version,
                tape_format_version,
                bytes: document.tape,
                documents: &[],
                whole: false,
            },
            document.tape.len(),
            HashKey::tape_identity(document_source_hash, document.tape),
            tape_place,
            created_generation,
        )?;
        transaction
            .prepare_cached(
                "INSERT INTO file_documents(tape_id,ordinal,parent_ordinal,role_id,via_id,document_tape_id,source_map,host_errors,at_start,at_end) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(tape_id,ordinal) DO NOTHING",
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
                document.at.map(|(start, _)| i64::from(start)),
                document.at.map(|(_, end)| i64::from(end)),
            ])?;
    }
    Ok(())
}

/// A tape of `raw_length` bytes at `place` (a segment's entry), as a source is (`put_source_prepared_tx`).
fn put_tape_prepared_tx(
    transaction: &Connection,
    source_id: SourceId,
    tape: TapePayload<'_>,
    raw_length: usize,
    canonical_hash: HashKey,
    place: Place,
    created_generation: i64,
) -> StoreResult<TapeId> {
    validate_tape_payload(Some(tape))?;
    let canonical_hash_id = intern_hash(transaction, canonical_hash)?;
    let raw_length = i64::try_from(raw_length)
        .map_err(|_| StoreError::Invalid("tape length exceeds i64".to_owned()))?;
    transaction
        .prepare_cached(
            "INSERT INTO tape_objects(source_id,canonical_hash_id,grammar_id,grammar_version,tape_format_version,codec_id,raw_length,frame,created_generation,seg,seg_at,seg_len) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,x'',?8,?9,?10,?11) \
             ON CONFLICT(source_id,grammar_id,grammar_version,tape_format_version) DO UPDATE SET seg=excluded.seg, \
             seg_at=excluded.seg_at, seg_len=excluded.seg_len WHERE tape_objects.seg < 0",
        )?
        .execute(params![
            source_id.0,
            canonical_hash_id,
            tape.grammar_id,
            tape.grammar_version,
            tape.tape_format_version,
            super::object_segments::CODEC_RAW,
            raw_length,
            created_generation,
            place.seg,
            place.at as i64,
            place.len as i64,
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

pub(super) fn source_id_by_hash(
    connection: &Connection,
    key: HashKey,
) -> StoreResult<Option<SourceId>> {
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

pub use rerail_store_base::content_store::TapeFrame;
