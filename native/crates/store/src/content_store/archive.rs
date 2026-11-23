//! The archive (decision 0450 step 4): sources and tapes that only aged owners hold, kept as zstd frames in rows of
//! `object_archive`, their segment's bytes then dropped by compaction; placed back in a segment when read again.
//!
//! - **Archived:** a source or tape in a segment that no session's latest data names (no live workspace's snapshot or
//!   overlay, no live pin, no corpus lane's latest run, no `file_documents` row) and that an aged owner holds (an
//!   ended workspace, a lapsed owner's pin, a corpus run past a lane's latest). Its row keeps its length (`seg_len`)
//!   and takes `seg` -2.
//! - **Read again:** a reader that meets `seg` -2 calls [`ContentStore::place_archived`] for the rows it read and reads
//!   them again: the frames decompressed into one segment, the rows pointed at it and their archive rows removed, in
//!   one transaction, so every read is in place (decision 0353). A put of an archived object places the caller's
//!   bytes, as for one marked absent (-1).
//! - **When:** the store's `Collect` upkeep job, after collection, while `corpus_state.archive_due` is set: ending a
//!   session (collection) and a keep decision set it, and a pass that finds nothing clears it.

use super::Cached as _;
use super::object_segments::{Placing, object_dir, row_frame};
use super::segments::SegmentMaps;
use super::reclamation::{OBJECT_SOURCE, OBJECT_TAPE, SESSION_LANE};
use super::{ContentStore, StoreError, StoreResult};
use rusqlite::{OptionalExtension as _, params};
use std::collections::HashSet;

/// A row's `seg` once its object is archived.
pub const ARCHIVED: i64 = -2;

/// The two archived kinds: their table and key column (`object_archive.kind` is `object_pins`' object kind).
const TABLES: [(i64, &str, &str); 2] = [(OBJECT_SOURCE, "source_objects", "source_id"), (OBJECT_TAPE, "tape_objects", "tape_id")];

/// Rows that hold a source with no live holder and an aged one: `?1` now, `?2` the sources of the corpus lanes'
/// latest runs (JSON), `?3` the batch.
const AGED_SOURCES: &str = "SELECT s.source_id, s.seg, s.seg_at, s.seg_len FROM source_objects s WHERE s.seg > 0 \
     AND NOT EXISTS (SELECT 1 FROM snapshot_files f JOIN workspaces w ON w.snapshot_id=f.snapshot_id WHERE f.source_id=s.source_id AND w.state_id=1) \
     AND NOT EXISTS (SELECT 1 FROM workspace_overlay v JOIN workspaces w ON w.workspace_id=v.workspace_id WHERE v.source_id=s.source_id AND w.state_id=1) \
     AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=2 AND p.object_id=s.source_id AND p.lease_until>=?1) \
     AND s.source_id NOT IN (SELECT value FROM json_each(?2)) \
     AND (EXISTS (SELECT 1 FROM corpus_live l WHERE l.source_id=s.source_id) \
          OR EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=2 AND p.object_id=s.source_id) \
          OR EXISTS (SELECT 1 FROM snapshot_files f JOIN workspaces w ON w.snapshot_id=f.snapshot_id WHERE f.source_id=s.source_id AND w.state_id=2) \
          OR EXISTS (SELECT 1 FROM workspace_overlay v JOIN workspaces w ON w.workspace_id=v.workspace_id WHERE v.source_id=s.source_id AND w.state_id=2)) \
     ORDER BY s.source_id LIMIT ?3";

/// The same for tapes; a tape a document names (`file_documents`) is not archived.
const AGED_TAPES: &str = "SELECT t.tape_id, t.seg, t.seg_at, t.seg_len FROM tape_objects t WHERE t.seg > 0 \
     AND NOT EXISTS (SELECT 1 FROM snapshot_files f JOIN workspaces w ON w.snapshot_id=f.snapshot_id WHERE f.tape_id=t.tape_id AND w.state_id=1) \
     AND NOT EXISTS (SELECT 1 FROM workspace_overlay v JOIN workspaces w ON w.workspace_id=v.workspace_id WHERE v.tape_id=t.tape_id AND w.state_id=1) \
     AND NOT EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=1 AND p.object_id=t.tape_id AND p.lease_until>=?1) \
     AND NOT EXISTS (SELECT 1 FROM file_documents d WHERE d.document_tape_id=t.tape_id) \
     AND t.source_id NOT IN (SELECT value FROM json_each(?2)) \
     AND (EXISTS (SELECT 1 FROM corpus_live l WHERE l.source_id=t.source_id AND l.grammar_version=t.grammar_version \
                  AND l.tape_format=t.tape_format_version) \
          OR EXISTS (SELECT 1 FROM object_pins p WHERE p.object_kind_id=1 AND p.object_id=t.tape_id) \
          OR EXISTS (SELECT 1 FROM snapshot_files f JOIN workspaces w ON w.snapshot_id=f.snapshot_id WHERE f.tape_id=t.tape_id AND w.state_id=2) \
          OR EXISTS (SELECT 1 FROM workspace_overlay v JOIN workspaces w ON w.workspace_id=v.workspace_id WHERE v.tape_id=t.tape_id AND w.state_id=2)) \
     ORDER BY t.tape_id LIMIT ?3";

impl ContentStore {
    /// Whether an archive pass is due (`corpus_state.archive_due`): a point read.
    pub fn archive_due(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(connection.query_row_cached("SELECT archive_due FROM corpus_state WHERE singleton=1", [], |r| r.get::<_, i64>(0)).optional()?.unwrap_or(0) != 0)
    }

    /// Archives up to `batch` aged sources and as many aged tapes (the module's rules); returns how many. A pass that
    /// archives nothing clears `archive_due`.
    pub fn archive_aged(&self, now: i64, batch: usize) -> StoreResult<usize> {
        // Archive rows whose object went or was placed again (collected, or a put of the same bytes).
        self.immediate_transaction_once(|tx| {
            Ok(tx.execute_cached(
                "DELETE FROM object_archive WHERE (kind=?1 AND NOT EXISTS (SELECT 1 FROM source_objects s WHERE s.source_id=id AND s.seg=?3)) \
                 OR (kind=?2 AND NOT EXISTS (SELECT 1 FROM tape_objects t WHERE t.tape_id=id AND t.seg=?3))",
                params![OBJECT_SOURCE, OBJECT_TAPE, ARCHIVED],
            )?)
        })?;
        let latest = self.latest_corpus_sources()?;
        let latest = serde_json::to_string(&latest).expect("integers");
        let mut archived = 0;
        for (kind, table, key) in TABLES {
            let rows: Vec<(i64, [i64; 3])> = {
                let connection = self.connection.lock().expect("content store mutex poisoned");
                let sql = if kind == OBJECT_SOURCE { AGED_SOURCES } else { AGED_TAPES };
                connection
                    .prepare_cached(sql)?
                    .query_map(params![now, latest, batch as i64], |r| Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?])))?
                    .collect::<Result<_, _>>()?
            };
            if rows.is_empty() {
                continue;
            }
            // Compressed outside the write transaction, read where the segment holds them.
            let mut maps = SegmentMaps::new(object_dir(&self.database_path));
            let mut frames = Vec::with_capacity(rows.len());
            for (id, [seg, at, len]) in rows {
                let Some(frame) = row_frame(&mut maps, seg, at, len) else { continue };
                let packed = super::compression::compress(frame.segment().bytes(), self.compression_level).map_err(|e| StoreError::Invalid(format!("an archive frame: {e}")))?;
                frames.push((id, [seg, at], packed));
            }
            let update = format!("UPDATE {table} SET seg={ARCHIVED} WHERE {key}=?1 AND seg=?2 AND seg_at=?3");
            archived += self.immediate_transaction_once(|tx| {
                let mut n = 0;
                for (id, [seg, at], packed) in &frames {
                    // Unchanged since it was read: another write may have placed it again, or collection taken it.
                    if tx.execute_cached(&update, params![id, seg, at])? == 1 {
                        tx.execute_cached("INSERT OR REPLACE INTO object_archive(kind,id,frame) VALUES (?1,?2,?3)", params![kind, id, packed])?;
                        n += 1;
                    }
                }
                Ok(n)
            })?;
        }
        if archived == 0 {
            self.immediate_transaction_once(|tx| {
                tx.execute_cached("UPDATE corpus_state SET archive_due=0 WHERE singleton=1", [])?;
                super::corpus_runs::kept::sync_at(tx, &super::corpus_runs::kept::Touch::default())
            })?;
        }
        Ok(archived)
    }

    /// The sources of each corpus lane's latest run (its live session's `latest` manifest) and of every run recorded
    /// since the last keep decision: the latest data no archive takes.
    fn latest_corpus_sources(&self) -> StoreResult<Vec<i64>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let blobs: Vec<Vec<u8>> = connection
            .prepare_cached(
                "SELECT sources FROM corpus_manifests WHERE manifest_id IN (SELECT latest FROM store_sessions WHERE kind=?1 AND ended IS NULL) \
                 OR last_run > coalesce((SELECT run FROM corpus_decided WHERE singleton=1), 0)",
            )?
            .query_map([SESSION_LANE], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut out = HashSet::new();
        for blob in blobs {
            out.extend(super::corpus_runs::decode_ids(&blob)?);
        }
        Ok(out.into_iter().collect())
    }

    /// Places back the archived among `rows` (each a kind, an id and its row's `seg`); whether any was, for the caller
    /// to read them again.
    pub(super) fn placed_again(&self, rows: impl IntoIterator<Item = (i64, i64, i64)>) -> StoreResult<bool> {
        let (mut sources, mut tapes) = (Vec::new(), Vec::new());
        for (kind, id, seg) in rows {
            if seg == ARCHIVED {
                if kind == OBJECT_SOURCE { sources.push(id) } else { tapes.push(id) }
            }
        }
        if sources.is_empty() && tapes.is_empty() {
            return Ok(false);
        }
        Ok(self.place_archived(OBJECT_SOURCE, &sources)? + self.place_archived(OBJECT_TAPE, &tapes)? > 0)
    }

    /// Places the archived objects of `kind` (`OBJECT_SOURCE` or `OBJECT_TAPE`) among `ids` back in one segment: their
    /// frames decompressed into it, their rows pointed at it and their archive rows removed, in one transaction.
    /// Returns how many; ids not archived are left as they are.
    pub(super) fn place_archived(&self, kind: i64, ids: &[i64]) -> StoreResult<usize> {
        let unpacked = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            Unpacked::read(&connection, &self.database_path, kind, ids)?
        };
        let Some(mut unpacked) = unpacked else { return Ok(0) };
        let (seg, placed) = self.immediate_transaction_once(|tx| unpacked.point(tx))?;
        unpacked.placing.name(seg)?;
        Ok(placed)
    }
}

/// For a reader on a connection of its own that cannot write through it (a SQL module's, inside its statement):
/// places the archived tape `tape_id` of the store at `database` back in a segment, from a connection of its own,
/// and returns its new place (`seg`, `seg_at`, `seg_len`); `None` when it is not archived.
pub fn place_archived_tape_at(database: &std::path::Path, tape_id: i64) -> StoreResult<Option<[i64; 3]>> {
    let mut connection = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(std::time::Duration::from_secs(10))?;
    let Some(mut unpacked) = Unpacked::read(&connection, database, OBJECT_TAPE, &[tape_id])? else { return Ok(None) };
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let (seg, _) = unpacked.point(&tx)?;
    tx.commit()?;
    unpacked.placing.name(seg)?;
    Ok(connection
        .query_row("SELECT seg, seg_at, seg_len FROM tape_objects WHERE tape_id=?1 AND seg > 0", [tape_id], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))
        .optional()?)
}

/// Archived objects decompressed into one segment written before the transaction that points their rows at it.
struct Unpacked {
    kind: i64,
    table: &'static str,
    key: &'static str,
    ids: Vec<i64>,
    placing: Placing,
}

impl Unpacked {
    /// The archived among `ids` of `kind`, read on `connection` and written to a segment; `None` when none is.
    fn read(connection: &rusqlite::Connection, database: &std::path::Path, kind: i64, ids: &[i64]) -> StoreResult<Option<Unpacked>> {
        let Some(&(_, table, key)) = TABLES.iter().find(|t| t.0 == kind) else {
            return Err(StoreError::Invalid(format!("no archived kind {kind}")));
        };
        if ids.is_empty() {
            return Ok(None);
        }
        let list = serde_json::to_string(ids).expect("integers");
        let frames: Vec<(i64, Vec<u8>, i64)> = connection
            .prepare(&format!(
                "SELECT a.id, a.frame, x.seg_len FROM object_archive a JOIN {table} x ON x.{key}=a.id \
                 WHERE a.kind=?1 AND x.seg={ARCHIVED} AND a.id IN (SELECT value FROM json_each(?2))"
            ))?
            .query_map(params![kind, list], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        if frames.is_empty() {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(frames.len());
        for (_, frame, len) in &frames {
            let raw = super::compression::decompress(frame, *len as usize).map_err(|e| StoreError::Corrupt(format!("an archive frame: {e}")))?;
            if raw.len() as i64 != *len {
                return Err(StoreError::Corrupt("an archive frame's length does not match its row".to_owned()));
            }
            bytes.push(raw);
        }
        let placing = Placing::write(database, &bytes.iter().map(Vec::as_slice).collect::<Vec<_>>(), super::connection::flush_of(connection))?;
        Ok(Some(Unpacked { kind, table, key, ids: frames.iter().map(|f| f.0).collect(), placing }))
    }

    /// In the write transaction `tx`: numbers the segment, points each still-archived row at its entry and removes
    /// its archive row. Returns the segment's number and how many rows were placed.
    fn point(&self, tx: &rusqlite::Connection) -> StoreResult<(Option<i64>, usize)> {
        let seg = self.placing.number(tx)?;
        let update = format!("UPDATE {} SET seg=?2, seg_at=?3, seg_len=?4 WHERE {}=?1 AND seg={ARCHIVED}", self.table, self.key);
        let mut placed = 0;
        for (i, id) in self.ids.iter().enumerate() {
            let place = self.placing.place(seg.expect("a segment for the frames"), i);
            if tx.execute_cached(&update, params![id, place.seg, place.at as i64, place.len as i64])? == 1 {
                placed += 1;
            }
            tx.execute_cached("DELETE FROM object_archive WHERE kind=?1 AND id=?2", params![self.kind, id])?;
        }
        Ok((seg, placed))
    }
}
