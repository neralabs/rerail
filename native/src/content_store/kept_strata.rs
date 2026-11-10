//! A cross-file part's strata as the last run of a program left them, kept across processes (decision 0077 §4): for
//! each input the part read, a digest of its rows; for each derived relation, a digest of its rows and the rows, one
//! block per relation (`cross_rows::encode`: each column as zigzag varints of its differences). Keyed by the part's
//! program (`prog`, a digest the caller makes); one row per relation, replaced when its rows change, so a run after
//! an edit writes only the relations whose rows changed.
//!
//! Only the last `PROGRAMS` programs written are kept: each build, and each pack, is another program.

use super::{ContentStore, StoreResult};
use rusqlite::{OptionalExtension as _, params};
use std::collections::HashMap;

/// How many programs' strata the store keeps.
const PROGRAMS: i64 = 4;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS kept_strata(prog BLOB NOT NULL, rel TEXT NOT NULL, digest INTEGER NOT NULL, \
     width INTEGER, rows INTEGER, types BLOB, data BLOB, PRIMARY KEY (prog, rel)) WITHOUT ROWID; \
     CREATE TABLE IF NOT EXISTS kept_strata_programs(prog BLOB PRIMARY KEY, written INTEGER NOT NULL) WITHOUT ROWID;";

/// A relation's rows as kept: each column's value type (the engine's codes), and the rows, flat.
pub struct KeptRows {
    pub types: Vec<u8>,
    pub rows: Vec<i64>,
    /// The rows' count (a relation of no columns has rows but no values).
    pub n: usize,
}

/// What a run writes of one relation: its digest, and its rows when it is derived (`None`: an input, a digest only).
pub struct KeptPut {
    pub rel: String,
    pub digest: u64,
    pub rows: Option<KeptRows>,
}

impl ContentStore {
    fn kept_strata_table(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let n: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='kept_strata'", [], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// Each relation program `prog`'s last run kept: its digest, and whether its rows are kept.
    pub fn kept_strata(&self, prog: &[u8; 32]) -> StoreResult<HashMap<String, (u64, bool)>> {
        self.settle(&kept_key(prog));
        if !self.kept_strata_table()? {
            return Ok(HashMap::new());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT rel, digest, data IS NOT NULL FROM kept_strata WHERE prog = ?1")?;
        let rows = st.query_map([&prog[..]], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)? as u64, r.get::<_, bool>(2)?))))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Relation `rel`'s rows as program `prog`'s last run kept them.
    pub fn kept_strata_rows(&self, prog: &[u8; 32], rel: &str) -> StoreResult<Option<KeptRows>> {
        self.settle(&kept_key(prog));
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT width, rows, types, data FROM kept_strata WHERE prog = ?1 AND rel = ?2 AND data IS NOT NULL")?;
        let found = st
            .query_row(params![&prog[..], rel], |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?)))
            .optional()?;
        let Some((width, rows, types, data)) = found else { return Ok(None) };
        let mut cols = vec![Vec::new(); width];
        super::cross_rows::decode(width, rows, &data, &mut cols)?;
        let mut flat = Vec::with_capacity(width * rows);
        for r in 0..if width == 0 { 0 } else { rows } {
            flat.extend(cols.iter().map(|c| c[r]));
        }
        Ok(Some(KeptRows { types, rows: flat, n: rows }))
    }

    /// `keep_strata`, queued behind the run (decision 0102): encoded and written by the store's writer.
    pub fn queue_keep_strata(&self, prog: [u8; 32], puts: Vec<KeptPut>, gone: Vec<String>) {
        let bytes = puts.iter().filter_map(|p| p.rows.as_ref()).map(|k| k.rows.len() * 8).sum();
        self.write_back("the strata's rows", bytes, vec![(kept_key(&prog), None)], Box::new(move |store| {
            // Attempted once, as before the queue: a busy store leaves these strata unkept, and the next run evaluates them.
            let _ = store.keep_strata(&prog, &puts, &gone);
            Ok(())
        }));
    }

    /// Keeps `puts` for program `prog` (each replacing the relation's row) and forgets `gone`; the programs past the
    /// last `PROGRAMS` written are forgotten whole.
    pub fn keep_strata(&self, prog: &[u8; 32], puts: &[KeptPut], gone: &[String]) -> StoreResult<()> {
        let blocks: Vec<Option<(usize, usize, Vec<u8>)>> = puts
            .iter()
            .map(|p| {
                p.rows.as_ref().map(|k| {
                    let width = k.types.len();
                    (width, k.n, super::cross_rows::encode(width, &k.rows))
                })
            })
            .collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64);
        self.immediate_transaction_once(|t| {
            t.execute_batch(SCHEMA)?;
            let mut ins = t.prepare_cached(
                "INSERT INTO kept_strata(prog, rel, digest, width, rows, types, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(prog, rel) DO UPDATE SET digest = excluded.digest, width = excluded.width, rows = excluded.rows, \
                 types = excluded.types, data = excluded.data",
            )?;
            for (p, block) in puts.iter().zip(&blocks) {
                match (block, &p.rows) {
                    (Some((width, rows, data)), Some(k)) => ins.execute(params![&prog[..], p.rel, p.digest as i64, *width as i64, *rows as i64, k.types, data])?,
                    _ => ins.execute(params![&prog[..], p.rel, p.digest as i64, None::<i64>, None::<i64>, None::<Vec<u8>>, None::<Vec<u8>>])?,
                };
            }
            let mut del = t.prepare_cached("DELETE FROM kept_strata WHERE prog = ?1 AND rel = ?2")?;
            for rel in gone {
                del.execute(params![&prog[..], rel])?;
            }
            t.execute("INSERT INTO kept_strata_programs(prog, written) VALUES (?1, ?2) ON CONFLICT(prog) DO UPDATE SET written = excluded.written", params![&prog[..], now])?;
            t.execute(
                "DELETE FROM kept_strata WHERE prog NOT IN (SELECT prog FROM kept_strata_programs ORDER BY written DESC LIMIT ?1)",
                [PROGRAMS],
            )?;
            t.execute("DELETE FROM kept_strata_programs WHERE prog NOT IN (SELECT prog FROM kept_strata_programs ORDER BY written DESC LIMIT ?1)", [PROGRAMS])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_strata_read_back_and_replace_by_relation() {
        let dir = std::env::temp_dir().join(format!("rerail-kept-strata-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let prog = [1u8; 32];
        assert!(store.kept_strata(&prog).unwrap().is_empty());
        let rows = KeptRows { types: vec![0, 1], rows: vec![5, -3, 1 << 40, 7], n: 2 };
        store
            .keep_strata(&prog, &[KeptPut { rel: "r".into(), digest: 9, rows: Some(rows) }, KeptPut { rel: "input".into(), digest: 4, rows: None }], &[])
            .unwrap();
        let kept = store.kept_strata(&prog).unwrap();
        assert_eq!(kept.get("r"), Some(&(9, true)));
        assert_eq!(kept.get("input"), Some(&(4, false)));
        let back = store.kept_strata_rows(&prog, "r").unwrap().unwrap();
        assert_eq!((back.types, back.rows), (vec![0, 1], vec![5, -3, 1 << 40, 7]));
        // A relation replaced, one forgotten.
        store.keep_strata(&prog, &[KeptPut { rel: "r".into(), digest: 10, rows: Some(KeptRows { types: vec![0], rows: vec![1], n: 1 }) }], &["input".into()]).unwrap();
        let kept = store.kept_strata(&prog).unwrap();
        assert_eq!((kept.get("r"), kept.get("input")), (Some(&(10, true)), None));
        // Past the programs kept, the oldest goes.
        for p in 2..=(PROGRAMS as u8 + 1) {
            store.keep_strata(&[p; 32], &[KeptPut { rel: "r".into(), digest: 1, rows: None }], &[]).unwrap();
        }
        assert!(store.kept_strata(&prog).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// What a write of program `prog`'s strata is queued under (`write_back`).
fn kept_key(prog: &[u8; 32]) -> super::write_back::Key {
    super::write_back::Key::of(&[b"kept_strata", prog])
}
