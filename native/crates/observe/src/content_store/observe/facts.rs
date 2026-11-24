//! The observer's answers, canonical forms, settings and run records as kept facts (t33 K2 and step 1, decisions 0104
//! and 0456): relations of one space in the content store whose observer it is, where its own SQLite file kept them
//! as the `observed`, `observer_meta` and `run_record` tables.
//!
//! - `observed(path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listed, listing, recorded_ns)` and
//!   `canonical(path, canonical, recorded_ns)`, one partition per directory: the answers and forms for the paths in
//!   it. A root's part of the tree is read as the partitions at and under it, and the root's own row from its
//!   parent's, as the tables were read by a range of paths.
//! - `observer_meta(key, is_text, number, text)`, one partition: each root's journal cursor, the observer's own
//!   journal's generation and floor, its own output directories.
//! - `run_record(key, list, at, text)` and `run_output(key, at, word)`, one partition per record's key: a record's
//!   lists of texts (each list's `at` -1 row names it, so an empty one is kept too) and its outputs' bytes (`at` -1
//!   their length, then each word, eight bytes, little-endian). The records of what a run read and found: the Git
//!   files' listing, the cone's rounds' cursors, the Git functors' answers, the constants, the whole-run cutoff.
//!
//! A change is applied as one put of the space (`apply`): the partitions it touches, read as they are, edited and
//! written whole, with the settings when they change. So the answers forgotten under a root and the cursor that
//! vouches for the rest are written together, as the table's transaction wrote them. A put waits for this process's
//! earlier puts of the space (`ContentStore::kept_row` settles them first), so each reads the last.
//!
//! The store is above the observer (rerail-store over rerail-observe): the observer reaches it through what it
//! needs of it (`observer_store::base`: `Store`, `Kept`), which the store registers at load (`STORE`). With none
//! registered, nothing is kept.
//!
//! The space is the observer's path's (`<store>.files`, or a test thread's own): two observers of one store keep
//! apart, as their files did.

use super::{Meta, Stat};
use crate::content_store::kept_facts::base::{BlockFlat, KeptPut, KeptWhat};
use crate::content_store::observer_store::base::{Kept, STORE};
use rerail_blocks::kept::{FactKey, KeptRead};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};

const META: &str = "observer_meta";
/// `observer_meta`'s column types: `s` an atom of the store.
const META_TYPES: &[u8] = b"siis";

const RECORD: &str = "run_record";
/// `run_record`'s column types: key, list, at (-1: the list's own row), text.
const RECORD_TYPES: &[u8] = b"siis";
const OUTPUT: &str = "run_output";
/// `run_output`'s column types: key, at (-1: the length), word.
const OUTPUT_TYPES: &[u8] = b"sii";

/// A run's record: its lists of texts, and its outputs' bytes.
pub(super) type Record = (Vec<Vec<String>>, Vec<u8>);

/// An answer as the table held it: path, kind, stat, digest, listing (encoded), when recorded.
pub(super) type Row = (String, i64, Stat, Vec<u8>, Option<Vec<u8>>, i64);
/// A canonical form: the path, its canonical path, when recorded.
pub(super) type Canon = (String, String, i64);

/// A relation of the space partitioned by directory: a row's first column is its path, an atom.
trait ByDir: Sized {
    const REL: &'static str;
    /// Its column types: `s` an atom of the store.
    const TYPES: &'static [u8];
    /// How many of its columns are texts (`texts`).
    const TEXTS: usize;
    fn path(&self) -> &str;
    /// Its texts, interned as atoms for `words`.
    fn texts(&self) -> [String; 3];
    fn words(&self, syms: &[u32]) -> Vec<i64>;
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Self>;
}

impl ByDir for Row {
    const REL: &'static str = "observed";
    const TYPES: &'static [u8] = b"siiiiiisisi";
    const TEXTS: usize = 3;
    fn path(&self) -> &str {
        &self.0
    }
    fn texts(&self) -> [String; 3] {
        [self.0.clone(), hex_of(&self.3), self.4.as_ref().map(|l| String::from_utf8_lossy(l).into_owned()).unwrap_or_default()]
    }
    fn words(&self, s: &[u32]) -> Vec<i64> {
        vec![i64::from(s[0]), self.1, self.2.dev, self.2.ino, self.2.size, self.2.mtime_ns, self.2.ctime_ns, i64::from(s[1]), i64::from(self.4.is_some()), i64::from(s[2]), self.5]
    }
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Row> {
        let path = facts.text(r[0])?;
        let stat = Stat { dev: r[2], ino: r[3], size: r[4], mtime_ns: r[5], ctime_ns: r[6] };
        let digest = super::unhex(&facts.text(r[7]).unwrap_or_default()).map_or_else(Vec::new, |d| d.to_vec());
        let listing = (r[8] == 1).then(|| facts.text(r[9]).unwrap_or_default().into_bytes());
        Some((path, r[1], stat, digest, listing, r[10]))
    }
}

impl ByDir for Canon {
    const REL: &'static str = "canonical";
    const TYPES: &'static [u8] = b"ssi";
    const TEXTS: usize = 2;
    fn path(&self) -> &str {
        &self.0
    }
    fn texts(&self) -> [String; 3] {
        [self.0.clone(), self.1.clone(), String::new()]
    }
    fn words(&self, s: &[u32]) -> Vec<i64> {
        vec![i64::from(s[0]), i64::from(s[1]), self.2]
    }
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Canon> {
        Some((facts.text(r[0])?, facts.text(r[1])?, r[2]))
    }
}

/// One relation's part of a change: rows put, paths whose rows go, paths whose rows go with every row under them.
pub(super) struct Edit<R> {
    pub put: Vec<R>,
    pub delete: Vec<String>,
    pub delete_under: Vec<String>,
}

impl<R> Default for Edit<R> {
    fn default() -> Self {
        Edit { put: Vec::new(), delete: Vec::new(), delete_under: Vec::new() }
    }
}

impl<R> Edit<R> {
    fn is_empty(&self) -> bool {
        self.put.is_empty() && self.delete.is_empty() && self.delete_under.is_empty()
    }
}

/// One change of the observer's kept facts, written as one put.
#[derive(Default)]
pub(super) struct Change {
    pub answers: Edit<Row>,
    pub canon: Edit<Canon>,
    pub meta_put: Vec<(String, rusqlite::types::Value)>,
    pub meta_delete: Vec<String>,
    /// Records put (`Some`) or gone (`None`), by key.
    pub records: Vec<(String, Option<Record>)>,
}

impl Change {
    fn is_empty(&self) -> bool {
        self.answers.is_empty() && self.canon.is_empty() && self.meta_put.is_empty() && self.meta_delete.is_empty() && self.records.is_empty()
    }
}

/// The observer's kept facts in the content store at `path`. The store is opened as it is first asked and closed at
/// the outermost run's end (`close`): a run leaves no handle of its store open.
pub(super) struct Facts {
    path: PathBuf,
    space: [u8; 32],
    store: Mutex<Option<Box<dyn Kept>>>,
}

/// The facts read and written through an open store.
struct Open<'s> {
    store: &'s dyn Kept,
    space: [u8; 32],
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.observer/v1\0{rel}").as_bytes()).into()
}

/// A directory's partition.
fn part(dir: &str) -> i64 {
    let d: [u8; 32] = Sha256::digest(dir.as_bytes()).into();
    i64::from_le_bytes(d[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// A path's separators: `/`, and on Windows `\` too.
fn separator(b: u8) -> bool {
    b == b'/' || (cfg!(windows) && b == b'\\')
}

/// A path's directory, as the partitions are named: its parent (the root itself at the top), or `""` for none.
fn dir_of(path: &str) -> &str {
    match path.bytes().rposition(separator) {
        // At the top of a volume (`/x`, `C:\x`): its root, with the separator.
        Some(i) if !path[..i].bytes().any(separator) => &path[..=i],
        Some(i) => &path[..i],
        None => "",
    }
}

/// Whether `path` is `at` or under it.
fn at_or_under(path: &str, at: &str) -> bool {
    path == at || (path.starts_with(at) && (at.bytes().last().is_some_and(separator) || path.as_bytes().get(at.len()).is_some_and(|&b| separator(b))))
}

impl Facts {
    /// The facts of the observer whose file is `observer` (`<store>.files…`), in the content store beside it; `None`
    /// when the store cannot be opened.
    pub(super) fn open(observer: &Path) -> Option<Facts> {
        let text = observer.to_string_lossy();
        let at = text.rfind(".files")?;
        let path = PathBuf::from(&text[..at]);
        let store = STORE.try_get()?.open(&path)?;
        let space = Sha256::digest(format!("rerail.observer-space/v1\0{text}").as_bytes()).into();
        Some(Facts { path, space, store: Mutex::new(Some(store)) })
    }

    /// `f` of the facts through the store, opened again if closed; `None` when it cannot be.
    fn with<T>(&self, f: impl FnOnce(&Open<'_>) -> T) -> Option<T> {
        let mut held = self.store.lock();
        if held.is_none() {
            *held = Some(STORE.try_get()?.open(&self.path)?);
        }
        Some(f(&Open { store: held.as_deref()?, space: self.space }))
    }

    /// Every setting kept.
    pub(super) fn meta(&self) -> Meta {
        self.with(|o| o.meta()).unwrap_or_default()
    }

    /// The answers and canonical forms kept at and under `unit`.
    pub(super) fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>) {
        self.with(|o| o.under(unit)).unwrap_or_default()
    }

    /// Every answer and canonical form kept (`collect`).
    pub(super) fn all(&self) -> (Vec<Row>, Vec<Canon>) {
        self.with(|o| o.all()).unwrap_or_default()
    }

    /// The record kept under `key`.
    pub(super) fn record(&self, key: &str) -> Option<Record> {
        self.with(|o| o.record(key)).flatten()
    }

    /// Every record whose key starts with `prefix`, by key, with its lists (not its outputs).
    pub(super) fn records(&self, prefix: &str) -> Vec<(String, Vec<Vec<String>>)> {
        self.with(|o| o.records(prefix)).unwrap_or_default()
    }

    /// Writes `change` as one put, a job called `what` (`Open::apply`).
    pub(super) fn apply(&self, change: Change, what: &'static str) {
        self.with(|o| o.apply(change, what));
    }

    /// Waits until every put queued is written, reading nothing (so a thread may ask it as it ends).
    pub(super) fn written(&self) {
        if let Some(store) = self.store.lock().as_ref() {
            store.settle(&self.space);
        }
    }

    /// Waits until every put queued is written, and closes the store (the outermost run's end).
    pub(super) fn close(&self) {
        if let Some(store) = self.store.lock().take() {
            store.settle(&self.space);
        }
    }
}

impl Open<'_> {
    fn text(&self, sym: i64) -> Option<String> {
        self.store.text(u32::try_from(sym).ok()?)
    }

    fn meta(&self) -> Meta {
        let mut meta = Meta::new();
        self.store.read(&self.space, &mut |chain| {
            let Some(kept) = chain.and_then(|c| c.get(&FactKey { rel: META, ident: &ident(META), part: 0 })) else { return };
            for i in 0..kept.rows.len() {
                let row = kept.row(i);
                let Some(key) = self.text(row[0]) else { continue };
                let value = if row[1] == 1 { rusqlite::types::Value::Text(self.text(row[3]).unwrap_or_default()) } else { rusqlite::types::Value::Integer(row[2]) };
                meta.insert(key, value);
            }
        });
        meta
    }

    fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>) {
        let mut out = Default::default();
        self.store.read(&self.space, &mut |chain| {
            if let Some(chain) = chain {
                out = (self.rows_under(chain, unit), self.rows_under(chain, unit));
            }
        });
        out
    }

    fn rows_under<R: ByDir>(&self, chain: &dyn KeptRead, unit: &str) -> Vec<R> {
        let mut out = Vec::new();
        for kept in chain.of(R::REL) {
            if kept.rows.is_empty() {
                continue;
            }
            // A partition's rows share their directory: its first row names it.
            let Some(first) = self.text(kept.row(0)[0]) else { continue };
            let dir = dir_of(&first);
            let whole = at_or_under(dir, unit);
            // The unit's own row is in its parent's partition.
            if !whole && dir != dir_of(unit) {
                continue;
            }
            for i in 0..kept.rows.len() {
                if let Some(row) = R::decode(self, &kept.row(i))
                    && (whole || row.path() == unit)
                {
                    out.push(row);
                }
            }
        }
        out
    }

    fn record(&self, key: &str) -> Option<Record> {
        let mut out = None;
        self.store.read(&self.space, &mut |chain| out = chain.and_then(|chain| self.record_in(chain, key)));
        out
    }

    /// The record of `key` as `chain` (the space's) keeps it.
    fn record_in(&self, chain: &dyn KeptRead, key: &str) -> Option<Record> {
        let at = part(key);
        let output = chain.get(&FactKey { rel: OUTPUT, ident: &ident(OUTPUT), part: at })?;
        // Rows of another key in the same partition (a hash's collision) are not this record's.
        let sym = (0..output.rows.len()).map(|i| output.rows.get(i, 0)).find(|&s| self.text(s).as_deref() == Some(key))?;
        let mut bytes: Option<(usize, Vec<u8>)> = None;
        for i in 0..output.rows.len() {
            if output.rows.get(i, 0) != sym {
                continue;
            }
            let (at, word) = (output.rows.get(i, 1), output.rows.get(i, 2));
            match (&mut bytes, at) {
                (None, -1) => bytes = Some((usize::try_from(word).ok()?, Vec::new())),
                (Some((_, out)), at) if at == (out.len() / 8) as i64 => out.extend_from_slice(&word.to_le_bytes()),
                _ => return None,
            }
        }
        let (n, mut outputs) = bytes?;
        (outputs.len() == n.div_ceil(8) * 8).then_some(())?;
        outputs.truncate(n);
        let lists = chain.get(&FactKey { rel: RECORD, ident: &ident(RECORD), part: at }).map_or(Some(Vec::new()), |kept| self.lists(&kept, sym))?;
        Some((lists, outputs))
    }

    /// The lists `kept` holds of the record whose key is atom `sym`, as `put` gave them: each list's own row, then its
    /// texts in order.
    fn lists(&self, kept: &rerail_blocks::kept::Kept<'_>, sym: i64) -> Option<Vec<Vec<String>>> {
        let mut lists: Vec<Vec<String>> = Vec::new();
        for i in 0..kept.rows.len() {
            if kept.rows.get(i, 0) != sym {
                continue;
            }
            let (list, at) = (kept.rows.get(i, 1), kept.rows.get(i, 2));
            if at == -1 {
                (list == lists.len() as i64).then_some(())?;
                lists.push(Vec::new());
                continue;
            }
            (list == lists.len() as i64 - 1).then_some(())?;
            let last = lists.last_mut()?;
            (at == last.len() as i64).then_some(())?;
            last.push(self.text(kept.rows.get(i, 3))?);
        }
        Some(lists)
    }

    fn records(&self, prefix: &str) -> Vec<(String, Vec<Vec<String>>)> {
        let mut out = Vec::new();
        self.store.read(&self.space, &mut |chain| {
            if let Some(chain) = chain {
                out = self.records_in(chain, prefix);
            }
        });
        out
    }

    /// The records whose keys begin with `prefix`, as `chain` (the space's) keeps them, in key order.
    fn records_in(&self, chain: &dyn KeptRead, prefix: &str) -> Vec<(String, Vec<Vec<String>>)> {
        let mut out = Vec::new();
        // Each record has its outputs' length row: the records are the keys `run_output` holds.
        for output in chain.of(OUTPUT) {
            let mut syms: Vec<i64> = (0..output.rows.len()).filter(|&i| output.rows.get(i, 1) == -1).map(|i| output.rows.get(i, 0)).collect();
            syms.dedup();
            let lists = chain.get(&FactKey { rel: RECORD, ident: &ident(RECORD), part: output.key.part });
            for sym in syms {
                let Some(key) = self.text(sym).filter(|k| k.starts_with(prefix)) else { continue };
                let Some(record) = lists.as_ref().map_or(Some(Vec::new()), |kept| self.lists(kept, sym)) else { continue };
                out.push((key, record));
            }
        }
        out.sort();
        out
    }

    fn all(&self) -> (Vec<Row>, Vec<Canon>) {
        let mut out = Default::default();
        self.store.read(&self.space, &mut |chain| {
            if let Some(chain) = chain {
                out = (self.all_of(chain), self.all_of(chain));
            }
        });
        out
    }

    fn all_of<R: ByDir>(&self, chain: &dyn KeptRead) -> Vec<R> {
        chain.of(R::REL).flat_map(|kept| (0..kept.rows.len()).filter_map(|i| R::decode(self, &kept.row(i))).collect::<Vec<_>>()).collect()
    }

    /// Writes `change`: the partitions it touches, and the settings when they change, as one put, a job called `what`
    /// (`Kept::queue`).
    fn apply(&self, change: Change, what: &'static str) {
        if change.is_empty() {
            return;
        }
        let Change { answers, canon, meta_put, meta_delete, records } = change;
        let (mut answers, mut canon, mut records) = (Some(answers), Some(canon), Some(records));
        let mut puts = Vec::new();
        let mut edited = false;
        self.store.read(&self.space, &mut |chain| {
            let (Some(answers), Some(canon), Some(records)) = (answers.take(), canon.take(), records.take()) else { return };
            edited = self.edit(chain, answers, &mut puts) && self.edit(chain, canon, &mut puts) && self.edit_records(chain, records, &mut puts);
        });
        if !edited {
            return;
        }
        if !meta_put.is_empty() || !meta_delete.is_empty() {
            let mut meta: HashMap<String, rusqlite::types::Value> = self.meta();
            for k in &meta_delete {
                meta.remove(k);
            }
            meta.extend(meta_put);
            let ordered: BTreeMap<String, rusqlite::types::Value> = meta.into_iter().collect();
            let texts: Vec<String> = ordered.iter().flat_map(|(k, v)| [k.clone(), match v { rusqlite::types::Value::Text(t) => t.clone(), _ => String::new() }]).collect();
            let Some(syms) = self.store.intern(&texts.iter().map(String::as_str).collect::<Vec<_>>()) else { return };
            let mut flat = Vec::new();
            for ((_, v), s) in ordered.iter().zip(syms.as_chunks::<2>().0) {
                let (is_text, number) = match v {
                    rusqlite::types::Value::Text(_) => (1, 0),
                    rusqlite::types::Value::Integer(i) => (0, *i),
                    _ => (0, 0),
                };
                flat.extend([i64::from(s[0]), is_text, number, i64::from(s[1])]);
            }
            let n = ordered.len();
            let rows_or_gone = if n == 0 { KeptWhat::Gone } else { KeptWhat::Picked(Box::new(BlockFlat::of_rows(&flat, META_TYPES.len(), true))) };
            puts.push(KeptPut { rel: META.to_owned(), ident: ident(META), part: 0, digest: digest_of(&flat), reads: Vec::new(), types: META_TYPES.to_vec(), what: rows_or_gone });
        }
        self.store.queue(self.space, puts, what);
    }

    /// `edit`'s partitions of `R`, read as kept, edited and pushed to `puts` whole. False if their atoms could not be
    /// interned (nothing is written then).
    fn edit<R: ByDir>(&self, chain: Option<&dyn KeptRead>, edit: Edit<R>, puts: &mut Vec<KeptPut>) -> bool {
        if edit.is_empty() {
            return true;
        }
        let ident = ident(R::REL);
        // The directories touched, each with its rows as kept.
        let mut dirs: BTreeMap<String, BTreeMap<String, R>> = BTreeMap::new();
        let load = |dirs: &mut BTreeMap<String, BTreeMap<String, R>>, dir: &str| {
            if dirs.contains_key(dir) {
                return;
            }
            let mut rows = BTreeMap::new();
            if let Some(kept) = chain.and_then(|c| c.get(&FactKey { rel: R::REL, ident: &ident, part: part(dir) })) {
                for i in 0..kept.rows.len() {
                    if let Some(row) = R::decode(self, &kept.row(i)) {
                        rows.insert(row.path().to_owned(), row);
                    }
                }
            }
            dirs.insert(dir.to_owned(), rows);
        };
        // Every partition at or under a path deleted with what is under it, found by its first row's directory.
        let mut whole_gone: BTreeSet<String> = BTreeSet::new();
        if !edit.delete_under.is_empty()
            && let Some(c) = chain
        {
            for kept in c.of(R::REL) {
                if kept.rows.is_empty() {
                    continue;
                }
                let Some(first) = self.text(kept.row(0)[0]) else { continue };
                let dir = dir_of(&first).to_owned();
                if edit.delete_under.iter().any(|u| at_or_under(&dir, u)) {
                    whole_gone.insert(dir);
                }
            }
        }
        for p in edit.delete.iter().chain(&edit.delete_under) {
            load(&mut dirs, dir_of(p));
            if let Some(rows) = dirs.get_mut(dir_of(p)) {
                rows.remove(p);
            }
        }
        for dir in &whole_gone {
            dirs.insert(dir.clone(), BTreeMap::new());
        }
        for row in edit.put {
            let dir = dir_of(row.path()).to_owned();
            load(&mut dirs, &dir);
            dirs.get_mut(&dir).expect("loaded").insert(row.path().to_owned(), row);
        }
        for (dir, rows) in dirs {
            if rows.is_empty() {
                puts.push(KeptPut { rel: R::REL.to_owned(), ident, part: part(&dir), digest: 0, reads: Vec::new(), types: R::TYPES.to_vec(), what: KeptWhat::Gone });
                continue;
            }
            let texts: Vec<String> = rows.values().flat_map(|r| r.texts().into_iter().take(R::TEXTS)).collect();
            let Some(syms) = self.store.intern(&texts.iter().map(String::as_str).collect::<Vec<_>>()) else { return false };
            let mut flat = Vec::with_capacity(rows.len() * R::TYPES.len());
            for (r, s) in rows.values().zip(syms.chunks_exact(R::TEXTS)) {
                flat.extend(r.words(s));
            }
            puts.push(KeptPut { rel: R::REL.to_owned(), ident, part: part(&dir), digest: digest_of(&flat), reads: Vec::new(), types: R::TYPES.to_vec(), what: KeptWhat::Picked(Box::new(BlockFlat::of_rows(&flat, R::TYPES.len(), true))) });
        }
        true
    }
}

impl Open<'_> {
    /// `records`' partitions, read as kept (the other keys a partition holds kept as they are), edited and pushed to
    /// `puts` whole. False if their atoms could not be interned (nothing is written then).
    fn edit_records(&self, chain: Option<&dyn KeptRead>, records: Vec<(String, Option<Record>)>, puts: &mut Vec<KeptPut>) -> bool {
        if records.is_empty() {
            return true;
        }
        // Each partition touched, with its records by key: those kept, then those this change puts or drops.
        let mut parts: BTreeMap<i64, BTreeMap<String, Record>> = BTreeMap::new();
        for (key, record) in records {
            let at = part(&key);
            let held = parts.entry(at).or_insert_with(|| {
                let mut held = BTreeMap::new();
                if let Some(output) = chain.and_then(|c| c.get(&FactKey { rel: OUTPUT, ident: &ident(OUTPUT), part: at })) {
                    let mut syms: Vec<i64> = (0..output.rows.len()).filter(|&i| output.rows.get(i, 1) == -1).map(|i| output.rows.get(i, 0)).collect();
                    syms.dedup();
                    for sym in syms {
                        if let Some(k) = self.text(sym)
                            && let Some(r) = chain.and_then(|c| self.record_in(c, &k))
                        {
                            held.insert(k, r);
                        }
                    }
                }
                held
            });
            match record {
                Some(r) => held.insert(key, r),
                None => held.remove(&key),
            };
        }
        for (at, held) in parts {
            if held.is_empty() {
                for (rel, types) in [(RECORD, RECORD_TYPES), (OUTPUT, OUTPUT_TYPES)] {
                    puts.push(KeptPut { rel: rel.to_owned(), ident: ident(rel), part: at, digest: 0, reads: Vec::new(), types: types.to_vec(), what: KeptWhat::Gone });
                }
                continue;
            }
            let mut texts: Vec<&str> = Vec::new();
            for (key, (lists, _)) in &held {
                texts.push(key);
                texts.extend(lists.iter().flatten().map(String::as_str));
            }
            texts.push("");
            let Some(syms) = self.store.intern(&texts) else { return false };
            let empty = i64::from(*syms.last().expect("the empty text"));
            let mut syms = syms.iter().map(|&s| i64::from(s));
            let (mut lists_flat, mut output_flat) = (Vec::new(), Vec::new());
            for (lists, outputs) in held.values() {
                let key = syms.next().expect("interned");
                for (l, list) in lists.iter().enumerate() {
                    lists_flat.extend([key, l as i64, -1, empty]);
                    for (i, _) in list.iter().enumerate() {
                        lists_flat.extend([key, l as i64, i as i64, syms.next().expect("interned")]);
                    }
                }
                output_flat.extend([key, -1, outputs.len() as i64]);
                for (i, c) in outputs.chunks(8).enumerate() {
                    let mut w = [0u8; 8];
                    w[..c.len()].copy_from_slice(c);
                    output_flat.extend([key, i as i64, i64::from_le_bytes(w)]);
                }
            }
            for (rel, types, flat) in [(RECORD, RECORD_TYPES, lists_flat), (OUTPUT, OUTPUT_TYPES, output_flat)] {
                let what = if flat.is_empty() { KeptWhat::Gone } else { KeptWhat::Picked(Box::new(BlockFlat::of_rows(&flat, types.len(), true))) };
                puts.push(KeptPut { rel: rel.to_owned(), ident: ident(rel), part: at, digest: digest_of(&flat), reads: Vec::new(), types: types.to_vec(), what });
            }
        }
        true
    }
}

fn digest_of(flat: &[i64]) -> u64 {
    let mut h = Sha256::new();
    for v in flat {
        h.update(v.to_le_bytes());
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"))
}

/// Every answer's path kept, sorted, for a test.
#[cfg(test)]
pub(super) fn all_paths(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().0.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}

/// Every canonical form's path kept, sorted, for a test.
#[cfg(test)]
pub(super) fn all_canonical(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().1.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}

/// A digest as hex (empty for none).
fn hex_of(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
