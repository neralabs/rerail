//! The per-file memo's entries as stored (decision 0240 §4, 0295 §4): each write's entries in a segment file beside
//! the store, mapped, and one small row an entry in `memo_entries` saying where it is.
//!
//! An entry was a frame (`frames`, relation `local`, and its outputs again as `local_out`): a record of about 5 KB
//! in a `WITHOUT ROWID` table of 4 KB pages, written twice. Then its bytes were compressed and appended to a pack
//! file. Now each batch of entries is one segment (decision 0295): a journal block (`rerail_blocks::journal`) of the
//! entries' 0239 images, uncompressed, and an entry table of its own, written through a mapped deferrable lease of
//! the pool, so the OS writes the pages back. SQLite holds only what finds an entry (its space, part, path, content
//! and document key) and where it is (segment, offset, length): reading it back maps the segment and reads the image
//! in place.
//!
//! The segment is written, written back and frozen (`Sealed::write_back`) before the store's write transaction; the
//! transaction numbers it, renames it into place and inserts its rows, so writers of other processes take turns as
//! they do at the store. A batch whose transaction fails leaves its file under its writer's name, removed. A segment
//! whose header does not check (not frozen, cut short) is a miss for every row naming it. Collection forgets rows
//! (`collect_unreachable_memo`, a dead plan's space); compaction (`compact_memo_pack`) copies the rows' entries to
//! a new segment and removes the old, once the bytes no row names are most of them. A segment is never given back to
//! the pool for reuse: another process may hold it mapped.
//!
//! An entry a later run found as it was, under more identities (decision 0225 §2), keeps its place: only those
//! identities are written, as a small image of a later segment its row names beside the entry's (`over_*`, decision
//! 0280). A reader takes both; an entry written again whole drops it.
//!
//! Rows of the packs before (`framed` 0: compressed bytes in `<pack>.pack`) are not read; compaction removes them.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::super::{ContentStore, StoreError, StoreResult};
use super::super::segments::{ALIGN, Place, SealedSegments, SegmentBytes, SegmentMaps, SegmentWriter, segment_path, write_segments};
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An entry a run writes: its part (its content's and path's, `memo::part_of`), its path, its content's SHA-256, its
/// document key, and its image (`memo::image`). `over`: the image is not the entry but what is kept over the entry
/// stored at its part (decision 0280): written only where that entry is the same content and document, else dropped
/// (the entry was collected: the next run misses it).
pub struct MemoPut {
    pub part: i64,
    pub path: String,
    pub digest: [u8; 32],
    pub key: i64,
    pub bytes: Vec<u8>,
    pub over: bool,
}

/// An entry as found: what finds it, its image where it is, and what a later run kept over it (`MemoPut::over`).
#[derive(Debug)]
pub struct MemoBlob {
    pub path: String,
    pub digest: [u8; 32],
    pub key: i64,
    pub bytes: SegmentBytes,
    pub over: Option<SegmentBytes>,
}

/// The directory of the store's segments: beside its file.
fn pack_dir(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".memo");
    PathBuf::from(name)
}

fn seg_path(database: &Path, seg: i64) -> PathBuf {
    segment_path(&pack_dir(database), seg)
}

/// A pack of before (`framed` 0), removed by compaction.
fn old_pack_path(database: &Path, pack: i64) -> PathBuf {
    pack_dir(database).join(format!("{pack}.pack"))
}

/// The next segment's number.
fn next_segment(t: &rusqlite::Connection) -> rusqlite::Result<i64> {
    t.query_row_cached("SELECT coalesce(max(pack), 0) + 1 FROM memo_packs", [], |r| r.get(0))
}

/// An entry of a batch, as `put_memo_segments` records it: what finds it (its part, path, content and document
/// key), and whether it is kept over the entry stored at its part (`MemoPut::over`).
pub struct MemoRow {
    pub part: i64,
    pub path: String,
    pub digest: [u8; 32],
    pub key: i64,
    pub over: bool,
}

/// An entry's row (`COLUMNS`): its part, path, content and document key, where it is, and where what is kept over
/// it is (none: `over_len` 0).
struct Row {
    part: i64,
    path: String,
    digest: Vec<u8>,
    key: i64,
    at: Place,
    over: Option<Place>,
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let over_len: i64 = r.get(9)?;
    Ok(Row {
        part: r.get(0)?,
        path: r.get(1)?,
        digest: r.get(2)?,
        key: r.get(3)?,
        at: Place { seg: r.get(4)?, at: r.get::<_, i64>(5)? as u64, len: r.get::<_, i64>(6)? as usize },
        over: (over_len > 0).then(|| Ok::<_, rusqlite::Error>(Place { seg: r.get(7)?, at: r.get::<_, i64>(8)? as u64, len: over_len as usize })).transpose()?,
    })
}

/// An entry's row as compaction reads it: its space, part, document key, content digest, place, and the place of
/// what is kept over it.
type Compacted = (Vec<u8>, i64, i64, Vec<u8>, Place, Option<Place>);

/// An entry compaction keeps: its space, part, document key, content digest, bytes and what is kept over it.
type Kept<'a> = (&'a [u8], i64, i64, [u8; 32], SegmentBytes, Option<SegmentBytes>);

const COLUMNS: &str = "part, path, digest, doc, pack, at, len, over_pack, over_at, over_len";

/// A batch of memo entries as the writer writes them (`queue_memo_segments`).
pub(in crate::content_store) struct MemoWrite {
    space: [u8; 32],
    segments: SealedSegments,
    rows: Vec<MemoRow>,
    keep: usize,
}

impl MemoWrite {
    pub(in crate::content_store) fn run(&mut self, store: &ContentStore) -> StoreResult<()> {
        store.put_memo_segments(&self.space, &mut self.segments, &self.rows, self.keep)
    }
}

impl ContentStore {
    /// The entries of `rows` (`COLUMNS`), in their segments, by part. What is kept over an entry that is not there is
    /// left: its identities are read again (decision 0280).
    fn blobs_of(&self, rows: Vec<Row>) -> HashMap<i64, Arc<MemoBlob>> {
        let mut segments = SegmentMaps::new(pack_dir(&self.database_path));
        rows.into_iter()
            .filter_map(|row| {
                let bytes = segments.at(row.at)?;
                let over = row.over.and_then(|o| segments.at(o));
                let digest = <[u8; 32]>::try_from(row.digest.as_slice()).ok()?;
                Some((row.part, Arc::new(MemoBlob { path: row.path, digest, key: row.key, bytes, over })))
            })
            .collect()
    }

    /// The entries of `space` at `parts`: from the run's snapshot (`stored` false, where this process admitted the
    /// space's entries: `admit_memo_entries`), else from the store.
    pub fn memo_entries(&self, space: &[u8; 32], parts: &[i64], stored: bool) -> StoreResult<HashMap<i64, Arc<MemoBlob>>> {
        if !stored && let Some(snapshot) = self.snapshot() {
            let snapshot = snapshot.lock().expect("the frames' snapshot");
            if snapshot.admitted.contains(space) {
                let held = snapshot.memo.get(space);
                return Ok(parts.iter().filter_map(|p| Some((*p, Arc::clone(held?.get(p)?)))).collect());
            }
        }
        let rows: Vec<Row> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1 AND part IN rarray(?2) AND framed = 1"))?;
            let parts: rusqlite::vtab::array::Array = std::rc::Rc::new(parts.iter().map(|&p| rusqlite::types::Value::from(p)).collect());
            st.query_map(params![&space[..], parts], row_of)?.collect::<Result<_, _>>()?
        };
        Ok(self.blobs_of(rows))
    }

    /// Every entry of `space`, by part.
    pub fn all_memo_entries(&self, space: &[u8; 32]) -> StoreResult<HashMap<i64, Arc<MemoBlob>>> {
        let rows: Vec<Row> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1 AND framed = 1"))?;
            st.query_map([&space[..]], row_of)?.collect::<Result<_, _>>()?
        };
        Ok(self.blobs_of(rows))
    }

    /// A run's memo entries its waves may look up (decision 0104: read before its first wave): in `space` (its
    /// plan's, `local_space`), each path's newest but those of `looked_up` (paths looked up before the first wave,
    /// from the store), held with the snapshot; `hold` is given them all at once, to hold what they name before the waves. Once a
    /// process for each space. Returns how many it read, their bytes, and the paths whose entries it admitted:
    /// `None` when this process admitted the space's already (or holds no snapshot).
    pub fn admit_memo_entries(&self, space: &[u8; 32], looked_up: &HashSet<&str>, hold: &dyn Fn(&[&MemoBlob]) -> StoreResult<()>) -> StoreResult<(usize, usize, Option<HashSet<String>>)> {
        let Some(snapshot) = self.snapshot() else { return Ok((0, 0, None)) };
        if !snapshot.lock().expect("the frames' snapshot").admitted.insert(*space) {
            return Ok((0, 0, None));
        }
        let rows: Vec<Row> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1 AND framed = 1 ORDER BY path, pack DESC, at DESC"))?;
            st.query_map([&space[..]], row_of)?.collect::<Result<_, _>>()?
        };
        let mut paths: HashSet<String> = HashSet::new();
        let mut newest: Vec<Row> = Vec::new();
        for row in rows {
            if !paths.contains(&row.path) {
                paths.insert(row.path.clone());
                if !looked_up.contains(row.path.as_str()) {
                    newest.push(row);
                }
            }
        }
        paths.retain(|p| !looked_up.contains(p.as_str()));
        let got = self.blobs_of(newest);
        hold(&got.values().map(|b| &**b).collect::<Vec<&MemoBlob>>())?;
        let bytes = got.values().map(|b| b.bytes.len()).sum::<usize>();
        let n = got.len();
        let mut snapshot = snapshot.lock().expect("the frames' snapshot");
        let held = snapshot.memo.entry(*space).or_default();
        for (part, blob) in got {
            held.entry(part).or_insert(blob);
        }
        Ok((n, bytes, Some(paths)))
    }

    /// Writes `puts` into `space` (each replacing the entry at its part, or kept over it: `MemoPut::over`): their
    /// images in new segments, then `put_memo_segments`.
    pub fn put_memo_entries(&self, space: &[u8; 32], puts: Vec<MemoPut>, keep: usize) -> StoreResult<()> {
        if puts.is_empty() {
            return Ok(());
        }
        let lens = puts.iter().map(|p| p.bytes.len());
        let mut w = SegmentWriter::new(&self.memo_dir(), puts.len(), rerail_blocks::journal::reserve_for(puts.len(), lens, ALIGN));
        let mut rows = Vec::with_capacity(puts.len());
        for p in puts {
            w.put(p.bytes.len(), p.key, &p.digest, |out| out.copy_from_slice(&p.bytes))?;
            rows.push(MemoRow { part: p.part, path: p.path, digest: p.digest, key: p.key, over: p.over });
        }
        self.put_memo_segments(space, &mut w.seal(), &rows, keep)
    }

    /// The directory a run's `SegmentWriter` writes the store's segments in.
    pub fn memo_dir(&self) -> PathBuf {
        pack_dir(&self.database_path)
    }

    /// Queues a batch's entries (`put_memo_segments`) for the store's writer, behind the run (decision 0306 §8), as
    /// `what`: `bytes` held until written, `keys` each with the entry a read gets meanwhile.
    pub fn queue_memo_segments(&self, what: &'static str, bytes: usize, keys: Vec<(super::super::write_back::Key, Option<super::super::write_back::Held>)>, space: [u8; 32], segments: SealedSegments, rows: Vec<MemoRow>, keep: usize) {
        let write = MemoWrite { space, segments, rows, keep };
        self.write_back_syms(super::super::write_back::Syms::None, what, bytes, keys, super::super::write_back::Work(super::super::write_back::Kind::Memo(Box::new(write))));
    }

    /// The writer's half of a batch (`SegmentWriter`): its segments written back (`Sealed::write_back`), then, in
    /// one transaction, numbered, renamed into place and their entries' rows inserted, `rows[i]` entry `i`. Of each
    /// path written whole, the newest `keep` contents stay (decision 0117: an edited file's earlier entries go).
    pub fn put_memo_segments(&self, space: &[u8; 32], segments: &mut SealedSegments, rows: &[MemoRow], keep: usize) -> StoreResult<()> {
        if rows.is_empty() {
            return Ok(());
        }
        assert_eq!(rows.len(), segments.len(), "a row an entry");
        let database = self.database_path.clone();
        segments.write_back()?;
        let files = segments.files();
        let done = self.immediate_transaction_once(|t| {
            let mut numbers = Vec::with_capacity(files.len());
            for _ in &files {
                let seg = next_segment(t)?;
                t.execute_cached("INSERT INTO memo_packs(pack) VALUES (?1)", [seg])?;
                numbers.push(seg);
            }
            // The parts whose entry took what is kept over it.
            let mut kept_over = HashSet::new();
            for (i, p) in rows.iter().enumerate() {
                let (s, at, len) = segments.placed(i);
                let seg = numbers[s];
                if p.over {
                    let n = t.execute_cached(
                        "UPDATE memo_entries SET over_pack = ?1, over_at = ?2, over_len = ?3, over_raw = ?3 WHERE space = ?4 AND part = ?5 AND digest = ?6 AND doc = ?7 AND framed = 1",
                        params![seg, at as i64, len as i64, &space[..], p.part, &p.digest[..], p.key],
                    )?;
                    if n > 0 {
                        kept_over.insert(p.part);
                    }
                } else {
                    // Written whole: what was kept over the entry it replaces goes with it (the columns' defaults).
                    t.execute_cached(
                        "INSERT OR REPLACE INTO memo_entries(space, part, path, digest, doc, pack, at, len, raw, framed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 1)",
                        params![&space[..], p.part, p.path, &p.digest[..], p.key, seg, at as i64, len as i64],
                    )?;
                }
            }
            let mut gone = Vec::new();
            let paths: HashSet<&str> = rows.iter().filter(|p| !p.over).map(|p| p.path.as_str()).collect();
            for path in paths {
                let mut st = t.prepare_cached("SELECT part FROM memo_entries WHERE space = ?1 AND path = ?2 ORDER BY pack DESC, at DESC LIMIT -1 OFFSET ?3")?;
                let old: Vec<i64> = st.query_map(params![&space[..], path, keep as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
                for part in old {
                    t.execute_cached("DELETE FROM memo_entries WHERE space = ?1 AND part = ?2", params![&space[..], part])?;
                    gone.push(part);
                }
            }
            Ok((gone, kept_over, numbers))
        });
        let (gone, kept_over, numbers) = done?;
        // Each segment under its number, once its rows are committed: a reader meanwhile misses.
        segments.name(&pack_dir(&database), &numbers)?;
        // The snapshot holds what is written, where its space's entries were admitted: in its segment, mapped.
        if let Some(snapshot) = self.snapshot() {
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            if snapshot.admitted.contains(space) {
                let held = snapshot.memo.entry(*space).or_default();
                for (i, p) in rows.iter().enumerate() {
                    let bytes = segments.entry(i);
                    if !p.over {
                        held.insert(p.part, Arc::new(MemoBlob { path: p.path.clone(), digest: p.digest, key: p.key, bytes, over: None }));
                    } else if kept_over.contains(&p.part)
                        && let Some(entry) = held.get_mut(&p.part)
                    {
                        *entry = Arc::new(MemoBlob { path: entry.path.clone(), digest: entry.digest, key: entry.key, bytes: entry.bytes.clone(), over: Some(bytes) });
                    }
                }
                for part in gone {
                    held.remove(&part);
                }
            }
        }
        Ok(())
    }

    /// The segments' bytes (and the packs' of before), and the bytes their rows name: what compaction would leave.
    pub fn memo_pack_bytes(&self) -> StoreResult<(u64, u64)> {
        let (packs, named): (Vec<i64>, i64) = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached("SELECT pack FROM memo_packs")?;
            let packs = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            (packs, connection.query_row_cached("SELECT coalesce(sum(len + over_len), 0) FROM memo_entries WHERE framed = 1", [], |r| r.get(0))?)
        };
        let size = |p: PathBuf| std::fs::metadata(p).map_or(0, |m| m.len());
        let held = packs.iter().map(|&p| size(seg_path(&self.database_path, p)) + size(old_pack_path(&self.database_path, p))).sum();
        Ok((held, named as u64))
    }

    /// Copies every entry (and what is kept over it) to a new segment and removes the others, where the bytes no row
    /// names are more than half the segments and at least `floor`: one write transaction, so no writer adds one
    /// meanwhile. Rows of the packs before are forgotten. Returns the bytes freed. A reader of another process that
    /// found a row before is told of an old segment: it misses (one it mapped stays as it was).
    pub fn compact_memo_pack(&self, floor: u64) -> StoreResult<u64> {
        let (held, named) = self.memo_pack_bytes()?;
        if held < floor || held.saturating_sub(named) < held / 2 {
            return Ok(0);
        }
        let database = self.database_path.clone();
        let (old, file) = self.immediate_transaction_once(|t| {
            let packs: Vec<i64> = t.prepare_cached("SELECT pack FROM memo_packs")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            t.execute_cached("DELETE FROM memo_entries WHERE framed = 0", [])?;
            // Each entry and what is kept over it (`over_len` 0: nothing).
            let rows: Vec<Compacted> = t
                .prepare_cached("SELECT space, part, doc, digest, pack, at, len, over_pack, over_at, over_len FROM memo_entries ORDER BY pack, at")?
                .query_map([], |r| {
                    let at = |p: usize| -> rusqlite::Result<Place> { Ok(Place { seg: r.get(p)?, at: r.get::<_, i64>(p + 1)? as u64, len: r.get::<_, i64>(p + 2)? as usize }) };
                    let over = at(7)?;
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, at(4)?, (over.len > 0).then_some(over)))
                })?
                .collect::<Result<_, _>>()?;
            let mut segments = SegmentMaps::new(pack_dir(&database));
            // An entry not there is forgotten; what is kept over it, where not there, too.
            let mut kept: Vec<Kept<'_>> = Vec::new();
            for (space, part, doc, digest, at, over) in &rows {
                match (segments.at(*at), <[u8; 32]>::try_from(digest.as_slice())) {
                    (Some(bytes), Ok(digest)) => kept.push((space, *part, *doc, digest, bytes, over.and_then(|o| segments.at(o)))),
                    _ => {
                        t.execute_cached("DELETE FROM memo_entries WHERE space = ?1 AND part = ?2", params![space, part])?;
                    }
                }
            }
            let mut entries: Vec<(&[u8], i64, &[u8; 32])> = Vec::new();
            for (_, _, doc, digest, bytes, over) in &kept {
                entries.push((bytes.bytes(), *doc, digest));
                if let Some(o) = over {
                    entries.push((o.bytes(), *doc, digest));
                }
            }
            t.execute_cached("DELETE FROM memo_packs", [])?;
            if kept.is_empty() {
                return Ok((packs, None));
            }
            let mut placed = write_segments(&pack_dir(&database), &entries)?;
            if placed.files().len() != 1 {
                return Err(StoreError::Invalid("the memo's compaction made more than one segment".into()));
            }
            let seg = packs.iter().copied().max().unwrap_or(0) + 1;
            t.execute_cached("INSERT INTO memo_packs(pack) VALUES (?1)", [seg])?;
            let mut offsets = (0..placed.len()).map(|i| placed.placed(i).1);
            for (space, part, _, _, _, over) in &kept {
                let at = offsets.next().expect("an offset an entry");
                let (over_at, over_len) = match over {
                    Some(o) => (offsets.next().expect("an offset an entry") as i64, o.len() as i64),
                    None => (0, 0),
                };
                t.execute_cached(
                    "UPDATE memo_entries SET pack = ?1, at = ?2, over_pack = ?1, over_at = ?3, over_len = ?4, over_raw = ?4 WHERE space = ?5 AND part = ?6",
                    params![seg, at as i64, over_at, over_len, space, part],
                )?;
            }
            placed.name(&pack_dir(&database), &[seg])?;
            Ok((packs, Some(seg)))
        })?;
        for pack in old.into_iter().filter(|&p| Some(p) != file) {
            let _ = std::fs::remove_file(seg_path(&database, pack));
            let _ = std::fs::remove_file(old_pack_path(&database, pack));
        }
        let (now, _) = self.memo_pack_bytes()?;
        Ok(held.saturating_sub(now))
    }
}

/// Forgets every memo entry of `space` (a dead plan's), in the caller's transaction: how many.
pub(in crate::content_store) fn drop_memo_space(t: &rusqlite::Connection, space: &[u8; 32]) -> rusqlite::Result<usize> {
    t.execute_cached("DELETE FROM memo_entries WHERE space = ?1", [&space[..]])
}

/// The memo entries no kept manifest reaches (decision 0227), forgotten in the caller's transaction: of the live
/// plans' spaces (`live`), those whose content no kept manifest names (`contents`). Another space's entries are a
/// dead plan's, which go with it (`drop_memo_space`), or a run's whose plan is not recorded yet: left.
pub(in crate::content_store) fn collect_memo_entries(t: &rusqlite::Connection, live: &HashSet<[u8; 32]>, contents: &HashSet<[u8; 32]>) -> rusqlite::Result<usize> {
    let mut n = 0;
    for plan in live {
        let space = super::local_space(plan);
        let rows: Vec<(i64, Vec<u8>)> = t
            .prepare_cached("SELECT part, digest FROM memo_entries WHERE space = ?1")?
            .query_map([&space[..]], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (part, digest) in rows {
            if !<[u8; 32]>::try_from(digest.as_slice()).is_ok_and(|d| contents.contains(&d)) {
                n += t.execute_cached("DELETE FROM memo_entries WHERE space = ?1 AND part = ?2", params![&space[..], part])?;
            }
        }
    }
    Ok(n)
}
