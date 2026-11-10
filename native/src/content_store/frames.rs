//! Facts as stored: every relation of every fact space, as frames (decision 0104; docs/final-shape/notes/t33 K1).
//!
//! A frame is the rows of one relation of one fact space, or of one partition of it (a document, a granule; `0` is
//! the whole relation), each column as zigzag varints of its differences (`cross_rows::encode`). It is keyed by:
//!
//! - **space**: the fact space, a digest of the corpus root and the space's name (a phase, a program's cross part);
//! - **rel** and **ident**: the relation's name, and its identity. The identity is the digest of the rules it is
//!   derived from, their reads' identities, and the engine (`plan::identities`), so a rule edit makes another
//!   relation, which is derived again. A relation that depends on no rule (a native answer's) has its own;
//! - **part**: the partition.
//!
//! A derived frame records what it was derived from: each relation its stratum read, with the digest that read had
//! (03 §1, "the versions of the facts it was derived from", at the grain of a relation). It is current while every
//! read has that digest still. That is what lets relations of the same identity be one copy for every program that
//! has them, in any order of runs.
//!
//! Frames are written only by the store's writer (decision 0102), behind the run. What a space keeps is the
//! relations its runs use: a relation is collected by the [`RUNS`]th run of its space in a row that does not use it.

use super::{ContentStore, StoreResult};
use rusqlite::{OptionalExtension as _, params};
use std::collections::HashMap;

/// How many runs of a space in a row may leave a relation unused before it is collected.
pub const RUNS: i64 = 4;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS frames(space BLOB NOT NULL, rel TEXT NOT NULL, ident BLOB NOT NULL, \
     part INTEGER NOT NULL, digest INTEGER NOT NULL, reads BLOB NOT NULL, width INTEGER NOT NULL, rows INTEGER NOT NULL, \
     types BLOB NOT NULL, data BLOB NOT NULL, used INTEGER NOT NULL, PRIMARY KEY (space, rel, ident, part)) WITHOUT ROWID; \
     CREATE TABLE IF NOT EXISTS frame_spaces(space BLOB PRIMARY KEY, runs INTEGER NOT NULL) WITHOUT ROWID;";

/// A relation as its frame names it: its name and its identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rel {
    pub name: String,
    pub ident: [u8; 32],
}

/// A frame's rows: each column's value type (the engine's codes), and the rows, flat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rows {
    pub types: Vec<u8>,
    pub rows: Vec<i64>,
    /// The rows' count (a relation of no columns has rows but no values).
    pub n: usize,
}

/// A stored frame, without its rows: its digest and what it was derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub digest: u64,
    /// Each relation it was derived from, by name, with that relation's digest then.
    pub reads: Vec<(String, u64)>,
}

/// A frame a run writes.
pub struct Put {
    pub rel: Rel,
    pub part: i64,
    pub digest: u64,
    pub reads: Vec<(String, u64)>,
    pub rows: Rows,
}

impl ContentStore {
    fn frames_table(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let n: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='frames'", [], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The stored frames of `rels` in `space`, partition `part`, without their rows.
    pub fn frames(&self, space: &[u8; 32], rels: &[Rel], part: i64) -> StoreResult<HashMap<Rel, Stored>> {
        self.settle(&space_key(space));
        if !self.frames_table()? {
            return Ok(HashMap::new());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT digest, reads FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?;
        let mut out = HashMap::new();
        for rel in rels {
            let found = st.query_row(params![&space[..], rel.name, &rel.ident[..], part], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))).optional()?;
            if let Some((digest, reads)) = found {
                out.insert(rel.clone(), Stored { digest: digest as u64, reads: decode_reads(&reads) });
            }
        }
        Ok(out)
    }

    /// The rows of `rel`'s frame `part` in `space`.
    pub fn frame_rows(&self, space: &[u8; 32], rel: &Rel, part: i64) -> StoreResult<Option<Rows>> {
        self.settle(&space_key(space));
        if !self.frames_table()? {
            return Ok(None);
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT width, rows, types, data FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?;
        let found = st
            .query_row(params![&space[..], rel.name, &rel.ident[..], part], |r| {
                Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?))
            })
            .optional()?;
        let Some((width, rows, types, data)) = found else { return Ok(None) };
        let mut cols = vec![Vec::new(); width];
        super::cross_rows::decode(width, rows, &data, &mut cols)?;
        let mut flat = Vec::with_capacity(width * rows);
        for r in 0..if width == 0 { 0 } else { rows } {
            flat.extend(cols.iter().map(|c| c[r]));
        }
        Ok(Some(Rows { types, rows: flat, n: rows }))
    }

    /// `keep_frames`, queued behind the run (decision 0102): encoded and written by the store's writer.
    pub fn queue_frames(&self, space: [u8; 32], puts: Vec<Put>, gone: Vec<(Rel, i64)>, used: Option<Vec<Rel>>) {
        let bytes = puts.iter().map(|p| p.rows.rows.len() * 8).sum();
        self.write_back("frames", bytes, vec![(space_key(&space), None)], Box::new(move |store| {
            // Attempted once: a busy store leaves these frames unwritten, and the next run derives them again.
            let _ = store.keep_frames(&space, &puts, &gone, used.as_deref());
            Ok(())
        }));
    }

    /// Writes `puts` into `space` (each replacing its frame) and forgets `gone`. With `used`, the run is over: the
    /// space counts it, marks the relations it used, and forgets those the last [`RUNS`] runs did not use.
    pub fn keep_frames(&self, space: &[u8; 32], puts: &[Put], gone: &[(Rel, i64)], used: Option<&[Rel]>) -> StoreResult<()> {
        let blocks: Vec<Vec<u8>> = puts.iter().map(|p| super::cross_rows::encode(p.rows.types.len(), &p.rows.rows)).collect();
        self.immediate_transaction_once(|t| {
            t.execute_batch(SCHEMA)?;
            t.execute("INSERT INTO frame_spaces(space, runs) VALUES (?1, 0) ON CONFLICT(space) DO NOTHING", [&space[..]])?;
            let run: i64 = t.query_row("SELECT runs FROM frame_spaces WHERE space = ?1", [&space[..]], |r| r.get(0))?;
            let mut ins = t.prepare_cached(
                "INSERT INTO frames(space, rel, ident, part, digest, reads, width, rows, types, data, used) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT(space, rel, ident, part) DO UPDATE SET \
                 digest = excluded.digest, reads = excluded.reads, width = excluded.width, rows = excluded.rows, \
                 types = excluded.types, data = excluded.data, used = excluded.used",
            )?;
            for (p, data) in puts.iter().zip(&blocks) {
                ins.execute(params![
                    &space[..],
                    p.rel.name,
                    &p.rel.ident[..],
                    p.part,
                    p.digest as i64,
                    encode_reads(&p.reads),
                    p.rows.types.len() as i64,
                    p.rows.n as i64,
                    p.rows.types,
                    data,
                    run
                ])?;
            }
            let mut del = t.prepare_cached("DELETE FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?;
            for (rel, part) in gone {
                del.execute(params![&space[..], rel.name, &rel.ident[..], part])?;
            }
            if let Some(used) = used {
                let mut mark = t.prepare_cached("UPDATE frames SET used = ?4 WHERE space = ?1 AND rel = ?2 AND ident = ?3")?;
                for rel in used {
                    mark.execute(params![&space[..], rel.name, &rel.ident[..], run])?;
                }
                t.execute("DELETE FROM frames WHERE space = ?1 AND used <= ?2", params![&space[..], run - RUNS])?;
                t.execute("UPDATE frame_spaces SET runs = runs + 1 WHERE space = ?1", [&space[..]])?;
            }
            Ok(())
        })
    }
}

/// What a write of space `space`'s frames is queued under (`write_back`).
fn space_key(space: &[u8; 32]) -> super::write_back::Key {
    super::write_back::Key::of(&[b"frames", space])
}

fn encode_reads(reads: &[(String, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, digest) in reads {
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(digest.to_le_bytes());
    }
    out
}

fn decode_reads(mut bytes: &[u8]) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    while bytes.len() >= 4 {
        let len = u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes")) as usize;
        if bytes.len() < 4 + len + 8 {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[4..4 + len]).into_owned();
        let digest = u64::from_le_bytes(bytes[4 + len..12 + len].try_into().expect("8 bytes"));
        out.push((name, digest));
        bytes = &bytes[12 + len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(name: &str, i: u8) -> Rel {
        Rel { name: name.into(), ident: [i; 32] }
    }

    #[test]
    fn a_frame_is_read_back_with_what_it_was_derived_from() {
        let dir = std::env::temp_dir().join(format!("rerail-frames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let space = [1u8; 32];
        let (r, s) = (rel("r", 1), rel("r", 2));
        assert!(store.frames(&space, &[r.clone()], 0).unwrap().is_empty());
        let rows = Rows { types: vec![0, 1], rows: vec![5, -3, 1 << 40, 7], n: 2 };
        let put = Put { rel: r.clone(), part: 0, digest: 9, reads: vec![("input".into(), 4), ("q".into(), 1 << 63)], rows };
        store.keep_frames(&space, &[put], &[], None).unwrap();
        let got = store.frames(&space, &[r.clone(), s.clone()], 0).unwrap();
        assert_eq!(got.get(&r), Some(&Stored { digest: 9, reads: vec![("input".into(), 4), ("q".into(), 1 << 63)] }));
        // Another identity of the same name is another relation.
        assert_eq!(got.get(&s), None);
        let back = store.frame_rows(&space, &r, 0).unwrap().unwrap();
        assert_eq!((back.types, back.rows, back.n), (vec![0, 1], vec![5, -3, 1 << 40, 7], 2));
        // Replaced, then forgotten.
        let put = Put { rel: r.clone(), part: 0, digest: 10, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        store.keep_frames(&space, &[put], &[], None).unwrap();
        assert_eq!(store.frames(&space, &[r.clone()], 0).unwrap()[&r].digest, 10);
        store.keep_frames(&space, &[], &[(r.clone(), 0)], None).unwrap();
        assert!(store.frames(&space, &[r.clone()], 0).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relation_no_run_used_for_four_runs_is_collected() {
        let dir = std::env::temp_dir().join(format!("rerail-frames-gc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let space = [2u8; 32];
        let (kept, old) = (rel("kept", 1), rel("old", 1));
        let put = |r: &Rel| Put { rel: r.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        store.keep_frames(&space, &[put(&kept), put(&old)], &[], Some(&[kept.clone(), old.clone()])).unwrap();
        for _ in 0..RUNS {
            assert!(store.frames(&space, &[old.clone()], 0).unwrap().contains_key(&old));
            store.keep_frames(&space, &[], &[], Some(&[kept.clone()])).unwrap();
        }
        let got = store.frames(&space, &[kept.clone(), old.clone()], 0).unwrap();
        assert!(got.contains_key(&kept) && !got.contains_key(&old));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
