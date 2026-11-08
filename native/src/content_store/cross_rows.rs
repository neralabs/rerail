//! The cross-file memo's rows (decision 0076): each import component's rows of the memoised relations, in
//! `cross_rows`, under a source that holds the component's descriptor (plan, key, documents' keys) and that
//! `cross_components` names by the descriptor's digest. A run finds every component with one query and reads every
//! found component's rows of the relations it needs with one more: SQLite plans each as one statement over the
//! clustered keys, and no per-component loop runs a statement.

use super::{ContentStore, SourceId, StoreResult};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap};

/// Each digest a key probe of `cross_components`.
const LOOKUP_QUERY: &str = "SELECT x.digest, x.source_id FROM json_each(?1) j JOIN cross_components x ON x.digest = j.value";
/// Each (source, relation) pair a range of `cross_rows`' primary key. `CROSS JOIN` fixes the order: left to SQLite,
/// it reads every row of a source and tests each against every name (2.18 M rows against 281 names took 33 s).
const ROWS_QUERY: &str = "SELECT r.rel, r.c0, r.c1, r.c2, r.c3, r.c4, r.c5, r.c6, r.c7, r.source_id \
     FROM json_each(?1) s CROSS JOIN json_each(?2) n CROSS JOIN cross_rows r ON r.source_id = s.value AND r.rel = n.value";

/// The widest relation `cross_rows` holds (columns `c0`..`c7`).
pub const CROSS_WIDTH: usize = 8;

impl ContentStore {
    /// The sources stored under each of `digests`, in one query.
    pub fn cross_lookup(&self, digests: &[String]) -> StoreResult<HashMap<String, SourceId>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let json = serde_json::to_string(digests).expect("strings encode");
        let mut st = connection.prepare_cached(
            LOOKUP_QUERY,
        )?;
        let rows = st.query_map([json], |r| Ok((r.get::<_, String>(0)?, SourceId(r.get(1)?))))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The rows of `rels` (name, width) stored under `sources`, as columns per relation, in one query; with
    /// `with_source`, each relation's last column is the source a row is stored under.
    pub fn cross_rows(&self, sources: &[SourceId], rels: &[(String, usize)], with_source: bool) -> StoreResult<BTreeMap<String, Vec<Vec<i64>>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let ids = serde_json::to_string(&sources.iter().map(|s| s.0).collect::<Vec<_>>()).expect("ids encode");
        let names = serde_json::to_string(&rels.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>()).expect("names encode");
        let mut out: Vec<(String, Vec<Vec<i64>>)> = rels.iter().map(|(r, w)| (r.clone(), vec![Vec::new(); *w + usize::from(with_source)])).collect();
        let index: HashMap<&str, usize> = rels.iter().enumerate().map(|(i, (r, _))| (r.as_str(), i)).collect();
        let mut st = connection.prepare_cached(
            ROWS_QUERY,
        )?;
        let mut q = st.query(params![ids, names])?;
        // Rows come clustered by (source, relation): the relation's index is looked up once per run of rows.
        let (mut last, mut at) = (String::new(), usize::MAX);
        while let Some(r) = q.next()? {
            let rel = r.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
            if at == usize::MAX || rel != last {
                let Some(&i) = index.get(rel) else { continue };
                (last, at) = (rel.to_owned(), i);
            }
            let (width, cols) = (rels[at].1, &mut out[at].1);
            for (i, col) in cols.iter_mut().enumerate() {
                // The source, when asked for, is column 9 (after the eight value columns).
                let at = if i < width { i + 1 } else { 9 };
                col.push(r.get::<_, Option<i64>>(at)?.unwrap_or(0));
            }
        }
        let out: BTreeMap<String, Vec<Vec<i64>>> = out.into_iter().collect();
        Ok(out)
    }

    /// Stores a component's descriptor as a source, names it by `digest`, and replaces its rows with `rels` (name to
    /// width and flat rows), in one transaction.
    pub fn put_cross_rows(&self, digest: &str, descriptor: &[u8], rels: &BTreeMap<String, (usize, Vec<i64>)>) -> StoreResult<SourceId> {
        let source = self.put_source(descriptor, &[])?;
        self.immediate_transaction_once(|t| {
            t.prepare_cached("INSERT INTO cross_components(digest, source_id) VALUES (?1, ?2) ON CONFLICT(digest) DO UPDATE SET source_id = excluded.source_id")?
                .execute(params![digest, source.0])?;
            t.prepare_cached("DELETE FROM cross_rows WHERE source_id = ?1")?.execute([source.0])?;
            let mut ins = t.prepare_cached(
                "INSERT INTO cross_rows(source_id, rel, seq, c0, c1, c2, c3, c4, c5, c6, c7) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for (rel, (width, values)) in rels {
                if *width == 0 || *width > CROSS_WIDTH {
                    return Err(super::StoreError::Invalid(format!("cross rows of {rel}: width {width} outside 1..={CROSS_WIDTH}")));
                }
                for (seq, row) in values.chunks_exact(*width).enumerate() {
                    let c = |i: usize| row.get(i).copied();
                    ins.execute(params![source.0, rel, seq as i64, c(0), c(1), c(2), c(3), c(4), c(5), c(6), c(7)])?;
                }
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
        let rows_plan = plan(ROWS_QUERY);
        assert!(rows_plan.contains("SEARCH r USING PRIMARY KEY (source_id=? AND rel=?)"), "{rows_plan}");
        let lookup_plan = plan(LOOKUP_QUERY);
        assert!(lookup_plan.contains("SEARCH x USING PRIMARY KEY (digest=?)"), "{lookup_plan}");
        drop(connection);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
