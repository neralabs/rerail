//! Every cache's one pattern (decision 0102): an entry is facts, and its freshness is a rule the engine runs.
//!
//! An entry records each thing it read with the digest it had then (`cache_read`), and its payload: rows of
//! relations, one stored row per fact (`cache_rows`), or a blob where the payload is not rows. A lookup asks the
//! observer what each of those things is now (`observed`), gives the three relations as facts to `lib/cache.rr`,
//! and the engine derives `cache_fresh`. No cache keys an entry by a hash of its inputs, and none compares
//! digests itself: an input left out of a key cannot give a stale hit, because nothing is left out of the reads.
//!
//! ```text
//! let mut entry = cached::Entry::new("constants", &wanted);
//! entry.read("build", "rerail-engine", env!("RERAIL_ENGINE_BUILD_ID"));
//! entry.reads(consulted);                          // what the run read of the world, misses included
//! entry.keep(&store, &rows)?;
//! // later, another run:
//! let hit = cached::fresh(&store, "constants", &wanted, &mut |kind, name| world.now(kind, name))?;
//! ```
//!
//! The store holds the entries in three tables, a row per fact: `cache_entries`, `cache_reads` and `cache_rows`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::{Error, Result};
use crate::code_rule_dsl::logic::front::Value;

/// The freshness rules (decision 0102 §3).
pub(super) const RULES: &str = include_str!("../../../../../../docs/rule-syntax/lib/cache.rr");

/// A read's digest as a fact: a thing that could not be read (an empty digest) is the symbol `-`, and back.
fn fact(digest: &str) -> String {
    if digest.is_empty() { UNREAD.to_owned() } else { digest.to_owned() }
}
const UNREAD: &str = "-";

/// An entry's payload: each relation's rows, a row's values tab-separated as a run's outputs write them.
pub(super) type Rows = BTreeMap<String, Vec<String>>;

/// An entry being made: its cache, its key, and what it read.
pub(super) struct Entry {
    cache: String,
    key: String,
    reads: BTreeSet<(String, String, String)>,
}

impl Entry {
    pub(super) fn new(cache: &str, key: &str) -> Self {
        Entry { cache: cache.to_owned(), key: key.to_owned(), reads: BTreeSet::new() }
    }

    /// One thing the entry read (`kind`, `name`) and its digest then.
    pub(super) fn read(&mut self, kind: &str, name: &str, digest: &str) {
        self.reads.insert((kind.to_owned(), name.to_owned(), fact(digest)));
    }

    /// Things a run consulted, as `Report::consulted` gives them (what, path, digest).
    pub(super) fn reads<'a>(&mut self, consulted: impl IntoIterator<Item = &'a (String, String, String)>) {
        for (kind, name, digest) in consulted {
            self.read(kind, name, digest);
        }
    }

    /// The entry's id: a digest of what it read, so the same reads are the same entry, and a key keeps an entry
    /// for each state of its inputs (a branch and its revert both hit).
    fn id(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for (k, n, d) in &self.reads {
            for part in [k, n, d] {
                h.update(part.as_bytes());
                h.update([0]);
            }
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Stores the entry, its reads and its rows in one transaction. The store at `path` is made if missing.
    pub(super) fn keep(&self, path: &Path, rows: &Rows) -> Result<()> {
        let mut db = open(path)?;
        let id = self.id();
        let tx = db.transaction()?;
        tx.execute("INSERT OR REPLACE INTO cache_entries VALUES (?1, ?2, ?3)", rusqlite::params![self.cache, self.key, id])?;
        tx.execute("DELETE FROM cache_reads WHERE cache=?1 AND key=?2 AND id=?3", rusqlite::params![self.cache, self.key, id])?;
        tx.execute("DELETE FROM cache_rows WHERE cache=?1 AND key=?2 AND id=?3", rusqlite::params![self.cache, self.key, id])?;
        {
            let mut read = tx.prepare("INSERT INTO cache_reads VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for (kind, name, digest) in &self.reads {
                read.execute(rusqlite::params![self.cache, self.key, id, kind, name, digest])?;
            }
            let mut row = tx.prepare("INSERT OR IGNORE INTO cache_rows VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for (rel, lines) in rows {
                for line in lines {
                    row.execute(rusqlite::params![self.cache, self.key, id, rel, line])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// The pattern's tables (decision 0102 §4), a row per fact.
pub(super) const TABLES: [&str; 3] = ["cache_entries", "cache_reads", "cache_rows"];

fn open(path: &Path) -> Result<rusqlite::Connection> {
    let db = rusqlite::Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(30))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cache_entries(cache TEXT, key TEXT, id TEXT, PRIMARY KEY(cache, key, id)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS cache_reads(cache TEXT, key TEXT, id TEXT, kind TEXT, name TEXT, digest TEXT,
             PRIMARY KEY(cache, key, id, kind, name)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS cache_rows(cache TEXT, key TEXT, id TEXT, rel TEXT, row TEXT,
             PRIMARY KEY(cache, key, id, rel, row)) WITHOUT ROWID;",
    )?;
    Ok(db)
}

/// A fresh entry: its rows, and what it read.
pub(super) struct Hit {
    pub(super) rows: Rows,
    pub(super) reads: Vec<(String, String, String)>,
}

/// The rows of a fresh entry of `cache` under `key`, if one is. `observe(kind, name)` says what a thing an entry
/// read is now (`None`: it cannot be observed, so the entry is stale). Which entry is fresh is `lib/cache.rr`'s,
/// derived by the engine over the entries, their reads and the observations.
pub(super) fn fresh(path: &Path, cache: &str, key: &str, observe: &mut dyn FnMut(&str, &str) -> Option<String>) -> Result<Option<Hit>> {
    if !path.exists() {
        return Ok(None);
    }
    let db = open(path)?;
    let ids: Vec<String> = db.prepare("SELECT id FROM cache_entries WHERE cache=?1 AND key=?2")?.query_map([cache, key], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    if ids.is_empty() {
        return Ok(None);
    }
    let reads: Vec<(String, String, String, String)> = db
        .prepare("SELECT id, kind, name, digest FROM cache_reads WHERE cache=?1 AND key=?2")?
        .query_map([cache, key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let got = evaluate(cache, key, &ids, &reads, observe)?;
    let fresh: BTreeSet<String> = got.outputs.get("cache_fresh").into_iter().flatten().filter_map(|line| line.split('\t').nth(2).map(str::to_owned)).collect();
    let Some(id) = fresh.into_iter().next() else { return Ok(None) };
    let mut rows = Rows::new();
    let mut select = db.prepare("SELECT rel, row FROM cache_rows WHERE cache=?1 AND key=?2 AND id=?3")?;
    for got in select.query_map([cache, key, id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (rel, row) = got?;
        rows.entry(rel).or_default().push(row);
    }
    let reads = reads.into_iter().filter(|(i, ..)| *i == id).map(|(_, k, n, d)| (k, n, if d == UNREAD { String::new() } else { d })).collect();
    Ok(Some(Hit { rows, reads }))
}

/// `lib/cache.rr` over entries `ids` of `cache` under `key`, their `reads` (id, kind, name, digest), and what
/// `observe` says each read thing is now: each asked once, however many entries read it.
fn evaluate(cache: &str, key: &str, ids: &[String], reads: &[(String, String, String, String)], observe: &mut dyn FnMut(&str, &str) -> Option<String>) -> Result<crate::code_rule_dsl::logic::plan::Outcome> {
    let asked: BTreeSet<(&str, &str)> = reads.iter().map(|(_, k, n, _)| (k.as_str(), n.as_str())).collect();
    let observed: Vec<(String, String, String)> = asked.into_iter().filter_map(|(k, n)| observe(k, n).map(|d| (k.to_owned(), n.to_owned(), fact(&d)))).collect();
    let sym = |s: &str| Value::Str(s.to_owned());
    let mut prog = crate::code_rule_dsl::logic::rr::parse(RULES).map_err(|e| Error::Plan(format!("lib/cache.rr: {e}")))?;
    prog.facts.insert("cache_entry".into(), ids.iter().map(|i| vec![sym(cache), sym(key), sym(i)]).collect());
    prog.facts.insert("cache_read".into(), reads.iter().map(|(i, k, n, d)| vec![sym(cache), sym(key), sym(i), sym(k), sym(n), sym(d)]).collect());
    prog.facts.insert("observed".into(), observed.iter().map(|(k, n, d)| vec![sym(k), sym(n), sym(d)]).collect());
    crate::code_rule_dsl::logic::plan::run_facts(&prog)
}

/// Which of `reads` (as `Report::consulted` records them: kind, name, digest) are not as they were, by
/// `lib/cache.rr` (`cache_changed`) over what `observe` says each is now. For a user that brings an entry up to date
/// by what changed (the whole-run cutoff, by the documents whose bytes changed) rather than deriving it again.
pub(super) fn changed(reads: &[(String, String, String)], observe: &mut dyn FnMut(&str, &str) -> Option<String>) -> Result<Vec<(String, String)>> {
    let id = "run".to_owned();
    let reads: Vec<(String, String, String, String)> = reads.iter().map(|(k, n, d)| (id.clone(), k.clone(), n.clone(), fact(d))).collect();
    let got = evaluate("run", "run", std::slice::from_ref(&id), &reads, observe)?;
    Ok(got
        .outputs
        .get("cache_changed")
        .into_iter()
        .flatten()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            Some((cols.get(3)?.to_string(), cols.get(4)?.to_string()))
        })
        .collect())
}

#[cfg(test)]
mod tests;
