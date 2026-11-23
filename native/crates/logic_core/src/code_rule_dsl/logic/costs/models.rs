//! The learned cost model saved per machine (t33 step 6), a kept fact of the statistics' store: `cost_model`, a
//! relation of its models' space beside the file (`<costs.sqlite>.facts`), one partition a machine. Its one column
//! is words in the order given, as the kept statistics' bytes are (`kept_stats`): when it was trained (an `f64`'s
//! bits), the samples it was trained on, then the machine's name, the comparison it won and the model's bytes, each
//! its length and its bytes eight to a word. It replaces the table `models(machine, at, trained_on, comparison,
//! model)`, which the statistics' file drops when it is opened.

use crate::content_store::kept_facts::{BlockFlat, KeptPut, KeptSpace, KeptWhat, chain_rows, write_space};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use std::path::Path;

const MODEL: &str = "cost_model";

/// The chain of the file's kept facts, as a content store's (`schema.rs`), and the table the model left.
pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS facts_chain(
    space BLOB NOT NULL CHECK(length(space) = 32),
    seq INTEGER NOT NULL CHECK(seq >= 1),
    name TEXT NOT NULL,
    tag INTEGER NOT NULL,
    PRIMARY KEY(space, seq)
) STRICT;
DROP TABLE IF EXISTS models;";

fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.cost-models-space/v1").into()
}

fn ident() -> [u8; 32] {
    Sha256::digest(b"rerail.cost-models/v1\0cost_model").into()
}

/// A machine's partition.
fn part(machine: &str) -> i64 {
    i64::from_le_bytes(Sha256::digest(machine.as_bytes())[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// A machine's saved model.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// When it was trained, in seconds since the epoch.
    pub at: f64,
    /// The samples it was trained on.
    pub trained_on: i64,
    /// How it compared with the rates when it was saved.
    pub comparison: String,
    /// The model (`CostModel::to_bytes`).
    pub model: Vec<u8>,
}

fn push_bytes(out: &mut Vec<i64>, bytes: &[u8]) {
    out.push(bytes.len() as i64);
    out.extend(bytes.chunks(8).map(|c| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        i64::from_le_bytes(w)
    }));
}

fn take_bytes(words: &mut impl Iterator<Item = i64>) -> Option<Vec<u8>> {
    let n = usize::try_from(words.next()?).ok()?;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let w = words.next()?.to_le_bytes();
        let k = (n - out.len()).min(8);
        out.extend_from_slice(&w[..k]);
    }
    Some(out)
}

fn words(machine: &str, saved: &Saved) -> Vec<i64> {
    let mut out = vec![saved.at.to_bits() as i64, saved.trained_on];
    push_bytes(&mut out, machine.as_bytes());
    push_bytes(&mut out, saved.comparison.as_bytes());
    push_bytes(&mut out, &saved.model);
    out
}

fn of_words(machine: &str, words: &[i64]) -> Option<Saved> {
    let mut w = words.iter().copied();
    let at = f64::from_bits(w.next()? as u64);
    let trained_on = w.next()?;
    // Another machine whose name falls in the same partition keeps no model of this one.
    if take_bytes(&mut w)? != machine.as_bytes() {
        return None;
    }
    let comparison = String::from_utf8(take_bytes(&mut w)?).ok()?;
    let model = take_bytes(&mut w)?;
    Some(Saved { at, trained_on, comparison, model })
}

/// `machine`'s saved model in the statistics' store at `path`, read on `db`, its connection.
pub fn read(db: &Connection, path: &Path, machine: &str) -> Option<Saved> {
    let rows = chain_rows(db, &space()).ok()?;
    if rows.is_empty() {
        return None;
    }
    let kept = KeptSpace::open(path, &space(), &rows).ok()?;
    let chain = kept.view();
    let entry = chain.get(&FactKey { rel: MODEL, ident: &ident(), part: part(machine) })?;
    let flat: Vec<i64> = (0..entry.rows.len()).map(|i| entry.rows.get(i, 0)).collect();
    of_words(machine, &flat)
}

/// Saves `machine`'s model in the statistics' store at `path`, on `db`, its connection, in a transaction of its own;
/// `None` takes the saved one out.
pub fn write(db: &mut Connection, path: &Path, machine: &str, saved: Option<&Saved>) -> crate::content_store::StoreResult<()> {
    let what = match saved {
        Some(saved) => KeptWhat::Picked(Box::new(BlockFlat::of_rows(&words(machine, saved), 1, true))),
        None => KeptWhat::Gone,
    };
    let digest = saved.map_or(0, |s| {
        let mut h = Sha256::new();
        for w in words(machine, s) {
            h.update(w.to_le_bytes());
        }
        u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"))
    });
    let put = KeptPut { rel: MODEL.to_owned(), ident: ident(), part: part(machine), digest, reads: Vec::new(), types: vec![b'i'], what };
    let t = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    write_space(path, &t, &space(), vec![put], None)?;
    t.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machines_model_is_a_kept_fact_of_the_statistics_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("costs.sqlite");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        assert!(read(&db, &path, "m1").is_none());
        let a = Saved { at: 1.5e9, trained_on: 1234, comparison: "beats the rates by 12%".into(), model: (0..=250u8).collect() };
        write(&mut db, &path, "m1", Some(&a)).unwrap();
        let b = Saved { at: 2.0e9, trained_on: 7, comparison: String::new(), model: vec![9; 3] };
        write(&mut db, &path, "m2", Some(&b)).unwrap();
        // Another connection (a later process) reads each machine's back.
        let other = Connection::open(&path).unwrap();
        assert_eq!(read(&other, &path, "m1"), Some(a.clone()));
        assert_eq!(read(&other, &path, "m2"), Some(b));
        // Taken out, it is gone; the other machine's stays.
        write(&mut db, &path, "m2", None).unwrap();
        assert_eq!(read(&other, &path, "m2"), None);
        assert_eq!(read(&other, &path, "m1"), Some(a));
        // No table keeps them.
        let tables: i64 = other.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'models'", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 0);
    }
}
