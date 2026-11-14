//! Spike (measurement only, `RERAIL_SPIKE_PROJECTION=<db>`): what keeping each document's projected relations on
//! disk would cost, so a later phase or run reads them instead of restoring the tape, rebuilding its facts and
//! projecting it again.
//!
//! After a chunk's projection, every relation the program reads (the batch's rows and its views, not the per-node
//! axes) is split by document, each (document, relation) encoded as columns (zigzag deltas, varints, zstd), written
//! to a SQLite table, read back, decoded and made a relation again. Each step is a span (`spike …`); the bytes by
//! kind go to the profile's decisions. Nothing read back is used: the run goes on with what it built.

use std::collections::HashMap;
use std::sync::Arc;

use crate::code_rule_dsl::facts::vtab::{Relation, Source};

/// Which kind of source a relation is: the batch's own rows, or a view over the documents' facts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Kind {
    Rows,
    View,
}

type Doc = i64;

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(bytes: &[u8], at: &mut usize) -> u64 {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let b = bytes[*at];
        *at += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return v;
        }
        shift += 7;
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// A (document, relation)'s columns: each column's values as deltas from the one before, zigzag varints.
fn encode(cols: &[Vec<i64>]) -> Vec<u8> {
    let mut out = Vec::new();
    for col in cols {
        let mut prev = 0i64;
        for &v in col {
            put_varint(&mut out, zigzag(v.wrapping_sub(prev)));
            prev = v;
        }
    }
    out
}

fn decode(bytes: &[u8], ncols: usize, nrows: usize) -> Vec<Vec<i64>> {
    let mut at = 0;
    (0..ncols)
        .map(|_| {
            let mut prev = 0i64;
            (0..nrows)
                .map(|_| {
                    prev = prev.wrapping_add(unzigzag(get_varint(bytes, &mut at)));
                    prev
                })
                .collect()
        })
        .collect()
}

/// Runs the spike over one chunk's `sources` (those the program reads), its documents' keys `keys`.
pub(super) fn run(db: &std::ffi::OsStr, sources: &[(String, Arc<dyn Source>, Kind)], keys: &[u32]) {
    use crate::code_rule_dsl::logic::profile::span;
    // 1. Split by document: a row's document is its first column's high half, or the column itself when it is `d`.
    let split = span(|| "spike split by document".into());
    let mut parts: Vec<(String, Kind, Vec<String>, HashMap<Doc, Vec<Vec<i64>>>)> = Vec::new();
    for (name, source, kind) in sources {
        let columns = source.columns();
        let by_key = columns.first().is_some_and(|c| c == "d");
        let mut docs: HashMap<Doc, Vec<Vec<i64>>> = HashMap::new();
        for row in 0..source.len() {
            let first = source.value(row, 0);
            let doc = if by_key { first } else { first >> 32 };
            let cols = docs.entry(doc).or_insert_with(|| vec![Vec::new(); columns.len()]);
            for (c, col) in cols.iter_mut().enumerate() {
                col.push(source.value(row, c));
            }
        }
        parts.push((name.clone(), *kind, columns, docs));
    }
    drop(split);
    // 2. Encode and compress.
    let encoding = span(|| "spike encode".into());
    let mut raw: HashMap<Kind, (usize, usize, usize)> = HashMap::new(); // rows, varint bytes, zstd bytes
    let mut blobs: Vec<(Doc, String, usize, usize, Vec<u8>)> = Vec::new();
    for (name, kind, columns, docs) in &parts {
        for (doc, cols) in docs {
            let bytes = encode(cols);
            let packed = zstd::bulk::compress(&bytes, 1).unwrap_or_default();
            let e = raw.entry(*kind).or_default();
            e.0 += cols.first().map_or(0, Vec::len);
            e.1 += bytes.len();
            e.2 += packed.len();
            blobs.push((*doc, name.clone(), columns.len(), cols.first().map_or(0, Vec::len), packed));
        }
    }
    drop(encoding);
    // 3. Write.
    let writing = span(|| "spike write".into());
    let Ok(mut conn) = rusqlite::Connection::open(db) else { return };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(30));
    let _ = conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS proj(doc INTEGER, rel TEXT, cols INTEGER, rows INTEGER, blob BLOB, PRIMARY KEY(doc, rel)) WITHOUT ROWID;",
    );
    if let Ok(tx) = conn.transaction() {
        if let Ok(mut insert) = tx.prepare("INSERT OR REPLACE INTO proj VALUES (?1, ?2, ?3, ?4, ?5)") {
            for (doc, rel, cols, rows, blob) in &blobs {
                let _ = insert.execute(rusqlite::params![doc, rel, *cols as i64, *rows as i64, blob]);
            }
        }
        let _ = tx.commit();
    }
    drop(writing);
    // 4. Read back, by document, each relation the program reads; 5. decode; 6. make the relations again.
    let (mut read_s, mut decode_s, mut build_s) = (0.0, 0.0, 0.0);
    let cpu = || {
        let (user, sys) = crate::code_rule_dsl::prof::thread_usage();
        user + sys
    };
    let reading = span(|| "spike read back".into());
    let mut made: Vec<Relation> = Vec::new();
    if let Ok(mut select) = conn.prepare("SELECT cols, rows, blob FROM proj WHERE doc = ?1 AND rel = ?2") {
        for (name, _, columns, _) in &parts {
            let names: Vec<&str> = columns.iter().map(String::as_str).collect();
            let mut r = Relation::new(&names);
            for &key in keys {
                let t0 = cpu();
                let got: Option<(i64, i64, Vec<u8>)> = select.query_row(rusqlite::params![i64::from(key), name], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).ok();
                let t1 = cpu();
                read_s += t1 - t0;
                let Some((ncols, nrows, blob)) = got else { continue };
                let bytes = zstd::bulk::decompress(&blob, 1 << 30).unwrap_or_default();
                let cols = decode(&bytes, ncols as usize, nrows as usize);
                let t2 = cpu();
                decode_s += t2 - t1;
                let mut row = vec![0i64; cols.len()];
                for i in 0..nrows as usize {
                    for (c, col) in cols.iter().enumerate() {
                        row[c] = col[i];
                    }
                    r.push(&row);
                }
                build_s += cpu() - t2;
            }
            made.push(r);
        }
    }
    drop(reading);
    let rows: usize = made.iter().map(|r| r.len()).sum();
    crate::code_rule_dsl::logic::profile::decision("stored projection spike", || {
        let kind = |k: Kind| raw.get(&k).copied().unwrap_or_default();
        let (r, v) = (kind(Kind::Rows), kind(Kind::View));
        format!(
            "{} documents: rows {} ({} varint, {} zstd bytes), views {} ({} varint, {} zstd bytes); read back {} rows: sqlite {:.3} s, decode {:.3} s, relations {:.3} s",
            keys.len(), r.0, r.1, r.2, v.0, v.1, v.2, rows, read_s, decode_s, build_s
        )
    });
}

#[cfg(test)]
pub mod tests {
    #[test]
    fn columns_round_trip() {
        let cols = vec![vec![5, 3, i64::MAX, -7, 0], vec![1 << 40, (1 << 40) + 3, 2, 2, -1]];
        assert_eq!(super::decode(&super::encode(&cols), 2, 5), cols);
    }
}
