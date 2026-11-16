//! The per-file memo's entries as stored (decision 0240 §4): each entry's bytes appended to a pack file beside the
//! store, and one small row an entry in `memo_entries` saying where they are.
//!
//! An entry was a frame (`frames`, relation `local`, and its outputs again as `local_out`): a record of about 5 KB
//! in a `WITHOUT ROWID` table of 4 KB pages, written twice. SQLite now holds only what finds an entry (its space,
//! part, path, content and document key) and where its bytes are (pack, offset, length); the bytes are written once,
//! in one `write` a batch, at the pack's end.
//!
//! The pack is appended inside the store's write transaction, so writers of other processes take turns at it as they
//! do at the store. A batch whose transaction fails leaves bytes no row names; a row whose bytes did not reach the
//! disk (a crash before the file was flushed) names bytes that do not decode, and is a miss. Collection forgets rows
//! (`collect_unreachable_frames`, a dead plan's space); compaction (`compact_memo_pack`) copies the rows' bytes to a
//! new pack and removes the old, once the bytes no row names are most of it.
//!
//! An entry a later run found as it was, under more identities (decision 0225 §2), keeps its bytes: only those
//! identities are appended, as a small image its row names beside the entry's (`over_*`, decision 0280). A reader
//! takes both; an entry written again whole drops it.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::super::{ContentStore, StoreError, StoreResult};
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Seek as _, Write as _};
use std::sync::Arc;

/// An entry a run writes: its part (its content's and path's, `memo::part_of`), its path, its content's SHA-256, its
/// document key, and its bytes (`memo::image`) with their length decoded. `over`: the bytes are not the entry but
/// what is kept over the entry stored at its part (decision 0280): written only where that entry is the same content
/// and document, else dropped (the entry was collected: the next run misses it).
pub struct MemoPut {
    pub part: i64,
    pub path: String,
    pub digest: [u8; 32],
    pub key: i64,
    pub bytes: Vec<u8>,
    pub raw: usize,
    pub over: bool,
}

/// An entry as found: what finds it, its bytes as stored, and what a later run kept over it (`MemoPut::over`), each
/// with its length decoded.
#[derive(Debug)]
pub struct MemoBlob {
    pub path: String,
    pub digest: [u8; 32],
    pub key: i64,
    pub bytes: Arc<[u8]>,
    pub raw: usize,
    pub over: Option<(Arc<[u8]>, usize)>,
}

/// Where an entry's bytes are: its pack, offset and length.
#[derive(Clone, Copy)]
struct At {
    pack: i64,
    at: u64,
    len: usize,
}

/// The directory of the store's packs: beside its file.
fn pack_dir(database: &std::path::Path) -> std::path::PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".memo");
    std::path::PathBuf::from(name)
}

fn pack_path(database: &std::path::Path, pack: i64) -> std::path::PathBuf {
    pack_dir(database).join(format!("{pack}.pack"))
}

/// The pack being appended: the newest.
fn current_pack(t: &rusqlite::Connection) -> rusqlite::Result<i64> {
    t.query_row_cached("SELECT coalesce(max(pack), 1) FROM memo_packs", [], |r| r.get(0))
}

/// The bytes at each of `wanted` (in order), read from the packs: a pack or range that is not there is `None`.
fn read_at(database: &std::path::Path, wanted: &[At]) -> Vec<Option<Vec<u8>>> {
    let mut files: HashMap<i64, Option<std::fs::File>> = HashMap::new();
    wanted
        .iter()
        .map(|w| {
            let file = files.entry(w.pack).or_insert_with(|| std::fs::File::open(pack_path(database, w.pack)).ok()).as_mut()?;
            file.seek(std::io::SeekFrom::Start(w.at)).ok()?;
            let mut bytes = vec![0u8; w.len];
            file.read_exact(&mut bytes).ok()?;
            Some(bytes)
        })
        .collect()
}

/// An entry's row (`COLUMNS`): its part, path, content and document key, where its bytes are and their length
/// decoded, and where what is kept over it is (none: `over_len` 0).
struct Row {
    part: i64,
    path: String,
    digest: Vec<u8>,
    key: i64,
    at: At,
    raw: i64,
    over: Option<(At, i64)>,
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let over_len: i64 = r.get(10)?;
    Ok(Row {
        part: r.get(0)?,
        path: r.get(1)?,
        digest: r.get(2)?,
        key: r.get(3)?,
        at: At { pack: r.get(4)?, at: r.get::<_, i64>(5)? as u64, len: r.get::<_, i64>(6)? as usize },
        raw: r.get(7)?,
        over: (over_len > 0).then(|| Ok::<_, rusqlite::Error>((At { pack: r.get(8)?, at: r.get::<_, i64>(9)? as u64, len: over_len as usize }, r.get(11)?))).transpose()?,
    })
}

const COLUMNS: &str = "part, path, digest, doc, pack, at, len, raw, over_pack, over_at, over_len, over_raw";

impl ContentStore {
    /// The entries of `rows` (`COLUMNS`), their bytes read from the packs, by part.
    /// What is kept over an entry whose bytes are not there is left: its identities are read again (decision 0280).
    fn blobs_of(&self, rows: Vec<Row>) -> HashMap<i64, Arc<MemoBlob>> {
        // One read of the packs for the entries and what is kept over them: the entries' first.
        let mut at: Vec<At> = rows.iter().map(|r| r.at).collect();
        at.extend(rows.iter().filter_map(|r| r.over.map(|(o, _)| o)));
        let mut bytes = read_at(&self.database_path, &at).into_iter();
        let entries: Vec<Option<Vec<u8>>> = bytes.by_ref().take(rows.len()).collect();
        rows.into_iter()
            .zip(entries)
            .filter_map(|(row, entry)| {
                let over = row.over.and_then(|(_, raw)| Some((Arc::from(bytes.next().flatten()?), raw as usize)));
                let digest = <[u8; 32]>::try_from(row.digest.as_slice()).ok()?;
                Some((row.part, Arc::new(MemoBlob { path: row.path, digest, key: row.key, bytes: Arc::from(entry?), raw: row.raw as usize, over })))
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
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1 AND part IN rarray(?2)"))?;
            let parts: rusqlite::vtab::array::Array = std::rc::Rc::new(parts.iter().map(|&p| rusqlite::types::Value::from(p)).collect());
            st.query_map(params![&space[..], parts], row_of)?.collect::<Result<_, _>>()?
        };
        Ok(self.blobs_of(rows))
    }

    /// Every entry of `space`, by part.
    pub fn all_memo_entries(&self, space: &[u8; 32]) -> StoreResult<HashMap<i64, Arc<MemoBlob>>> {
        let rows: Vec<Row> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1"))?;
            st.query_map([&space[..]], row_of)?.collect::<Result<_, _>>()?
        };
        Ok(self.blobs_of(rows))
    }

    /// A run's memo entries its waves may look up (decision 0104: read before its first wave): in `space` (its
    /// plan's, `local_space`), each path's newest but those of `looked_up` (paths looked up before the first wave,
    /// from the store), held with the snapshot; `hold` is given each, to hold what it names before the waves. Once a
    /// process for each space. Returns how many it read, their bytes, and the paths whose entries it admitted:
    /// `None` when this process admitted the space's already (or holds no snapshot).
    pub fn admit_memo_entries(&self, space: &[u8; 32], looked_up: &HashSet<&str>, hold: &dyn Fn(&MemoBlob) -> StoreResult<()>) -> StoreResult<(usize, usize, Option<HashSet<String>>)> {
        let Some(snapshot) = self.snapshot() else { return Ok((0, 0, None)) };
        if !snapshot.lock().expect("the frames' snapshot").admitted.insert(*space) {
            return Ok((0, 0, None));
        }
        let rows: Vec<Row> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(&format!("SELECT {COLUMNS} FROM memo_entries WHERE space = ?1 ORDER BY path, pack DESC, at DESC"))?;
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
        for blob in got.values() {
            hold(blob)?;
        }
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
    /// bytes appended to the pack in one write, their rows in the same transaction. Of each path written whole, the
    /// newest `keep` contents stay (decision 0117: an edited file's earlier entries go).
    pub fn put_memo_entries(&self, space: &[u8; 32], puts: Vec<MemoPut>, keep: usize) -> StoreResult<()> {
        if puts.is_empty() {
            return Ok(());
        }
        let database = self.database_path.clone();
        let gone = self.immediate_transaction_once(|t| {
            let pack = current_pack(t)?;
            t.execute_cached("INSERT OR IGNORE INTO memo_packs(pack) VALUES (?1)", [pack])?;
            std::fs::create_dir_all(pack_dir(&database))?;
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(pack_path(&database, pack))?;
            let start = file.seek(std::io::SeekFrom::End(0))?;
            let mut all = Vec::with_capacity(puts.iter().map(|p| p.bytes.len()).sum());
            for p in &puts {
                all.extend_from_slice(&p.bytes);
            }
            file.write_all(&all)?;
            drop(all);
            let mut at = start;
            // The parts whose entry took what is kept over it.
            let mut kept_over = HashSet::new();
            for p in &puts {
                if p.over {
                    let n = t.execute_cached(
                        "UPDATE memo_entries SET over_pack = ?1, over_at = ?2, over_len = ?3, over_raw = ?4 WHERE space = ?5 AND part = ?6 AND digest = ?7 AND doc = ?8",
                        params![pack, at as i64, p.bytes.len() as i64, p.raw as i64, &space[..], p.part, &p.digest[..], p.key],
                    )?;
                    if n > 0 {
                        kept_over.insert(p.part);
                    }
                } else {
                    // Written whole: what was kept over the entry it replaces goes with it (the columns' defaults).
                    t.execute_cached(
                        "INSERT OR REPLACE INTO memo_entries(space, part, path, digest, doc, pack, at, len, raw) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![&space[..], p.part, p.path, &p.digest[..], p.key, pack, at as i64, p.bytes.len() as i64, p.raw as i64],
                    )?;
                }
                at += p.bytes.len() as u64;
            }
            let mut gone = Vec::new();
            let paths: HashSet<&str> = puts.iter().filter(|p| !p.over).map(|p| p.path.as_str()).collect();
            for path in paths {
                let mut st = t.prepare_cached("SELECT part FROM memo_entries WHERE space = ?1 AND path = ?2 ORDER BY pack DESC, at DESC LIMIT -1 OFFSET ?3")?;
                let old: Vec<i64> = st.query_map(params![&space[..], path, keep as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
                for part in old {
                    t.execute_cached("DELETE FROM memo_entries WHERE space = ?1 AND part = ?2", params![&space[..], part])?;
                    gone.push(part);
                }
            }
            Ok((gone, kept_over))
        })?;
        let (gone, kept_over) = gone;
        // The snapshot holds what is written, where its space's entries were admitted.
        if let Some(snapshot) = self.snapshot() {
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            if snapshot.admitted.contains(space) {
                let held = snapshot.memo.entry(*space).or_default();
                for p in puts {
                    if !p.over {
                        held.insert(p.part, Arc::new(MemoBlob { path: p.path, digest: p.digest, key: p.key, bytes: Arc::from(p.bytes), raw: p.raw, over: None }));
                    } else if kept_over.contains(&p.part)
                        && let Some(entry) = held.get_mut(&p.part)
                    {
                        let over = Some((Arc::from(p.bytes), p.raw));
                        *entry = Arc::new(MemoBlob { path: entry.path.clone(), digest: entry.digest, key: entry.key, bytes: Arc::clone(&entry.bytes), raw: entry.raw, over });
                    }
                }
                for part in gone {
                    held.remove(&part);
                }
            }
        }
        Ok(())
    }

    /// The packs' bytes, and the bytes their rows name: what compaction would leave.
    pub fn memo_pack_bytes(&self) -> StoreResult<(u64, u64)> {
        let (packs, named): (Vec<i64>, i64) = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached("SELECT pack FROM memo_packs")?;
            let packs = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            (packs, connection.query_row_cached("SELECT coalesce(sum(len + over_len), 0) FROM memo_entries", [], |r| r.get(0))?)
        };
        let held = packs.iter().filter_map(|p| std::fs::metadata(pack_path(&self.database_path, *p)).ok()).map(|m| m.len()).sum();
        Ok((held, named as u64))
    }

    /// Copies every entry's bytes to a new pack and removes the others, where the bytes no row names are more than
    /// half the packs and at least `floor`: one write transaction, so no writer appends meanwhile. Returns the bytes
    /// freed. A reader of another process that found a row before is told of an old pack: it misses.
    pub fn compact_memo_pack(&self, floor: u64) -> StoreResult<u64> {
        let (held, named) = self.memo_pack_bytes()?;
        if held < floor || held.saturating_sub(named) < held / 2 {
            return Ok(0);
        }
        let database = self.database_path.clone();
        let old = self.immediate_transaction_once(|t| {
            let packs: Vec<i64> = t.prepare_cached("SELECT pack FROM memo_packs")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            let pack = current_pack(t)? + 1;
            // Each entry's bytes and what is kept over it (`over_len` 0: nothing).
            let rows: Vec<(Vec<u8>, i64, At, At)> = t
                .prepare_cached("SELECT space, part, pack, at, len, over_pack, over_at, over_len FROM memo_entries ORDER BY pack, at")?
                .query_map([], |r| {
                    let at = |p: usize| -> rusqlite::Result<At> { Ok(At { pack: r.get(p)?, at: r.get::<_, i64>(p + 1)? as u64, len: r.get::<_, i64>(p + 2)? as usize }) };
                    Ok((r.get(0)?, r.get(1)?, at(2)?, at(5)?))
                })?
                .collect::<Result<_, _>>()?;
            std::fs::create_dir_all(pack_dir(&database))?;
            let mut out = std::io::BufWriter::new(std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(pack_path(&database, pack))?);
            let bytes = read_at(&database, &rows.iter().map(|r| r.2).collect::<Vec<_>>());
            let over = read_at(&database, &rows.iter().map(|r| r.3).filter(|o| o.len > 0).collect::<Vec<_>>());
            let mut over = over.into_iter();
            let mut at = 0u64;
            for ((space, part, entry, kept), bytes) in rows.iter().zip(bytes) {
                let kept = if kept.len > 0 { over.next().flatten() } else { None };
                match bytes {
                    Some(bytes) => {
                        out.write_all(&bytes)?;
                        let entry_at = at;
                        at += entry.len as u64;
                        // What is kept over it follows it; where its bytes are not there, it is forgotten.
                        let (over_at, over_len) = match &kept {
                            Some(kept) => {
                                out.write_all(kept)?;
                                at += kept.len() as u64;
                                (at - kept.len() as u64, kept.len())
                            }
                            None => (0, 0),
                        };
                        t.execute_cached(
                            "UPDATE memo_entries SET pack = ?1, at = ?2, over_pack = ?1, over_at = ?3, over_len = ?4, over_raw = CASE WHEN ?4 = 0 THEN 0 ELSE over_raw END WHERE space = ?5 AND part = ?6",
                            params![pack, entry_at as i64, over_at as i64, over_len as i64, space, part],
                        )?;
                    }
                    // Bytes that are not there: the row is a miss, forgotten.
                    None => {
                        t.execute_cached("DELETE FROM memo_entries WHERE space = ?1 AND part = ?2", params![space, part])?;
                    }
                }
            }
            out.into_inner().map_err(|e| StoreError::Io(e.into_error()))?.sync_data()?;
            t.execute_cached("DELETE FROM memo_packs", [])?;
            t.execute_cached("INSERT INTO memo_packs(pack) VALUES (?1)", [pack])?;
            Ok(packs)
        })?;
        for pack in old {
            let _ = std::fs::remove_file(pack_path(&database, pack));
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
