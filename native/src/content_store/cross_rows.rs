//! The cross-file memo's rows (decision 0076): each import component's rows of the memoised relations, in
//! `cross_blocks`, under a source that holds the component's descriptor (plan, key, documents' keys) and that
//! `cross_components` names by the descriptor's digest. A run finds every component with one query and reads every
//! found component's rows of the relations it needs with one more: SQLite plans each as one statement over the
//! clustered keys, and no per-component loop runs a statement.
//!
//! A component's rows of one relation are one block: its columns one after another, each value the zigzag varint of
//! its difference from the value above it. A component of 30,000 rows is a few hundred inserts, not 30,000 (the
//! row-per-insert table, `cross_rows`, cost 3.5 µs a row: 13.9 s for calls_cx's 4.0 M rows). The digests
//! `cross_components` holds for blocks carry `BLOCKS`, so a binary that reads `cross_rows` finds none of them (a
//! miss, never rows read as empty).

use super::{ContentStore, SourceId, StoreResult};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap};

/// The prefix of a component's digest whose rows are blocks.
const BLOCKS: &str = "blocks/1:";

/// Each digest a key probe of `cross_components`.
const LOOKUP_QUERY: &str = "SELECT x.digest, x.source_id FROM json_each(?1) j JOIN cross_components x ON x.digest = j.value";
/// Each (source, relation) pair a key of `cross_blocks`. `CROSS JOIN` fixes the order: left to SQLite, it reads
/// every block of a source and tests each against every name.
const BLOCKS_QUERY: &str = "SELECT b.rel, b.width, b.rows, b.data, b.source_id \
     FROM json_each(?1) s CROSS JOIN json_each(?2) n CROSS JOIN cross_blocks b ON b.source_id = s.value AND b.rel = n.value";

/// The widest relation the memo holds.
pub const CROSS_WIDTH: usize = 8;

/// `values` (rows of `width`, flat) as a block: each column in turn, as zigzag varints of the difference from the row
/// above.
fn encode(width: usize, values: &[i64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for c in 0..width {
        let mut last = 0i64;
        for row in values.chunks_exact(width) {
            let d = row[c].wrapping_sub(last);
            last = row[c];
            let mut z = ((d << 1) ^ (d >> 63)) as u64;
            while z >= 0x80 {
                out.push((z as u8) | 0x80);
                z >>= 7;
            }
            out.push(z as u8);
        }
    }
    out
}

/// A block's columns: `rows` values each, `width` of them.
fn decode(width: usize, rows: usize, data: &[u8], cols: &mut [Vec<i64>]) -> StoreResult<()> {
    let bad = || super::StoreError::Invalid("a cross-file memo block ends early".to_owned());
    let mut at = 0;
    for col in cols.iter_mut().take(width) {
        col.reserve(rows);
        let mut last = 0i64;
        for _ in 0..rows {
            let (mut z, mut shift) = (0u64, 0);
            loop {
                let b = *data.get(at).ok_or_else(bad)?;
                at += 1;
                z |= u64::from(b & 0x7f) << shift;
                if b < 0x80 {
                    break;
                }
                shift += 7;
            }
            last = last.wrapping_add(((z >> 1) as i64) ^ -((z & 1) as i64));
            col.push(last);
        }
    }
    Ok(())
}

impl ContentStore {
    /// The sources stored under each of `digests`, in one query.
    pub fn cross_lookup(&self, digests: &[String]) -> StoreResult<HashMap<String, SourceId>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let json = serde_json::to_string(&digests.iter().map(|d| format!("{BLOCKS}{d}")).collect::<Vec<_>>()).expect("strings encode");
        let mut st = connection.prepare_cached(LOOKUP_QUERY)?;
        let rows = st.query_map([json], |r| Ok((r.get::<_, String>(0)?, SourceId(r.get(1)?))))?;
        rows.map(|r| r.map(|(d, s)| (d.strip_prefix(BLOCKS).map_or(d.clone(), str::to_owned), s)).map_err(Into::into)).collect()
    }

    /// The rows of `rels` (name, width) stored under `sources`, as columns per relation, in one query; with
    /// `with_source`, each relation's last column is the source a row is stored under.
    pub fn cross_rows(&self, sources: &[SourceId], rels: &[(String, usize)], with_source: bool) -> StoreResult<BTreeMap<String, Vec<Vec<i64>>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let ids = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("ids encode");
        let names = serde_json::to_string(&rels.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>()).expect("names encode");
        let mut out: Vec<(String, Vec<Vec<i64>>)> = rels.iter().map(|(r, w)| (r.clone(), vec![Vec::new(); *w + usize::from(with_source)])).collect();
        let index: HashMap<&str, usize> = rels.iter().enumerate().map(|(i, (r, _))| (r.as_str(), i)).collect();
        let mut st = connection.prepare_cached(BLOCKS_QUERY)?;
        let mut q = st.query(params![ids, names])?;
        while let Some(r) = q.next()? {
            let rel = r.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
            let Some(&i) = index.get(rel) else { continue };
            let (stored, rows) = (r.get::<_, i64>(1)? as usize, r.get::<_, i64>(2)? as usize);
            let width = rels[i].1;
            if stored != width {
                return Err(super::StoreError::Invalid(format!("cross rows of {rel}: stored {stored} wide, asked {width}")));
            }
            let cols = &mut out[i].1;
            decode(width, rows, r.get_ref(3)?.as_blob().map_err(rusqlite::Error::from)?, cols)?;
            if with_source {
                let source = r.get::<_, i64>(4)?;
                cols[width].extend(std::iter::repeat_n(source, rows));
            }
        }
        Ok(out.into_iter().collect())
    }

    /// Stores a component's descriptor as a source, names it by `digest`, and replaces its rows with `rels` (name to
    /// width and flat rows), in one transaction: a block per relation with rows.
    pub fn put_cross_rows(&self, digest: &str, descriptor: &[u8], rels: &BTreeMap<String, (usize, Vec<i64>)>) -> StoreResult<SourceId> {
        let source = self.put_source(descriptor, &[])?;
        self.immediate_transaction_once(|t| {
            t.prepare_cached("INSERT INTO cross_components(digest, source_id) VALUES (?1, ?2) ON CONFLICT(digest) DO UPDATE SET source_id = excluded.source_id")?
                .execute(params![format!("{BLOCKS}{digest}"), source.0])?;
            t.prepare_cached("DELETE FROM cross_blocks WHERE source_id = ?1")?.execute([source.0])?;
            let mut ins = t.prepare_cached("INSERT INTO cross_blocks(source_id, rel, width, rows, data) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for (rel, (width, values)) in rels {
                if *width == 0 || *width > CROSS_WIDTH {
                    return Err(super::StoreError::Invalid(format!("cross rows of {rel}: width {width} outside 1..={CROSS_WIDTH}")));
                }
                if values.is_empty() {
                    continue;
                }
                let rows = values.len() / width;
                ins.execute(params![source.0, rel, *width as i64, rows as i64, encode(*width, values)])?;
            }
            Ok(source)
        })
    }
}

impl SourceId {
    /// The id as `cross_rows` returns it in a source column (`ContentStore::cross_rows` with `with_source`).
    pub(crate) fn raw(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (std::path::PathBuf, ContentStore) {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("rerail-cross-rows-{}-{nonce}.sqlite", std::process::id()));
        let store = ContentStore::open(&path).unwrap();
        (path, store)
    }

    /// The lookup and the replay are one statement each, and each reads only its keys: a probe per digest, a
    /// primary-key range per (source, relation).
    #[test]
    fn cross_rows_are_read_by_key_ranges_in_one_query() {
        let (path, store) = store();
        let rels = BTreeMap::from([("a".to_owned(), (2, vec![1, 2, 3, 4])), ("b".to_owned(), (1, vec![9]))]);
        let one = store.put_cross_rows("d1", b"one", &rels).unwrap();
        let two = store.put_cross_rows("d2", b"two", &BTreeMap::from([("a".to_owned(), (2, vec![5, 6]))])).unwrap();
        let found = store.cross_lookup(&["d1".to_owned(), "d2".to_owned(), "d3".to_owned()]).unwrap();
        assert_eq!(found, HashMap::from([("d1".to_owned(), one), ("d2".to_owned(), two)]));
        let rows = store.cross_rows(&[one, two], &[("a".to_owned(), 2)], false).unwrap();
        let mut a: Vec<(i64, i64)> = rows["a"][0].iter().copied().zip(rows["a"][1].iter().copied()).collect();
        a.sort_unstable();
        assert_eq!(a, vec![(1, 2), (3, 4), (5, 6)]);
        let connection = store.connection.lock().unwrap();
        let plan = |sql: &str| -> String {
            let mut st = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let empty: Vec<&str> = vec!["[]"; st.parameter_count()];
            let rows = st.query_map(rusqlite::params_from_iter(empty), |r| r.get::<_, String>(3)).unwrap();
            rows.map(Result::unwrap).collect::<Vec<_>>().join("; ")
        };
        let rows_plan = plan(BLOCKS_QUERY);
        assert!(rows_plan.contains("SEARCH b USING PRIMARY KEY (source_id=? AND rel=?)"), "{rows_plan}");
        let lookup_plan = plan(LOOKUP_QUERY);
        assert!(lookup_plan.contains("SEARCH x USING PRIMARY KEY (digest=?)"), "{lookup_plan}");
        drop(connection);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    /// A block gives back its rows, whatever their values (negative, extreme, unsorted), column by column.
    #[test]
    fn a_block_gives_back_its_rows() {
        let values: Vec<i64> = vec![0, i64::MIN, 5 << 32, i64::MAX, -1, 7, 3, 3, (9 << 32) | 4, -12345];
        for width in [1, 2, 5] {
            let values = &values[..values.len() / width * width];
            let rows = values.len() / width;
            let mut cols = vec![Vec::new(); width];
            decode(width, rows, &encode(width, values), &mut cols).unwrap();
            let back: Vec<i64> = (0..rows).flat_map(|r| cols.iter().map(move |c| c[r])).collect();
            assert_eq!(back, values, "width {width}");
        }
        assert!(decode(1, 2, &encode(1, &[1]), &mut [Vec::new()]).is_err());
    }
}
